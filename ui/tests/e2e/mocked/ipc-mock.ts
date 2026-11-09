import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import type { Page } from '@playwright/test'

/** One scripted step of a streaming command (clone). */
export interface Step {
  /** Wait this many ms first. */
  delay?: number
  /** Send a progress message on the command's channel. */
  progress?: Record<string, unknown>
  /** Pause until the test calls `release()` (or the op is cancelled). */
  hold?: boolean
  resolve?: unknown
  reject?: { code: string; message: string }
}

export interface Scenario {
  /** command -> result. A `{__error:{code,message}}` value rejects. */
  commands?: Record<string, unknown>
  /** command -> scripted steps; the command's channel arg is `onProgress`. */
  scripts?: Record<string, Step[]>
  /** command -> source of `(args, calls) => result`, for results that depend on the arguments. */
  handlers?: Record<string, string>
  /** What the folder picker returns. */
  dialog?: string | null
  /** localStorage seeded before the page loads (only if the key is absent). */
  storage?: Record<string, string>
}

export function fixture<T = unknown>(name: string): T {
  return JSON.parse(readFileSync(fileURLToPath(new URL(`../../fixtures/${name}.json`, import.meta.url)), 'utf8')) as T
}

export const RECENT = JSON.stringify([
  { path: '/repos/hero-film', name: 'hero-film', openedAt: Date.now() - 2 * 3600_000, branch: 'main' },
  { path: '/repos/trailer-cut', name: 'trailer-cut', openedAt: Date.now() - 26 * 3600_000, branch: 'main' },
])

/** Install a fake Tauri IPC layer before the app boots. */
export async function mockIpc(page: Page, scenario: Scenario): Promise<void> {
  await page.addInitScript((sc: Scenario) => {
    type Call = { cmd: string; args: unknown }
    const calls: Call[] = []
    const callbacks = new Map<number, (m: unknown) => void>()
    const listeners = new Map<string, number[]>()
    let nextCb = 1
    let release: (() => void)[] = []
    let cancel: (() => void)[] = []

    for (const [k, v] of Object.entries(sc.storage ?? {})) {
      if (localStorage.getItem(k) === null) localStorage.setItem(k, v)
    }

    const w = window as unknown as Record<string, unknown>
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }

    const reply = (v: unknown) => {
      const err = (v as { __error?: unknown } | null)?.__error
      return err ? Promise.reject(err) : Promise.resolve(v ?? null)
    }

    async function runScript(steps: NonNullable<Scenario['scripts']>[string], channelId: number | undefined) {
      let index = 0
      const cancelled = new Promise<'cancelled'>((r) => cancel.push(() => r('cancelled')))
      for (const s of steps) {
        if (s.delay) await new Promise((r) => setTimeout(r, s.delay))
        if (s.progress && channelId !== undefined) callbacks.get(channelId)?.({ index: index++, message: s.progress })
        if (s.hold) {
          const r = await Promise.race([new Promise<'go'>((go) => release.push(() => go('go'))), cancelled])
          if (r === 'cancelled') throw { code: 'cancelled', message: 'Cancelled' }
        }
        if (s.reject) throw s.reject
        if (s.resolve !== undefined) return s.resolve
      }
      return null
    }

    w.__ipc = {
      calls,
      emit(event: string, payload: unknown = null) {
        for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload })
      },
      release() {
        const r = release
        release = []
        r.forEach((f) => f())
      },
    }

    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
      transformCallback(cb: (m: unknown) => void) {
        const id = nextCb++
        callbacks.set(id, cb)
        return id
      },
      unregisterCallback(id: number) {
        callbacks.delete(id)
      },
      // Real asset URLs do not exist under Vite: images and audio resolve to tiny inline data.
      convertFileSrc: (p: string) =>
        /\.(png|jpe?g|webp|gif)$/i.test(p)
          ? 'data:image/svg+xml,' +
            encodeURIComponent(
              '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#5b7c99"/><stop offset="1" stop-color="#d9a066"/></linearGradient></defs><rect width="320" height="180" fill="url(#g)"/><circle cx="220" cy="70" r="28" fill="#f4e4c1" opacity="0.85"/><path d="M0 180 L90 100 L150 150 L220 90 L320 180 Z" fill="#2f3b47" opacity="0.8"/></svg>',
            )
          : /\.wav$/i.test(p)
            ? 'data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAIA+AAACABAAZGF0YQAAAAA='
            : p,
      async invoke(cmd: string, args: Record<string, unknown> = {}) {
        calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) })
        if (cmd === 'plugin:event|listen') {
          const list = listeners.get(args.event as string) ?? []
          list.push(args.handler as number)
          listeners.set(args.event as string, list)
          return list.length
        }
        if (cmd === 'plugin:event|unlisten') return null
        if (cmd === 'plugin:dialog|open') return sc.dialog ?? null
        if (cmd === 'cancel_op') {
          const c = cancel
          cancel = []
          c.forEach((f) => f())
          return true
        }
        const handler = sc.handlers?.[cmd]
        if (handler) return reply(new Function(`return (${handler})`)()(args, calls))
        const script = sc.scripts?.[cmd]
        if (script) {
          const channel = args.onProgress as { id?: number } | undefined
          return runScript(script, channel?.id)
        }
        return reply(sc.commands?.[cmd])
      },
    }
  }, scenario)
}

export interface Call {
  cmd: string
  args: Record<string, unknown>
}

export const ipcCalls = (page: Page, cmd?: string): Promise<Call[]> =>
  page.evaluate((c) => {
    const all = (window as unknown as { __ipc: { calls: Call[] } }).__ipc.calls
    return c ? all.filter((x) => x.cmd === c) : all
  }, cmd)

export const ipcRelease = (page: Page) => page.evaluate(() => (window as unknown as { __ipc: { release(): void } }).__ipc.release())

export const ipcEmit = (page: Page, event: string) =>
  page.evaluate((e) => (window as unknown as { __ipc: { emit(e: string): void } }).__ipc.emit(e), event)

/** Standard commands for a populated, healthy app. */
export function healthy(extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    engine_info: fixture('engine_ok'),
    repo_status: fixture('status_populated'),
    // Quiet defaults for the data the inspector asks for on any selection.
    lock_list: { locks: [] },
    repo_log: { commits: [], refs: [] },
    media_info: {},
    config_schema: { keys: [] },
    ...extra,
  }
}

/** `repo_show` handler: the tree fixture for `--tree`, the plain commit otherwise. */
export const showHandler = () =>
  `(a) => a.tree ? ${JSON.stringify(fixture('show_tree'))} : ${JSON.stringify(fixture('show_commit'))}`

/** `repo_log` handler: a linear history of `total` commits, served page by page. */
export const pagedLogHandler = (total: number) => `(a) => {
  const id = (i) => String(i).padStart(4, '0').repeat(10)
  const commits = []
  for (let i = a.skip; i < Math.min(${total}, a.skip + a.max); i++) {
    commits.push({ oid: id(i), parents: i + 1 < ${total} ? [id(i + 1)] : [], author: { name: 'Mira', email: 'm@x.example' }, time: 1793000000 - i * 60, summary: 'commit ' + i, message: 'commit ' + i })
  }
  return { commits, refs: [{ name: 'HEAD', oid: id(0), kind: 'head' }, { name: 'main', oid: id(0), kind: 'branch' }] }
}`
