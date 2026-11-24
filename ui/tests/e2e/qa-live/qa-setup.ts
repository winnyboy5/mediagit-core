import { execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdirSync, openSync, readFileSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { CLI, DESKTOP, OUT, SERVER, WORK, repoRoot, cdpUp, launchApp, readState, resetState, stopTracked, trackPid, waitFor } from './env'

const uiDir = resolve(import.meta.dirname, '..', '..', '..')

// Step 0 (PLAN.md decision 2): the binaries and the built UI must be newer than the last commit that touched
// product code. Product code is frozen since 9506546; later commits are docs and tests, so the path filter
// (not HEAD) is what counts. MG_QA_ALLOW_STALE=1 skips the gate and says so in the log.
function staleBinaryCheck() {
  const git = (...a: string[]) => execFileSync('git', ['-C', repoRoot, ...a], { encoding: 'utf8' }).trim()
  const head = git('rev-parse', '--short', 'HEAD')
  const [ct, sha, ...subj] = git('log', '-1', '--format=%ct %h %s', '--', 'crates', 'ui/src', 'ui/index.html', 'Cargo.toml', 'Cargo.lock').split(' ')
  const since = Number(ct) * 1000
  const stale: string[] = []
  for (const [k, p] of [['mediagit.exe', CLI], ['mediagit-server.exe', SERVER], ['mediagit-desktop.exe', DESKTOP], ['ui/dist/index.html', join(uiDir, 'dist', 'index.html')]] as const) {
    if (!existsSync(p)) { stale.push(`${k} missing`); continue }
    if (statSync(p).mtimeMs < since) stale.push(`${k} built ${new Date(statSync(p).mtimeMs).toISOString()}`)
  }
  const line = `step 0: HEAD ${head}; last product commit ${sha} ${subj.join(' ')} (${new Date(since).toISOString()}); ${stale.length ? 'STALE: ' + stale.join(', ') : 'binaries and UI are newer'}`
  console.log(line)
  if (stale.length && process.env.MG_QA_ALLOW_STALE !== '1') throw new Error(`stale build (older than the last product commit ${sha}): ${stale.join(', ')}. Rebuild (user builds release; pnpm --dir ui build for ui/dist) or set MG_QA_ALLOW_STALE=1 to run anyway.`)
}

// Global setup: make the folders and, unless MG_QA_NO_APP=1 (CLI-only cases), start the
// desktop app once. Teardown stops only the PIDs this campaign started.
export default async function globalSetup() {
  for (const [k, p] of [['cli', CLI], ['server', SERVER], ['desktop', DESKTOP]] as const) {
    if (!existsSync(p)) throw new Error(`${k} not found: ${p}`)
  }
  staleBinaryCheck()
  for (const d of [OUT, join(OUT, 'frames'), join(OUT, 'fail'), WORK]) mkdirSync(d, { recursive: true })
  resetState()
  const wantApp = process.env.MG_QA_NO_APP !== '1'
  if (wantApp && !(await cdpUp().catch(() => false))) {
    // A plain `cargo build --release` has no `custom-protocol` feature, so the exe loads
    // devUrl (localhost:5173) instead of the embedded UI. Serve the built ui/dist there.
    const ui = 'http://localhost:5173'
    const up = async () => (await fetch(ui)).ok
    // Whatever already answers on :5173 must be the built ui/dist, not the user's own dev server.
    if (process.env.MG_QA_UI !== 'dev' && (await up().catch(() => false))) {
      const served = (await (await fetch(ui)).text()).trim()
      const built = readFileSync(join(uiDir, 'dist', 'index.html'), 'utf8').trim()
      if (served !== built) throw new Error('something else is serving :5173 (not ui/dist, maybe a vite dev server). Stop it, or set MG_QA_UI=dev to test it on purpose.')
    }
    if (!(await up().catch(() => false))) {
      const out = openSync(join(WORK, 'ui-server.log'), 'a')
      const mode = process.env.MG_QA_UI === 'dev' ? ['dev'] : ['exec', 'vite', 'preview']
      const p = spawn('pnpm', [...mode, '--port', '5173', '--strictPort'], { cwd: uiDir, stdio: ['ignore', out, out], shell: true, windowsHide: true })
      if (!p.pid) throw new Error('could not start the UI server')
      trackPid(p.pid, 'pnpm')
      await waitFor('UI on :5173', 90_000, up)
    }
    await launchApp()
  }
  return async () => {
    if (process.env.MG_QA_KEEP_APP === '1') return
    for (const e of [...(readState().procs ?? [])].reverse()) stopTracked(e.pid)
  }
}
