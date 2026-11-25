import { execFileSync, spawn } from 'node:child_process'
import { appendFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { chromium, type Browser, type Page } from '@playwright/test'
import { CDP_PORT, CLI, CLI_ENV, OUT, WORK, killTree, pidsOf } from './env'

// One Recorder per case: frames, a plain log, the MP4, a contact sheet and a result JSON.
// A case is never passed from an exit code: specs call check() for every observed fact,
// and the coordinator reviews the contact sheet before the result stands.

export interface CliResult { code: number | null; stdout: string; stderr: string; ms: number; peakMb: number }
interface Check { ok: boolean; what: string; detail?: string }

const NOISE = /panicked at|thread '.*' panicked|RUST_BACKTRACE|called `(Option|Result)::unwrap\(\)`|stack backtrace/
const esc = (s: string) => s.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]!)
const pad = (n: number) => String(n).padStart(3, '0')

let term: { browser: Browser; page: Page } | null = null
async function termPage(): Promise<Page> {
  if (!term) {
    const browser = await chromium.launch()
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
    term = { browser, page }
  }
  return term.page
}

/** Peak working set in MB of one PID, sampled now. 0 when the process is gone. */
function memMb(pid: number): number {
  try {
    const out = execFileSync('tasklist', ['/FI', `PID eq ${pid}`, '/FO', 'CSV', '/NH'], { encoding: 'utf8' })
    const m = out.match(/"([\d,.\s]+) K"/)
    return m ? Number(m[1]!.replace(/[^\d]/g, '')) / 1024 : 0
  } catch {
    return 0
  }
}

/** Command line, start time and parent of a PID, for the case log (an orphan is only fixable once we know what it is). */
function describePid(pid: number): string {
  try {
    const ps = `$p=Get-CimInstance Win32_Process -Filter "ProcessId=${pid}"; if(-not $p){'gone'} else {$q=Get-CimInstance Win32_Process -Filter "ProcessId=$($p.ParentProcessId)"; "pid ${pid} started $($p.CreationDate.ToString('s')) cmd [$($p.CommandLine)] parent $($p.ParentProcessId) " + $(if($q){"alive ($($q.Name) $($q.CommandLine))"}else{'DEAD'})}`
    return execFileSync('powershell', ['-NoProfile', '-Command', ps], { encoding: 'utf8', windowsHide: true }).trim().replace(/\s+/g, ' ').slice(0, 700)
  } catch (e) {
    return `pid ${pid} (could not inspect: ${String(e).slice(0, 80)})`
  }
}

export class Recorder {
  readonly dir: string
  readonly log: string
  private n = 0
  private checks: Check[] = []
  private notes: string[] = []
  private skipReason: string | null = null
  private pageErrors: string[] = []
  private cliNoise: string[] = []
  private repos = new Set<string>()
  private t0 = Date.now()
  private mgBefore: number[]
  page: Page | null = null
  private appBrowser: Browser | null = null

  private constructor(readonly id: string, readonly slug: string) {
    this.dir = join(OUT, 'frames', id)
    rmSync(this.dir, { recursive: true, force: true })
    mkdirSync(this.dir, { recursive: true })
    this.log = join(OUT, `${id}-${slug}.log`)
    writeFileSync(this.log, `# ${id} ${slug}\n# started ${new Date().toISOString()}\n# cli ${CLI}\n\n`)
    this.mgBefore = pidsOf('mediagit.exe')
  }

  static async start(id: string, slug: string, opts: { gui?: boolean } = {}): Promise<Recorder> {
    const r = new Recorder(id, slug)
    if (opts.gui) await r.attach()
    return r
  }

  /** Attach (or re-attach after an app restart) to the app's WebView2 over CDP. */
  async attach(): Promise<Page> {
    await this.appBrowser?.close().catch(() => {})
    this.appBrowser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`)
    const ctx = this.appBrowser.contexts()[0]
    if (!ctx) throw new Error('WebView2 exposed no browser context over CDP')
    this.page = ctx.pages()[0] ?? (await ctx.waitForEvent('page'))
    await this.page.waitForLoadState('domcontentloaded')
    // No fixture, so no config timeouts: without this a locator read on a missing element
    // waits until the 30-minute test timeout (it hung S-08 once).
    this.page.setDefaultTimeout(30_000)
    this.page.on('response', (r) => { if (r.status() === 404) this.line(`NOTE 404 ${r.url()}`) })
    this.page.on('pageerror', (e) => this.pageErrors.push(`pageerror: ${e}`))
    this.page.on('console', (m) => { if (m.type() === 'error') this.pageErrors.push(`console.error: ${m.text()} @ ${m.location().url}`) })
    return this.page
  }

  line(s: string) { appendFileSync(this.log, `${s}\n`) }
  note(s: string) { this.notes.push(s); this.line(`NOTE ${s}`) }
  /** Mark a repo this case touched: it gets `fsck --quick` in the ambient watch. */
  touch(repo: string) { this.repos.add(repo) }
  /** Drop a repo from the ambient fsck: the case corrupted it on purpose. */
  untouch(repo: string) { this.repos.delete(repo) }

  /** Record one observed fact. Never throws: a case collects every failure it sees. */
  check(ok: boolean, what: string, detail?: string): boolean {
    this.checks.push({ ok, what, detail })
    this.line(`${ok ? 'CHECK ok  ' : 'CHECK FAIL'} ${what}${detail ? ` :: ${detail}` : ''}`)
    return ok
  }

  private frame() { return join(this.dir, `step-${pad(this.n++)}.png`) }

  /** Screenshot the app as it is now. */
  async shot(label: string) {
    if (!this.page) return
    const f = this.frame()
    await this.page.screenshot({ path: f }).catch((e) => this.line(`SHOT FAILED ${label}: ${e}`))
    this.line(`SHOT ${f.split('\\').pop()} ${label}`)
  }

  /** One GUI step: frame before, the action, frame after. A thrown error is a failed check. */
  async step<T>(name: string, fn: () => Promise<T>): Promise<T | undefined> {
    const t = Date.now()
    this.line(`\nSTEP ${name}`)
    await this.shot(`before: ${name}`)
    try {
      const v = await fn()
      await this.shot(`after: ${name}`)
      this.line(`STEP ok ${Date.now() - t} ms`)
      return v
    } catch (e) {
      await this.shot(`FAILED: ${name}`)
      if (this.page) await this.page.screenshot({ path: join(OUT, 'fail', `${this.id}-${pad(this.n)}.png`) }).catch(() => {})
      this.check(false, `step "${name}"`, String(e).slice(0, 600))
      return undefined
    }
  }

  /** Run the CLI, log command, exit code, output, time and peak memory, and add a terminal frame. */
  async cli(cwd: string, args: string[], opts: { input?: string; env?: Record<string, string>; bin?: string; quietFrame?: boolean; timeoutMs?: number } = {}): Promise<CliResult> {
    const exe = opts.bin ?? CLI
    const t = Date.now()
    const res = await new Promise<CliResult>((done) => {
      const p = spawn(exe, args, { cwd, env: { ...process.env, ...CLI_ENV, ...opts.env }, windowsHide: true })
      let stdout = ''
      let stderr = ''
      let peakMb = 0
      const poll = setInterval(() => { if (p.pid) peakMb = Math.max(peakMb, memMb(p.pid)) }, 500)
      const killer = opts.timeoutMs && p.pid ? setTimeout(() => { stderr += `
killed by the harness after ${opts.timeoutMs} ms`; killTree(p.pid!) }, opts.timeoutMs) : null
      p.stdout.on('data', (d) => { stdout += d })
      p.stderr.on('data', (d) => { stderr += d })
      if (opts.input !== undefined) p.stdin.end(opts.input)
      else p.stdin.end()
      p.on('error', (e) => { clearInterval(poll); done({ code: -1, stdout, stderr: `${stderr}\nspawn error: ${e}`, ms: Date.now() - t, peakMb }) })
      p.on('close', (code) => { clearInterval(poll); if (killer) clearTimeout(killer); done({ code, stdout, stderr, ms: Date.now() - t, peakMb }) })
    })
    const cmd = `${exe.split('\\').pop()!.replace('.exe', '')} ${args.map((a) => (/\s/.test(a) ? `"${a}"` : a)).join(' ')}`
    this.line(`\n$ ${cmd}\n  cwd: ${cwd}\n  exit: ${res.code}  time: ${res.ms} ms  peak: ${res.peakMb.toFixed(0)} MB`)
    if (res.stdout) this.line(`--- stdout\n${res.stdout.trimEnd()}`)
    if (res.stderr) this.line(`--- stderr\n${res.stderr.trimEnd()}`)
    if (NOISE.test(res.stdout + res.stderr)) this.cliNoise.push(`${cmd}: ${(res.stdout + res.stderr).match(NOISE)![0]}`)
    if (!opts.quietFrame) await this.termFrame(cmd, cwd, res)
    return res
  }

  private async termFrame(cmd: string, cwd: string, r: CliResult) {
    const body = (r.stdout.slice(0, 20_000) + (r.stderr ? `\n${r.stderr.slice(0, 5_000)}` : '')).trimEnd().split('\n')
    const shown = body.length > 34 ? [...body.slice(0, 16), `   ... ${body.length - 32} more lines (see the .log) ...`, ...body.slice(-16)] : body
    const colour = r.code === 0 ? '#6DD196' : '#FF8A80'
    const html = `<body style="margin:0;background:#1F1F1F;color:#EDEDED;font:15px/1.45 Consolas,monospace;padding:18px 22px">
<div style="color:#A8A8A8;font-size:13px">${esc(this.id)} ${esc(this.slug)} &nbsp;|&nbsp; ${esc(cwd)}</div>
<div style="margin:10px 0 8px;color:#FFD24D;white-space:pre-wrap;word-break:break-all">$ ${esc(cmd)}</div>
<pre style="margin:0;white-space:pre-wrap;word-break:break-all;font:inherit">${esc(shown.map((l) => l.slice(0, 300)).join('\n'))}</pre>
<div style="position:fixed;bottom:12px;left:22px;color:${colour}">exit ${r.code} &nbsp; ${r.ms} ms &nbsp; peak ${r.peakMb.toFixed(0)} MB</div></body>`
    // A decoration frame must not fail a case: on a dead terminal browser, relaunch it once.
    for (let i = 0; i < 2; i++) {
      try {
        const p = await termPage()
        await p.setContent(html)
        await p.screenshot({ path: this.frame() })
        return
      } catch {
        await term?.browser.close().catch(() => {})
        term = null
      }
    }
  }

  /** Record the real app window while fn runs (motion cases). */
  // CDP screenshots, not gdigrab: a GDI grab of the WebView2 window returns a frozen first
  // frame (the content is GPU-composited). Each frame is held until the next one, so the
  // MP4 keeps real timing.
  async liveCapture<T>(fn: () => Promise<T>, _title = 'MediaGit'): Promise<T> {
    if (!this.page) return fn()
    const out = join(OUT, `${this.id}-${this.slug}-live.mp4`)
    const dir = join(this.dir, 'live')
    rmSync(out, { force: true })
    rmSync(dir, { recursive: true, force: true })
    mkdirSync(dir, { recursive: true })
    const got: { f: string; t: number }[] = []
    const page = this.page
    let on = true
    // Poll screenshots at about 4 fps. (CDP screencast sends nothing while the page is still.)
    const loop = (async () => {
      while (on) {
        const f = join(dir, `f${String(got.length).padStart(5, '0')}.jpg`)
        const t = Date.now()
        const ok = await page.screenshot({ path: f, type: 'jpeg', quality: 70, scale: 'css' }).then(() => true, () => false)
        if (ok) got.push({ f, t })
        await new Promise((r) => setTimeout(r, Math.max(0, 250 - (Date.now() - t))))
      }
    })()
    try {
      return await fn()
    } finally {
      on = false
      await loop
      const end = Date.now()
      if (got.length) {
        const list = got.map((g, i) => `file '${g.f.replace(/\\/g, '/')}'\nduration ${(((got[i + 1]?.t ?? end) - g.t) / 1000).toFixed(3)}`).join('\n')
        writeFileSync(join(dir, 'list.txt'), `${list}\nfile '${got.at(-1)!.f.replace(/\\/g, '/')}'\n`)
        try {
          execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', join(dir, 'list.txt'), '-vf', 'scale=1280:-2,fps=10', '-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-crf', '24', out], { stdio: ['ignore', 'pipe', 'pipe'] })
        } catch (e) {
          this.line(`FFMPEG FAILED (live) ${String((e as { stderr?: Buffer }).stderr ?? e).slice(0, 400)}`)
        }
      }
      rmSync(dir, { recursive: true, force: true })
      this.line(`LIVE ${out} ${got.length} frames, ${existsSync(out) ? `${(statSync(out).size / 1e6).toFixed(1)} MB` : 'MISSING'}`)
    }
  }

  private ambient(): Check[] {
    const a: Check[] = []
    a.push({ ok: this.pageErrors.length === 0, what: 'ambient: no page errors', detail: this.pageErrors.slice(0, 8).join(' | ') })
    a.push({ ok: this.cliNoise.length === 0, what: 'ambient: no panic or backtrace in CLI output', detail: this.cliNoise.join(' | ') })
    const stray = () => pidsOf('mediagit.exe').filter((p) => !this.mgBefore.includes(p))
    // Describe each stray while it is still alive: it can exit between the last poll and the report.
    const seen = new Map<number, string>()
    const note = () => { for (const p of stray()) if (!seen.has(p)) seen.set(p, describePid(p)) }
    note()
    // children of a just-closed app/server need a moment to exit
    for (let i = 0; i < 20 && stray().length; i++) { Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 500); note() }
    for (const [p, d] of seen) this.line(`STRAY ${p} ${d}`)
    const orphans = stray()
    a.push({ ok: orphans.length === 0, what: 'ambient: no orphan mediagit.exe', detail: orphans.join(',') })
    // Say what each orphan is: command line, start time, and whether its parent is alive (and what it is).
    for (const pid of orphans) this.line(`ORPHAN ${describePid(pid)}`)
    for (const repo of this.repos) {
      if (!existsSync(join(repo, '.mediagit'))) continue
      let out = ''
      let code = 0
      try {
        out = execFileSync(CLI, ['fsck', '--quick'], { cwd: repo, env: { ...process.env, ...CLI_ENV }, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })
      } catch (e) {
        const x = e as { status?: number; stdout?: string; stderr?: string }
        code = x.status ?? -1
        out = `${x.stdout ?? ''}${x.stderr ?? ''}`
      }
      a.push({ ok: code === 0, what: `ambient: fsck --quick ${repo}`, detail: code === 0 ? undefined : out.slice(-400) })
      const junk = readdirSync(join(repo, '.mediagit')).filter((f) => /\.(tmp|partial)$|\.lock$/.test(f))
      a.push({ ok: junk.length === 0, what: `ambient: no leftover temp or lock files in ${repo}\\.mediagit`, detail: junk.join(',') })
    }
    for (const c of a) this.line(`${c.ok ? 'AMBIENT ok  ' : 'AMBIENT FAIL'} ${c.what}${c.detail ? ` :: ${c.detail}` : ''}`)
    return a
  }

  /** Record a SKIP verdict (a missing prerequisite, not a failure). A failed check still wins. */
  skip(reason: string) {
    this.skipReason = reason
    this.line(`SKIP ${reason}`)
  }

  /** Build the MP4 and contact sheet, run the ambient watch, write the result JSON. */
  async finish(): Promise<{ failed: number }> {
    const ambient = this.ambient()
    const frames = readdirSync(this.dir).filter((f) => f.endsWith('.png')).length
    const mp4 = join(OUT, `${this.id}-${this.slug}.mp4`)
    const sheet = join(OUT, `${this.id}-${this.slug}.contact.png`)
    const fit = 'scale=1280:800:force_original_aspect_ratio=decrease:flags=lanczos,pad=1280:800:(ow-iw)/2:(oh-ih)/2:color=0x1F1F1F'
    if (frames > 0) {
      const ff = (args: string[]) => {
        try {
          execFileSync('ffmpeg', ['-y', '-loglevel', 'error', ...args], { stdio: ['ignore', 'pipe', 'pipe'] })
        } catch (e) {
          this.line(`FFMPEG FAILED ${String((e as { stderr?: Buffer }).stderr ?? e).slice(0, 400)}`)
        }
      }
      ff(['-framerate', '2', '-i', join(this.dir, 'step-%03d.png'), '-vf', fit, '-r', '10', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '20', mp4])
      const cols = Math.min(6, frames)
      // From the MP4, not the PNGs: frames differ in size, and a size change resets `tile`.
      ff(['-i', mp4, '-vf', `fps=2,scale=480:300,tile=${cols}x${Math.ceil(frames / cols)}`, '-frames:v', '1', sheet])
    }
    const all = [...this.checks, ...ambient]
    const failed = all.filter((c) => !c.ok)
    const result = {
      id: this.id, slug: this.slug, frames, ms: Date.now() - this.t0,
      result: failed.length > 0 ? 'FAIL' : this.skipReason ? 'SKIP' : 'PASS-PENDING-REVIEW',
      skipReason: this.skipReason,
      checks: this.checks, ambient, notes: this.notes,
      mp4: existsSync(mp4) ? mp4 : null, contact: existsSync(sheet) ? sheet : null, log: this.log,
      cli: CLI, cliBuilt: statSync(CLI).mtime.toISOString(),
    }
    writeFileSync(join(OUT, `${this.id}-${this.slug}.json`), JSON.stringify(result, null, 2))
    this.line(`\n# finished ${new Date().toISOString()} checks ${all.length} failed ${failed.length}`)
    appendFileSync(join(OUT, '..', 'STATUS.md'), `- ${this.id} ${this.slug}: ${result.result} (${failed.length} failed of ${all.length} checks, ${frames} frames, ${Math.round(result.ms / 1000)} s)\n`)
    await this.appBrowser?.close().catch(() => {}) // detaches only
    return { failed: failed.length }
  }
}

export async function closeTerm() {
  await term?.browser.close().catch(() => {})
  term = null
}

export const workDir = (name: string) => {
  const d = join(WORK, name)
  mkdirSync(d, { recursive: true })
  return d
}
