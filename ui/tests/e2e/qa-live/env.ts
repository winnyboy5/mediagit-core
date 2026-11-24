import { execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

// Shared paths, binaries and app lifecycle for the live QA campaign.

export const repoRoot = resolve(import.meta.dirname, '..', '..', '..', '..')
// Results root (outside the repo). Override with MG_QA_ROOT.
export const QA = process.env.MG_QA_ROOT ?? resolve(repoRoot, '..', 'qa-live')
// Run id. Playwright runs globalSetup and workers in separate processes, so the runner
// (dev-tests/qa-suite/scripts/run_qalive.ps1) sets MG_QA_DAY explicitly. The timestamp default is for ad-hoc
// runs only: set MG_QA_DAY yourself if you run globalSetup and the specs in different invocations.
const stamp = () => new Date().toISOString().replace(/\..*$/, '').replace(/[-:]/g, '').replace('T', '-')
export const DAY = process.env.MG_QA_DAY ?? (process.env.MG_QA_DAY = `run-${stamp()}`)
export const OUT = join(QA, DAY)
export const WORK = join(QA, 'work')
// Same variable GA's dev-tests/qa-suite/config.ps1 uses.
export const CORPUS = process.env.MG_QA_TESTFILES ?? join(repoRoot, 'test-files')
const profile = process.env.MG_QA_PROFILE ?? 'release'
const bin = (n: string) => join(repoRoot, 'target', profile, n)
export const CLI = process.env.MG_QA_CLI ?? bin('mediagit.exe')
export const SERVER = process.env.MG_QA_SERVER ?? bin('mediagit-server.exe')
export const DESKTOP = process.env.MG_QA_DESKTOP ?? bin('mediagit-desktop.exe')
// Silo/MinIO client used by S-13 only. Unset or missing makes S-13 record SKIP.
export const MCLI = process.env.MG_QA_MCLI
export const CDP_PORT = Number(process.env.MG_QA_CDP_PORT ?? 9333)
export const CLI_ENV: Record<string, string> = {
  MEDIAGIT_AUTHOR_NAME: 'QA Live',
  MEDIAGIT_AUTHOR_EMAIL: 'qa-live@example.test',
  MEDIAGIT_CDC_SEED: '20260716',
}
const STATE = join(WORK, 'state.json')

// procs: what this run started, with the spawn time (ms), so the runner's cleanup can prove a PID is still the same process
// (Windows reuses PIDs) before it kills anything.
export interface QaProc { pid: number; started: number; name: string }
export interface QaState { appPid: number | null; pids: number[]; procs?: QaProc[] }
export const readState = (): QaState => (existsSync(STATE) ? (JSON.parse(readFileSync(STATE, 'utf8')) as QaState) : { appPid: null, pids: [], procs: [] })
const saveState = (s: QaState) => writeFileSync(STATE, JSON.stringify(s, null, 2))

export function killTree(pid: number) {
  try {
    execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' })
  } catch {
    // already gone
  }
}

/** Start time (ms epoch) and image name of a live PID, or null. */
function procInfo(pid: number): { start: number; name: string } | null {
  try {
    const out = execFileSync('powershell', ['-NoProfile', '-Command', `$p=Get-Process -Id ${pid} -ErrorAction Stop; [string][DateTimeOffset]::new($p.StartTime).ToUnixTimeMilliseconds()+' '+$p.ProcessName`], { encoding: 'utf8', windowsHide: true }).trim().split(' ')
    return { start: Number(out[0]), name: out[1] ?? '' }
  } catch {
    return null
  }
}

/** True only if `e` still names the process this run spawned (Windows reuses PIDs): same PID, started 0-15 s before it was recorded, plausible image. Same rule as run_qalive.ps1. */
export function isRecorded(e: QaProc): boolean {
  const i = procInfo(e.pid)
  if (!i) return false
  const delta = (e.started - i.start) / 1000
  return /^(mediagit|node|cmd|pnpm)/i.test(i.name) && delta >= -2 && delta <= 15
}

/** Stop a process this run recorded, but only if the PID still belongs to it; always forget the entry. A PID that was never recorded is never touched. */
export function stopTracked(pid: number) {
  const s = readState()
  const e = (s.procs ?? []).find((x) => x.pid === pid)
  if (e && isRecorded(e)) killTree(pid)
  s.procs = (s.procs ?? []).filter((x) => x.pid !== pid)
  s.pids = s.pids.filter((x) => x !== pid)
  if (s.appPid === pid) s.appPid = null
  saveState(s)
}

/** A fresh state file for this run: entries from an earlier (ad-hoc or -SkipCleanup) run name PIDs that may be reused by now. */
export function resetState() {
  saveState({ appPid: null, pids: [], procs: [] })
}

export async function waitFor(what: string, ms: number, probe: () => Promise<boolean> | boolean) {
  const end = Date.now() + ms
  while (Date.now() < end) {
    try {
      if (await probe()) return
    } catch {
      // keep polling
    }
    await new Promise((r) => setTimeout(r, 300))
  }
  throw new Error(`timed out after ${ms / 1000}s waiting for ${what}`)
}

export const cdpUp = async () => (await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`)).ok

/** Start the desktop app with CDP on its WebView2. Returns its PID. */
export async function launchApp(extraEnv: Record<string, string> = {}, unset: string[] = []): Promise<number> {
  for (const d of ['logs', 'webview2', 'app-server']) mkdirSync(join(WORK, d), { recursive: true })
  const out = openSync(join(WORK, 'logs', 'desktop.log'), 'a')
  const childEnv: Record<string, string | undefined> = {
      ...process.env,
      ...CLI_ENV,
      MEDIAGIT_DESKTOP_CLI: CLI,
      MEDIAGIT_DESKTOP_SERVER: SERVER,
      MEDIAGIT_DESKTOP_SERVER_DIR: join(WORK, 'app-server'),
      MEDIAGIT_DESKTOP_STATE_DIR: join(WORK, 'app-state'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: join(WORK, 'webview2'),
      ...extraEnv,
  }
  for (const k of unset) delete childEnv[k]
  const p = spawn(DESKTOP, [], {
    env: childEnv,
    stdio: ['ignore', out, out],
    detached: true,
    windowsHide: false,
  })
  if (!p.pid) throw new Error('could not start the desktop app')
  p.unref()
  const s = readState()
  s.appPid = p.pid
  s.pids.push(p.pid)
  ;(s.procs ??= []).push({ pid: p.pid, started: Date.now(), name: 'mediagit-desktop' })
  saveState(s)
  await waitFor(`CDP on :${CDP_PORT}`, 60_000, cdpUp)
  return p.pid
}

export function killApp() {
  const s = readState()
  if (s.appPid) stopTracked(s.appPid)
}

/** Close the app the way a user does (WM_CLOSE), so WebView2 flushes its storage. Falls back to a kill. */
export async function quitApp() {
  const s = readState()
  const e = (s.procs ?? []).find((x) => x.pid === s.appPid)
  if (!s.appPid || !e) return
  const pid = s.appPid
  if (!isRecorded(e)) { stopTracked(pid); return } // already gone, or the PID was reused: touch nothing
  const alive = () => { try { process.kill(pid, 0); return true } catch { return false } }
  try { execFileSync('taskkill', ['/PID', String(pid)], { stdio: 'ignore' }) } catch { /* fall through to the kill */ }
  for (let i = 0; i < 40 && alive(); i++) await new Promise((r) => setTimeout(r, 250))
  stopTracked(pid)
}

export function trackPid(pid: number, name = 'node') {
  const s = readState()
  s.pids.push(pid)
  ;(s.procs ??= []).push({ pid, started: Date.now(), name })
  saveState(s)
}

/** PIDs of every running process with this image name. */
export function pidsOf(image: string): number[] {
  const out = execFileSync('tasklist', ['/FI', `IMAGENAME eq ${image}`, '/FO', 'CSV', '/NH'], { encoding: 'utf8' })
  return [...out.matchAll(/^"[^"]+","(\d+)"/gm)].map((m) => Number(m[1]))
}

export function freeGb(): number {
  const out = execFileSync('powershell', ['-NoProfile', '-Command', '(Get-PSDrive D).Free'], { encoding: 'utf8' })
  return Math.round(Number(out.trim()) / 1e9)
}
