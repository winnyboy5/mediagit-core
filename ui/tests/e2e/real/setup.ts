import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

// Global setup for the real-app suite. Returns the teardown (Playwright runs a function
// returned from globalSetup after the run). Specs read the JSON state file named by
// MG_E2E_STATE; teardown stops exactly the PIDs this file spawned, as process trees.

export interface E2EState {
  root: string
  repo: string // the fixture repo the app opens
  clone2: string // path for the second clone (created by the spec)
  remoteUrl: string
  cli: string
  cdpPort: number
  serverPort: number
  pids: number[]
  tempDirs: string[]
  env: Record<string, string>
}

const uiDir = resolve(import.meta.dirname, '..', '..', '..')
const repoRoot = resolve(uiDir, '..')
const AUTHOR = { MEDIAGIT_AUTHOR_NAME: 'E2E Tester', MEDIAGIT_AUTHOR_EMAIL: 'e2e@example.test' }

export function readState(): E2EState {
  const p = process.env.MG_E2E_STATE
  if (!p) throw new Error('MG_E2E_STATE not set: run through playwright.real.config.ts')
  return JSON.parse(readFileSync(p, 'utf8')) as E2EState
}

function freePort(): Promise<number> {
  return new Promise((ok, fail) => {
    const s = createServer()
    s.once('error', fail)
    s.listen(0, '127.0.0.1', () => {
      const port = (s.address() as { port: number }).port
      s.close(() => ok(port))
    })
  })
}

async function waitFor(what: string, ms: number, probe: () => Promise<boolean>) {
  const end = Date.now() + ms
  let last = ''
  while (Date.now() < end) {
    try {
      if (await probe()) return
    } catch (e) {
      last = String(e)
    }
    await new Promise((r) => setTimeout(r, 500))
  }
  throw new Error(`timed out after ${ms / 1000}s waiting for ${what}${last ? ` (${last})` : ''}`)
}

const up = async (url: string) => (await fetch(url)).ok

function killTree(pid: number) {
  try {
    execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' })
  } catch {
    // already gone
  }
}

export default async function globalSetup() {
  const exe = process.env.MG_E2E_DESKTOP_EXE ?? join(repoRoot, 'target', 'debug', 'mediagit-desktop.exe')
  const cli = process.env.MG_E2E_CLI ?? join(repoRoot, 'target', 'debug', 'mediagit.exe')
  const server = process.env.MG_E2E_SERVER ?? join(repoRoot, 'target', 'debug', 'mediagit-server.exe')
  const cdpPort = Number(process.env.MG_E2E_CDP_PORT ?? 9333)
  for (const [k, p] of [['desktop exe', exe], ['cli', cli], ['server', server]] as const) {
    if (!existsSync(p)) throw new Error(`${k} not found: ${p} (build the debug binaries or set MG_E2E_*)`)
  }

  const root = mkdtempSync(join(tmpdir(), 'mg-e2e-'))
  const logs = join(root, 'logs')
  mkdirSync(logs)
  const state: E2EState = {
    root, repo: join(root, 'hero-film'), clone2: join(root, 'second-clone'), remoteUrl: '', cli, cdpPort,
    serverPort: 0, pids: [], tempDirs: [root], env: { ...AUTHOR },
  }
  const statePath = join(root, 'state.json')
  process.env.MG_E2E_STATE = statePath
  const save = () => writeFileSync(statePath, JSON.stringify(state, null, 2))
  const env = { ...process.env, ...AUTHOR, MEDIAGIT_CDC_SEED: '20260716' }
  const mg = (cwd: string, ...args: string[]) => execFileSync(cli, args, { cwd, env, encoding: 'utf8' })

  const teardown = async () => {
    for (const pid of [...state.pids].reverse()) killTree(pid)
    await new Promise((r) => setTimeout(r, 1000))
    if (process.env.MG_E2E_KEEP === '1') return
    for (const d of state.tempDirs) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 })
  }
  const start = (cmd: string, args: string[], name: string, extra: Record<string, string> = {}, shell = false): ChildProcess => {
    const out = openSync(join(logs, `${name}.log`), 'a')
    const p = spawn(cmd, args, { env: { ...env, ...extra }, stdio: ['ignore', out, out], shell, windowsHide: true })
    if (!p.pid) throw new Error(`could not start ${name}`)
    state.pids.push(p.pid)
    save()
    return p
  }

  try {
    // --- local server: a bare repo dir with a filesystem backend, as the QA suite does ---
    const srvDir = join(root, 'server')
    const repoName = 'hero-film'
    const srvRepo = join(srvDir, 'repos', repoName)
    mkdirSync(srvRepo, { recursive: true })
    mg(root, 'init', '--bare', srvRepo)
    const fwd = (p: string) => p.replace(/\\/g, '/')
    writeFileSync(join(srvRepo, '.mediagit', 'config.toml'), `[storage]\nbackend = "filesystem"\nbase_path = "${fwd(join(srvDir, 'storage'))}"\n`)
    state.serverPort = await freePort()
    writeFileSync(join(srvDir, 'server.toml'), `port = ${state.serverPort}\nhost = "127.0.0.1"\nrepos_dir = "${fwd(join(srvDir, 'repos'))}"\nenable_rate_limiting = true\n`)
    start(server, ['--config', join(srvDir, 'server.toml')], 'server')
    const base = `http://127.0.0.1:${state.serverPort}`
    await waitFor('mediagit-server health', 60_000, () => up(`${base}/health`))
    state.remoteUrl = `${base}/${repoName}`

    // --- fixture repo via the real CLI ---
    mkdirSync(state.repo)
    mg(state.repo, 'init')
    writeFileSync(join(state.repo, 'notes.txt'), 'first draft\n')
    mkdirSync(join(state.repo, 'art'))
    // ~6 MB deterministic pseudo-random bytes (a binary-ish asset worth locking)
    const bin = Buffer.alloc(6 * 1024 * 1024)
    let x = 0x9e3779b9
    for (let i = 0; i < bin.length; i++) {
      x = (Math.imul(x, 1664525) + 1013904223) >>> 0
      bin[i] = x >>> 24
    }
    writeFileSync(join(state.repo, 'art', 'hero.bin'), bin)
    writeFileSync(join(state.repo, 'README.md'), '# hero-film\n')
    mg(state.repo, 'add', '.')
    mg(state.repo, 'commit', '-m', 'Initial fixture')
    mg(state.repo, 'remote', 'add', 'origin', state.remoteUrl)
    mg(state.repo, 'push', '-u', 'origin', 'main')

    // --- Vite dev server (the debug desktop build loads http://localhost:5173) ---
    const devUp = await up('http://localhost:5173').catch(() => false)
    if (!devUp) {
      start('pnpm', ['dev', '--port', '5173', '--strictPort'], 'vite', {}, true)
      await waitFor('vite on :5173', 90_000, () => up('http://localhost:5173'))
    }

    // --- desktop app with a CDP port on its WebView2 ---
    const wvData = join(root, 'webview2')
    start(exe, [], 'desktop', {
      MEDIAGIT_DESKTOP_CLI: cli,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}`,
      WEBVIEW2_USER_DATA_FOLDER: wvData,
    })
    await waitFor(`CDP on :${cdpPort} (is another app already using it?)`, 60_000, () => up(`http://127.0.0.1:${cdpPort}/json/version`))
    save()
  } catch (e) {
    await teardown()
    throw e
  }
  return teardown
}
