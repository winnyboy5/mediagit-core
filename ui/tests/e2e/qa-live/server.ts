import { execFileSync, spawn } from 'node:child_process'
import { mkdirSync, openSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { join } from 'node:path'
import { CLI, CLI_ENV, SERVER, stopTracked, trackPid, waitFor } from './env'

// A local mediagit-server started by the harness (not by the app), no auth, filesystem
// storage. Same layout tests/e2e/real/setup.ts uses. For transfer, perf and scale cases.

export interface LocalServer { dir: string; port: number; base: string; pid: number; log: string; url: (repo: string) => string; addRepo: (name: string) => string; stop: () => void }

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

const fwd = (p: string) => p.replace(/\\/g, '/')

export async function startServer(dir: string, repos: string[] = []): Promise<LocalServer> {
  mkdirSync(join(dir, 'repos'), { recursive: true })
  const addRepo = (name: string) => {
    const r = join(dir, 'repos', name)
    mkdirSync(r, { recursive: true })
    execFileSync(CLI, ['init', '--bare', r], { env: { ...process.env, ...CLI_ENV }, stdio: 'ignore' })
    writeFileSync(join(r, '.mediagit', 'config.toml'), `[storage]\nbackend = "filesystem"\nbase_path = "${fwd(join(dir, 'storage', name))}"\n`)
    return r
  }
  for (const n of repos) addRepo(n)
  const port = await freePort()
  writeFileSync(join(dir, 'server.toml'), `port = ${port}\nhost = "127.0.0.1"\nrepos_dir = "${fwd(join(dir, 'repos'))}"\n`)
  const log = join(dir, 'server.log')
  const out = openSync(log, 'a')
  const p = spawn(SERVER, ['--config', join(dir, 'server.toml')], { cwd: dir, env: { ...process.env }, stdio: ['ignore', out, out], windowsHide: true })
  if (!p.pid) throw new Error('could not start mediagit-server')
  trackPid(p.pid, 'mediagit-server')
  const base = `http://127.0.0.1:${port}`
  await waitFor('mediagit-server health', 30_000, async () => (await fetch(`${base}/health`)).ok)
  return { dir, port, base, pid: p.pid, log, url: (repo) => `${base}/${repo}`, addRepo, stop: () => stopTracked(p.pid!) }
}
