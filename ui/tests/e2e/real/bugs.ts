import { randomBytes } from 'node:crypto'
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import { mkdirSync, mkdtempSync, openSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { chromium, expect, type Browser, type Page } from '@playwright/test'
import { readState, type E2EState } from './setup'

// Shared by the bug0NN real-app specs: attach to the app, run the CLI, open a repo, start a throwaway server.

export interface Ctx { st: E2EState; browser: Browser; page: Page }

export async function attach(): Promise<Ctx> {
  const st = readState()
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${st.cdpPort}`)
  const c = browser.contexts()[0]
  if (!c) throw new Error('WebView2 exposed no browser context over CDP')
  const page = c.pages()[0] ?? (await c.waitForEvent('page'))
  await page.waitForLoadState('domcontentloaded')
  return { st, browser, page }
}

export const mg = (st: E2EState, cwd: string, args: string[], input?: string) =>
  execFileSync(st.cli, args, { cwd, env: { ...process.env, ...st.env, MEDIAGIT_CDC_SEED: '20260716' }, encoding: 'utf8', input })

export const tmp = (tag: string) => mkdtempSync(join(tmpdir(), `mg-${tag}-`))

/** Open `repo` from the Welcome page's recent list, the way a person would. */
export async function openRecent(page: Page, repo: string, name: string) {
  await page.evaluate(
    ([recent]) => {
      localStorage.removeItem('mg.repo')
      localStorage.setItem('mg.recent', recent!)
    },
    [JSON.stringify([{ path: repo, name, openedAt: Date.now(), branch: 'main' }])],
  )
  await page.evaluate(() => { window.location.hash = '#/welcome' })
  await page.reload()
  await page.getByTestId('recent-list').locator('button.open', { hasText: name }).click()
  await expect(page.getByTestId('repo-switcher')).toContainText(name)
  await expect(page).toHaveURL(/files[/]all/)
}

/** The native folder picker cannot be driven over CDP: answer the dialog plugin's IPC fetch with `dir`. */
export async function stubPicker(page: Page, dir: string) {
  await page.evaluate((d) => {
    const w = window as unknown as { fetch: typeof fetch }
    const real = w.fetch.bind(window)
    w.fetch = (input, init) =>
      String(input instanceof Request ? input.url : input).includes('plugin%3Adialog')
        ? Promise.resolve(new Response(JSON.stringify(d), { headers: { 'Content-Type': 'application/json', 'Tauri-Response': 'ok' } }))
        : real(input, init)
  }, dir)
}

const freePort = () =>
  new Promise<number>((ok, fail) => {
    const s = createServer()
    s.once('error', fail)
    s.listen(0, '127.0.0.1', () => {
      const port = (s.address() as { port: number }).port
      s.close(() => ok(port))
    })
  })

export interface Srv { url: string; stop: () => void }

/** A throwaway local server with one empty repo `name`; `auth` turns on sign-in with open sign-up. */
export async function startServer(st: E2EState, name: string, auth: boolean): Promise<Srv> {
  const exe = process.env.MG_E2E_SERVER ?? join(dirname(st.cli), 'mediagit-server.exe')
  const dir = tmp('srv')
  const fwd = (p: string) => p.replace(/\\/g, '/')
  const repo = join(dir, 'repos', name)
  mkdirSync(repo, { recursive: true })
  mg(st, dir, ['init', '--bare', repo])
  writeFileSync(join(repo, '.mediagit', 'config.toml'), `[storage]\nbackend = "filesystem"\nbase_path = "${fwd(join(dir, 'storage'))}"\n`)
  const port = await freePort()
  const authCfg = auth ? 'enable_auth = true\nallow_open_registration = true\njwt_secret = "' + randomBytes(24).toString('hex') + '"\n' : ''
  writeFileSync(join(dir, 'server.toml'), `port = ${port}\nhost = "127.0.0.1"\nrepos_dir = "${fwd(join(dir, 'repos'))}"\nenable_rate_limiting = false\n${authCfg}`)
  const out = openSync(join(dir, 'server.log'), 'a')
  const p: ChildProcess = spawn(exe, ['--config', join(dir, 'server.toml')], { stdio: ['ignore', out, out], windowsHide: true })
  const url = `http://127.0.0.1:${port}`
  const stop = () => {
    try { execFileSync('taskkill', ['/PID', String(p.pid), '/T', '/F'], { stdio: 'ignore' }) } catch { /* gone */ }
  }
  for (let i = 0; i < 60; i++) {
    try { if ((await fetch(`${url}/health`)).ok) return { url, stop } } catch { /* not yet */ }
    await new Promise((r) => setTimeout(r, 500))
  }
  stop()
  throw new Error(`server did not come up; see ${join(dir, 'server.log')}`)
}
