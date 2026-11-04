import { execFileSync } from 'node:child_process'
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { chromium, expect, test, type Browser, type Page } from '@playwright/test'
import { readState, type E2EState } from './setup'

// Gate G4, wave W5: the Server panel in the REAL app. The app starts its own local
// mediagit-server (bundled sidecar, isolated data folder). Flow: set up -> start -> add a
// person -> make an API key -> create a repository -> push to it and clone it with that key
// -> revoke the key -> stop. Every step cross-checks the CLI. Runs after w3.spec.ts.

test.describe.configure({ mode: 'serial' })

const shots = resolve(import.meta.dirname, '..', '..', '..', 'test-results', 'real')
const ADMIN = 'e2eadmin'
const ADMIN_PASSWORD = 'Quartz-Lantern-74!' // a test value for this throwaway server
let st: E2EState
let browser: Browser
let page: Page
let url = ''
let apiKey = ''

const env = (extra: Record<string, string> = {}) => ({ ...process.env, ...st.env, MEDIAGIT_CDC_SEED: '20260716', ...extra })
const cli = (cwd: string, extra: Record<string, string>, ...args: string[]) => execFileSync(st.cli, args, { cwd, env: env(extra), encoding: 'utf8' })
const cliJson = (...args: string[]) => (JSON.parse(cli(st.root, {}, '--machine', ...args)) as { ok: boolean; data: Record<string, any> }).data // eslint-disable-line @typescript-eslint/no-explicit-any
const shot = (name: string) => page.screenshot({ path: join(shots, `${name}.png`) })
const go = async (tab?: string) => {
  await page.evaluate(() => { window.location.hash = '#/locks' })
  await page.evaluate(() => { window.location.hash = '#/server' })
  await expect(page.getByRole('heading', { name: 'Server', level: 1 })).toBeVisible()
  if (tab) await page.getByRole('tab', { name: tab }).click()
}
const healthz = async () => {
  try {
    return (await fetch(`${url}/healthz`)).ok
  } catch {
    return false
  }
}

test.beforeAll(async () => {
  st = readState()
  mkdirSync(shots, { recursive: true })
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${st.cdpPort}`)
  const ctx = browser.contexts()[0]
  if (!ctx) throw new Error('WebView2 exposed no browser context over CDP')
  page = ctx.pages()[0] ?? (await ctx.waitForEvent('page'))
  await page.waitForLoadState('domcontentloaded')
  page.on('pageerror', (e) => appendFileSync(join(st.root, 'logs', 'pageerror.log'), `${e}\n`))
})

test.afterAll(async () => {
  // The CLI keeps the admin sign-in in the OS keychain: take it out again.
  if (url) {
    try {
      cli(st.root, {}, '--machine', 'auth', 'logout', `--server=${url}`)
    } catch {
      // never signed in
    }
  }
  await browser?.close().catch(() => {})
})

test('a. set up the local server: first admin, running, signed in', async () => {
  await go()
  await page.getByRole('button', { name: 'Set up local server…' }).click()
  const dlg = page.getByRole('dialog', { name: 'Set up local server' })
  await dlg.getByLabel('Admin username').fill(ADMIN)
  await dlg.getByLabel('Admin email').fill('e2e-admin@example.test')
  await dlg.getByLabel('Password', { exact: true }).fill(ADMIN_PASSWORD)
  await dlg.getByLabel('Repeat password').fill(ADMIN_PASSWORD)
  await dlg.getByRole('button', { name: 'Set up and start' }).click()
  await expect(page.getByTestId('local-state')).toHaveText('Running', { timeout: 30_000 })
  url = (await page.getByTestId('local-card').locator('dt:text-is("Address") + dd .mono').innerText()).trim()
  expect(url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
  expect(await healthz()).toBe(true)
  await expect(page.getByTestId('me-line')).toContainText(`Signed in as ${ADMIN}`)
  const who = cliJson('auth', 'status', `--server=${url}`)
  expect(who.logged_in).toBe(true)
  expect(who.identity.role).toBe('admin')
  // The password reached no page text, storage or log line.
  const seen = await page.evaluate(() => document.body.innerText + JSON.stringify({ ...localStorage }))
  expect(seen).not.toContain(ADMIN_PASSWORD)
  await shot('w5-01-running')
})

test('b. people: add a person, the server lists them', async () => {
  await go('People')
  await page.getByRole('button', { name: 'Add person…' }).click()
  const dlg = page.getByRole('dialog', { name: 'Add person' })
  await dlg.getByLabel('Username').fill('lena')
  await dlg.getByLabel('Email').fill('lena@example.test')
  await dlg.getByLabel('Starting password').fill('Harbor-Violet-58!')
  await dlg.getByRole('button', { name: 'Add person' }).click()
  await expect(page.getByTestId('people')).toContainText('lena')
  const users = cliJson('auth', 'admin', 'list-users', `--server=${url}`).users as { username: string; role: string }[]
  expect(users.find((u) => u.username === 'lena')?.role).toBe('write')
  await shot('w5-02-people')
})

test('c. API key: shown once, then listed', async () => {
  await go('API keys')
  await page.getByRole('button', { name: 'New key…' }).click()
  await page.getByRole('dialog', { name: 'New API key' }).getByLabel('Name').fill('e2e-key')
  await page.getByRole('dialog', { name: 'New API key' }).getByRole('button', { name: 'Make key' }).click()
  const once = page.getByRole('dialog', { name: 'Copy your new key' })
  apiKey = (await once.getByTestId('new-key-value').innerText()).trim()
  expect(apiKey.length).toBeGreaterThan(16)
  await once.getByRole('checkbox', { name: 'I saved it' }).click()
  await once.getByRole('button', { name: 'Done' }).click()
  await expect(page.getByTestId('keys')).toContainText('e2e-key')
  const keys = cliJson('auth', 'key', 'list', `--server=${url}`).keys as { name: string }[]
  expect(keys.map((k) => k.name)).toContain('e2e-key')
  expect(await page.evaluate(() => document.body.innerText)).not.toContain(apiKey)
})

test('d. repositories: create one, push to it and clone it with the key', async () => {
  await go('Repositories')
  await page.getByRole('button', { name: 'New repository…' }).click()
  await page.getByRole('dialog', { name: 'New repository' }).getByLabel('Name').fill('e2e-repo')
  await page.getByRole('dialog', { name: 'New repository' }).getByRole('button', { name: 'Create repository' }).click()
  await expect(page.getByTestId('repos')).toContainText('e2e-repo')
  const hosted = cliJson('remote', 'repos', 'list', `--server=${url}`).repos as { name: string }[]
  expect(hosted.map((r) => r.name)).toContain('e2e-repo')

  const key = { MEDIAGIT_API_KEY: apiKey }
  const work = join(st.root, 'w5-work')
  const copy = join(st.root, 'w5-clone')
  mkdirSync(work)
  cli(work, key, 'init')
  writeFileSync(join(work, 'notes.txt'), 'pushed with an api key\n')
  cli(work, key, 'add', '.')
  cli(work, key, 'commit', '-m', 'first')
  cli(work, key, 'remote', 'add', 'origin', `${url}/e2e-repo`)
  cli(work, key, 'push', '-u', 'origin', 'main')
  cli(st.root, key, 'clone', `${url}/e2e-repo`, copy)
  expect(cli(copy, key, 'log', '--oneline')).toContain('first')
  await go('Repositories')
  await expect(page.getByTestId('repos').getByRole('row', { name: /e2e-repo/ })).not.toContainText('Nothing pushed yet')
  await shot('w5-03-repos')
})

test('e. revoking the key stops it working', async () => {
  await go('API keys')
  await page.getByRole('button', { name: 'Revoke key e2e-key' }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: 'Revoke key' }).click()
  await expect(page.getByText('You have no API keys')).toBeVisible()
  const keys = cliJson('auth', 'key', 'list', `--server=${url}`).keys as { name: string }[]
  expect(keys.map((k) => k.name)).not.toContain('e2e-key')
  const work = join(st.root, 'w5-work')
  writeFileSync(join(work, 'more.txt'), 'after revoke\n')
  cli(work, {}, 'add', '.')
  cli(work, {}, 'commit', '-m', 'second')
  expect(() => cli(work, { MEDIAGIT_API_KEY: apiKey }, 'push', 'origin', 'main')).toThrow()
})

test('f. stop the server: it stops answering', async () => {
  await go()
  await page.getByRole('button', { name: 'Stop server…' }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: 'Stop server' }).click()
  await expect(page.getByTestId('local-state')).toHaveText('Stopped', { timeout: 15_000 })
  await expect.poll(healthz, { timeout: 10_000 }).toBe(false)
  await shot('w5-04-stopped')
})
