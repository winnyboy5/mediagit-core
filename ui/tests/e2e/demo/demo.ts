import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { chromium, type Browser, type Locator, type Page } from '@playwright/test'
import { CDP_PORT, DEMO_ROOT, OUT, WORK, cdpUp, launchApp, quitApp } from './demo-env'

// Demo recorder: attaches to the real app over CDP, records a 1920x1080 clip from
// Page.startScreencast frames (30 fps CFR mp4), and draws plain-English captions.

const W = 1280
const H = 720
const SCALE = 1.5

export interface Chapter {
  page: Page
  /** Show a caption for `ms`, then hide it. */
  caption(text: string, ms: number): Promise<void>
  pace(ms: number): Promise<void>
  /** Click, then type slowly like a person. */
  type(loc: Locator, text: string): Promise<void>
  /** Blur a field (access key IDs, account names) before it is typed into. */
  blur(loc: Locator): Promise<void>
  /** Run fn (a real wait, such as a transfer) and cut the time it took out of the video; later captions and audio shift to match. */
  skip<T>(fn: () => Promise<T>): Promise<T>
  stop(): Promise<string>
}

const CAP_JS = (text: string | null) => {
  let el = document.getElementById('__demo_caption')
  if (!text) { el?.remove(); return }
  if (!el) {
    el = document.createElement('div')
    el.id = '__demo_caption'
    el.setAttribute('style', 'position:fixed;left:50%;bottom:28px;transform:translateX(-50%);max-width:80%;z-index:2147483647;' +
      'background:rgba(15,15,20,.88);color:#fff;font:600 22px/1.35 "Segoe UI",system-ui,sans-serif;padding:12px 26px;' +
      'border-radius:12px;text-align:center;pointer-events:none;box-shadow:0 6px 24px rgba(0,0,0,.35)')
    document.body.appendChild(el)
  }
  el.textContent = text
}

const slugOf = (t: string) => t.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')

const LOCK = join(OUT, '.reclock')
const alive = (pid: number) => { try { process.kill(pid, 0); return true } catch { return false } }

/** One recording at a time (one screen, one CDP port): mkdir is the atomic lock; a dead owner's lock is taken over. */
async function acquireLock() {
  mkdirSync(OUT, { recursive: true })
  for (let waited = 0; ; waited += 5) {
    try {
      mkdirSync(LOCK)
      writeFileSync(join(LOCK, 'pid'), String(process.pid))
      return
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== 'EEXIST') throw e
      const owner = Number(readFileSync(join(LOCK, 'pid'), 'utf8').trim() || 0)
      if (owner && !alive(owner)) { rmSync(LOCK, { recursive: true, force: true }); continue }
      if (waited >= 30 * 60) throw new Error(`recording lock held by pid ${owner} for 30 min`)
      await new Promise((r) => setTimeout(r, 5000))
    }
  }
}
/** Poll once a second until nothing answers CDP; false if still up after `ms`. */
async function waitCdpDown(ms: number): Promise<boolean> {
  for (let t = 0; ; t += 1000) {
    if (!(await cdpUp().catch(() => false))) return true
    if (t >= ms) return false
    await new Promise((r) => setTimeout(r, 1000))
  }
}

/** PID listening on the CDP port, for the error message. */
function portPid(): string {
  try {
    const out = execFileSync('netstat', ['-ano', '-p', 'TCP'], { encoding: 'utf8' })
    return out.split('\n').find((l) => l.includes(`:${CDP_PORT} `) && /LISTENING/.test(l))?.trim().split(/\s+/).pop() ?? 'unknown'
  } catch { return 'unknown' }
}

const releaseLock =() => rmSync(LOCK, { recursive: true, force: true })

export async function startChapter(id: string, title: string): Promise<Chapter> {
  await acquireLock()
  try {
    const ch = await launchChapter(id, title)
    const stop = ch.stop
    // Idempotent; the app is always quit and the lock always released, even if encoding failed.
    let done: Promise<string> | null = null
    ch.stop = () => (done ??= (async () => {
      try { return await stop() } finally { await quitApp().catch(() => {}); await waitCdpDown(20_000); releaseLock() }
    })())
    return ch
  } catch (e) {
    await quitApp().catch(() => {}) // launchApp may have started the app before the failure
    releaseLock()
    throw e
  }
}

async function launchChapter(id: string, title: string): Promise<Chapter> {
  if (WORK.toLowerCase() !== join(DEMO_ROOT, 'work').toLowerCase()) throw new Error(`unsafe demo work path: ${WORK}`)
  // Lock is already ours. A previous holder's WebView2 may still be exiting: wait up to 45 s for the port to free.
  if (!(await waitCdpDown(45_000))) throw new Error(`something still answers CDP on :${CDP_PORT} (held by pid ${portPid()}); refusing to record against an app this run did not start`)
  // Fresh app state per chapter; state.json and ui-server.log (what setup started) stay.
  for (const d of ['app-state', 'webview2', 'app-server', 'demo']) rmSync(join(WORK, d), { recursive: true, force: true })
  mkdirSync(WORK, { recursive: true })
  mkdirSync(OUT, { recursive: true })
  await launchApp()

  const browser: Browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`)
  const ctx = browser.contexts()[0]
  if (!ctx) throw new Error('WebView2 exposed no browser context over CDP')
  const page = ctx.pages()[0] ?? (await ctx.waitForEvent('page'))
  await page.waitForLoadState('domcontentloaded')
  page.setDefaultTimeout(30_000)

  const cdp = await ctx.newCDPSession(page)
  // Window first (frames take the window's aspect), then 1280x720 @1.5 => 1920x1080 frames.
  const win = (await cdp.send('Browser.getWindowForTarget')) as { windowId: number }
  await cdp.send('Browser.setWindowBounds', { windowId: win.windowId, bounds: { windowState: 'normal', width: W, height: H } })
  await page.waitForTimeout(500)
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: SCALE, mobile: false })

  const slug = `${id}-${slugOf(title)}`
  const dir = join(OUT, 'frames', slug)
  rmSync(dir, { recursive: true, force: true })
  mkdirSync(dir, { recursive: true })
  const frames: { f: string; t: number }[] = []
  // Serialise writes so frame order is arrival order.
  let chain: Promise<unknown> = Promise.resolve()
  cdp.on('Page.screencastFrame', (e: { data: string; sessionId: number }) => {
    const t = Date.now()
    const f = join(dir, `f${String(frames.length).padStart(6, '0')}.jpg`)
    frames.push({ f, t })
    chain = chain.then(() => import('node:fs/promises').then((fs) => fs.writeFile(f, Buffer.from(e.data, 'base64'))))
    cdp.send('Page.screencastFrameAck', { sessionId: e.sessionId }).catch(() => {})
  })
  await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 90, maxWidth: W * SCALE, maxHeight: H * SCALE, everyNthFrame: 1 })

  // Narration: text -> wav/duration from narrate.mjs (never synthesized here). Times are Date.now() ms.
  let manifest: Record<string, { wav: string; durationMs: number }> = {}
  try { manifest = JSON.parse(readFileSync(join(OUT, 'tts-cache', 'manifest.json'), 'utf8')) } catch { /* no narration */ }
  const narration: { text: string; t: number; wav: string }[] = []
  const caps: { text: string; t: number; ms: number }[] = []
  let firstCap = 0
  const skips: { a: number; b: number }[] = []

  // Caption survives navigation and reload: re-inject whatever is current.
  let current: string | null = null
  const paint = () => page.evaluate(CAP_JS, current).catch(() => {})
  page.on('load', () => { void paint() })
  page.on('framenavigated', () => { void paint() })

  return {
    page,
    async caption(text, ms) {
      const nar = manifest[text]
      if (nar) ms = Math.max(ms, nar.durationMs + 600)
      current = text
      await paint()
      const t = Date.now()
      firstCap ||= t
      caps.push({ text, t, ms })
      if (nar) narration.push({ text, t, wav: nar.wav })
      await page.waitForTimeout(ms)
      current = null
      await paint()
    },
    pace: (ms) => page.waitForTimeout(ms),
    async skip(fn) {
      const a = Date.now()
      try { return await fn() } finally { skips.push({ a, b: Date.now() }) }
    },
    async type(loc, text) {
      await loc.click()
      await loc.pressSequentially(text, { delay: 80 })
    },
    blur: (loc) => loc.evaluate((el) => { (el as HTMLElement).style.filter = 'blur(7px)' }),
    async stop() {
      let end = Date.now()
      await cdp.send('Page.stopScreencast').catch(() => {})
      await chain
      const out = join(OUT, `${slug}.mp4`)
      if (!frames.length) throw new Error('screencast produced no frames')
      // Cut skipped spans: drop their frames, pull every later time earlier by the skipped time.
      const adj = (t: number) => t - skips.reduce((n, s) => n + Math.min(Math.max(t - s.a, 0), s.b - s.a), 0)
      const live = frames.filter((g) => !skips.some((s) => g.t >= s.a && g.t < s.b)).map((g) => ({ ...g, t: adj(g.t) }))
      frames.splice(0, frames.length, ...live)
      end = adj(end)
      firstCap = firstCap && adj(firstCap)
      for (const c of caps) c.t = adj(c.t)
      for (const n of narration) n.t = adj(n.t)
      // Drop the empty opening: keep frames from 0.3 s before the first caption (all, if there is none).
      const cut = firstCap ? firstCap - 300 : frames[0]!.t
      let k = frames.findIndex((g) => g.t > cut)
      k = k < 0 ? frames.length - 1 : Math.max(0, k - 1)
      const kept = frames.slice(k).map((g, i) => (i === 0 ? { ...g, t: Math.max(g.t, cut) } : g))
      const t0 = kept[0]!.t
      const list = kept.map((g, i) => `file '${g.f.replace(/\\/g, '/')}'\nduration ${(((kept[i + 1]?.t ?? end) - g.t) / 1000).toFixed(3)}`).join('\n')
      writeFileSync(join(dir, 'list.txt'), `${list}\nfile '${kept.at(-1)!.f.replace(/\\/g, '/')}'\n`)
      // Audio: each narration wav delayed to its caption; no narration = a silent track. Always 48 kHz stereo AAC.
      const aIn = narration.length ? narration.flatMap((n) => ['-i', n.wav]) : ['-f', 'lavfi', '-i', 'anullsrc=r=48000:cl=stereo']
      const aFilter = narration.length
        ? [...narration.map((n, i) => `[${i + 1}:a]aresample=48000,aformat=channel_layouts=stereo,adelay=${Math.max(0, n.t - t0)}:all=1[a${i}]`),
          `${narration.map((_, i) => `[a${i}]`).join('')}amix=inputs=${narration.length}:normalize=0,loudnorm=I=-16[a]`].join(';')
        : '[1:a]acopy[a]'
      execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', join(dir, 'list.txt'), ...aIn,
        '-filter_complex', aFilter, '-map', '0:v', '-map', '[a]', '-vf', 'fps=30,format=yuv420p', '-r', '30',
        '-c:v', 'libx264', '-preset', 'medium', '-crf', '18', '-c:a', 'aac', '-b:a', '192k', '-ar', '48000', '-ac', '2', '-shortest', out],
      { stdio: ['ignore', 'pipe', 'pipe'] })
      writeFileSync(join(OUT, `${slug}.json`), JSON.stringify({ id, title, clip: `${slug}.mp4`, frames: kept.length,
        captions: caps.map((c) => ({ text: c.text, startMs: c.t - t0, endMs: c.t - t0 + c.ms })) }, null, 2))
      await browser.close().catch(() => {}) // detaches only
      await quitApp()
      rmSync(dir, { recursive: true, force: true })
      return out
    },
  }
}
