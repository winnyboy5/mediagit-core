import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, openSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { CLI, DESKTOP, SERVER, WORK, quitApp, readState, resetState, stopTracked, trackPid, waitFor } from './demo-env'

const uiDir = resolve(import.meta.dirname, '..', '..', '..')

// A plain release build loads the UI from localhost:5173, so serve the built ui/dist there
// (same as qa-setup). The app itself is launched per chapter by startChapter.
export default async function globalSetup() {
  for (const [k, p] of [['cli', CLI], ['server', SERVER], ['desktop', DESKTOP]] as const) {
    if (!existsSync(p)) throw new Error(`${k} not found: ${p}`)
  }
  mkdirSync(WORK, { recursive: true })
  resetState()
  const ui = 'http://localhost:5173'
  const up = async () => (await fetch(ui)).ok
  if (await up().catch(() => false)) {
    const served = (await (await fetch(ui)).text()).trim()
    const built = readFileSync(join(uiDir, 'dist', 'index.html'), 'utf8').trim()
    if (served !== built) throw new Error('something else is serving :5173 (not ui/dist). Stop it first.')
  } else {
    const out = openSync(join(WORK, 'ui-server.log'), 'a')
    const p = spawn('pnpm', ['exec', 'vite', 'preview', '--port', '5173', '--strictPort'], { cwd: uiDir, stdio: ['ignore', out, out], shell: true, windowsHide: true })
    if (!p.pid) throw new Error('could not start the UI server')
    trackPid(p.pid, 'pnpm')
    await waitFor('UI on :5173', 90_000, up)
  }
  // Quits any app this run launched (a test that threw before ch.stop), then stops what it recorded.
  return async () => { await quitApp().catch(() => {}); for (const e of [...(readState().procs ?? [])].reverse()) stopTracked(e.pid) }
}
