import { execFileSync } from 'node:child_process'
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { chromium, expect, test, type Browser, type Page } from '@playwright/test'
import { expectNoAxeViolations } from '../mocked/helpers'
import { readState, type E2EState } from './setup'

// Gate G4: the REAL Tauri app (WebView2 over CDP) against a real repo, server and CLI.
// Serial: each test builds on the state the previous one left, and every test checks
// both what the user sees and what the CLI says is on disk.

test.describe.configure({ mode: 'serial' })

const shots = resolve(import.meta.dirname, '..', '..', '..', 'test-results', 'real')
let st: E2EState
let browser: Browser
let page: Page

const env = () => ({ ...process.env, ...st.env, MEDIAGIT_CDC_SEED: '20260716' })
const cli = (cwd: string, ...args: string[]) => execFileSync(st.cli, args, { cwd, env: env(), encoding: 'utf8' })
const shot = (name: string) => page.screenshot({ path: join(shots, `${name}.png`) })
const go = async (view: string, heading: string) => {
  await page.evaluate((v) => { window.location.hash = `#/${v}` }, view)
  await expect(page.getByRole('heading', { name: heading, level: 2 })).toBeVisible()
}
const refresh = () => page.evaluate(() => window.dispatchEvent(new Event('focus')))
const unstaged = () => page.getByRole('tree', { name: 'Files not staged' })
const staged = () => page.getByRole('tree', { name: 'Staged files' })

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
  await browser?.close().catch(() => {}) // detaches; teardown stops the app by PID
})

test('a. Welcome shows the engine version from the real engine_info', async () => {
  await page.evaluate(
    ([recent]) => {
      localStorage.removeItem('mg.repo')
      localStorage.setItem('mg.recent', recent!)
    },
    [JSON.stringify([{ path: st.repo, name: 'hero-film', openedAt: Date.now(), branch: 'main' }])],
  )
  await page.evaluate(() => { window.location.hash = '#/welcome' })
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Welcome' })).toBeVisible()
  const want = cli(st.repo, '--version').trim().split(/\s+/).pop()!
  await expect(page.getByText(`mediagit ${want}`)).toBeVisible()
  await shot('01-welcome')
})

test('b. open the fixture repo from Recent: sidebar shows repo and main; Changes is clean', async () => {
  await page.getByTestId('recent-list').locator('button.open', { hasText: 'hero-film' }).click()
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
  await expect(page.getByTestId('repo-switcher')).toContainText('hero-film')
  await expect(page.getByTestId('branch-indicator')).toContainText('main')
  await go('changes', 'Changes')
  await expect(page.getByText('No changes')).toBeVisible()
  expect(cli(st.repo, 'status', '--porcelain').trim()).toBe('')
  await shot('02-changes-clean')
})

test('c. on-disk edits show as modified and untracked; inspector shows the selection', async () => {
  writeFileSync(join(st.repo, 'notes.txt'), 'second draft, edited outside the app\n')
  writeFileSync(join(st.repo, 'new-shot.txt'), 'brand new\n')
  await refresh()
  await expect(unstaged().getByRole('treeitem', { name: /notes\.txt/ })).toBeVisible()
  await expect(unstaged().getByRole('treeitem', { name: /new-shot\.txt/ })).toBeVisible()
  await unstaged().getByRole('treeitem', { name: /notes\.txt/ }).click()
  await expect(page.getByTestId('inspector').getByText('notes.txt').first()).toBeVisible()
  const porcelain = cli(st.repo, 'status', '--porcelain')
  expect(porcelain).toContain('notes.txt')
  expect(porcelain).toContain('new-shot.txt')
  await shot('03-changes-dirty')
})

test('d. stage both and commit in the UI; the CLI agrees', async () => {
  await page.getByRole('button', { name: 'Stage all', exact: true }).click()
  await expect(staged().getByRole('treeitem')).toHaveCount(2)
  await page.getByLabel('Message').fill('G4 edit and add')
  await page.getByRole('button', { name: 'Save version (commit)' }).click()
  await expect(page.getByText('No changes')).toBeVisible()
  expect(cli(st.repo, 'status', '--porcelain').trim()).toBe('')
  expect(cli(st.repo, 'log', '--oneline', '-n', '1')).toContain('G4 edit and add')
  await shot('04-changes-committed')
})

test('e. History shows the new commit on top with graph and changed files', async () => {
  await go('history', 'History')
  const rows = page.getByRole('listbox', { name: 'Versions, newest first' }).getByRole('option')
  await expect(rows.first()).toContainText('G4 edit and add')
  await expect(rows.first().locator('svg.graph-cell')).toHaveCount(1)
  await expect(rows).toHaveCount(2)
  const detail = page.getByTestId('commit-detail')
  await expect(detail.getByRole('heading', { name: 'G4 edit and add' })).toBeVisible()
  await expect(detail.getByRole('tab', { name: /Changes in this version · 2/ })).toBeVisible()
  await expect(detail).toContainText('notes.txt')
  await expect(detail).toContainText('new-shot.txt')
  await shot('05-history')
})

test('f. Branches: create feature/g4, switch to it and back', async () => {
  await go('branches', 'Branches')
  const row = (n: string) => page.locator(`[data-branch="${n}"]`)
  await page.getByRole('button', { name: 'New branch' }).click()
  const dlg = page.getByRole('dialog', { name: 'New branch' })
  await dlg.getByLabel('Branch name').fill('feature/g4')
  await dlg.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(row('feature/g4')).toBeVisible()
  expect(cli(st.repo, 'branch', 'list')).toContain('feature/g4')
  await row('feature/g4').getByRole('button', { name: 'Switch to', exact: true }).click()
  await expect(page.getByTestId('branch-indicator')).toContainText('feature/g4')
  expect(cli(st.repo, 'branch', 'list')).toMatch(/^\*\s+feature\/g4/m)
  await shot('06-branches-on-feature')
  await row('main').getByRole('button', { name: 'Switch to', exact: true }).click()
  await expect(page.getByTestId('branch-indicator')).toContainText('main')
  expect(cli(st.repo, 'branch', 'list')).toMatch(/^\*\s+main/m)
})

test('g. Locks: lock the binary in the UI, see it listed, unlock; CLI agrees', async () => {
  await go('locks', 'Locks')
  await page.getByRole('button', { name: 'Lock a file' }).first().click()
  const dlg = page.getByRole('dialog', { name: 'Lock a file' })
  await dlg.getByLabel('File path').fill('art/hero.bin')
  await dlg.getByRole('button', { name: 'Lock file' }).click()
  const row = page.locator('[data-lock="art/hero.bin"]')
  await expect(row).toBeVisible()
  expect(cli(st.repo, 'lock', 'list')).toContain('art/hero.bin')
  await shot('07-locks')
  await row.getByRole('button', { name: 'Unlock' }).click()
  await expect(row).toHaveCount(0)
  await expect(page.getByRole('alert')).toHaveCount(0)
  await shot('07-locks-after-unlock')
  expect(cli(st.repo, 'lock', 'list')).not.toContain('art/hero.bin')
})

test('h. Sync: Send, then Get latest brings in a commit pushed from a second clone', async () => {
  await go('sync', 'Sync')
  await page.getByRole('button', { name: 'Send', exact: true }).click()
  await expect(page.getByText(/^Sent /).first()).toBeVisible({ timeout: 45_000 })
  await shot('08-sync-sent')
  expect(cli(st.repo, 'status', '-b', '--porcelain')).not.toMatch(/ahead/i)

  cli(st.root, 'clone', st.remoteUrl, st.clone2)
  writeFileSync(join(st.clone2, 'from-clone.txt'), 'pushed by the second clone\n')
  cli(st.clone2, 'add', '.')
  cli(st.clone2, 'commit', '-m', 'Second clone commit')
  cli(st.clone2, 'push', 'origin', 'main')

  // Watch for the progress panel appearing at all: a small pull can finish quickly.
  await page.evaluate(() => {
    const w = window as unknown as { __sawProgress: boolean }
    w.__sawProgress = false
    new MutationObserver(() => {
      if (document.querySelector('[data-testid="transfer-progress"]')) w.__sawProgress = true
    }).observe(document.body, { childList: true, subtree: true })
  })
  await page.getByRole('button', { name: 'Get latest', exact: true }).click()
  await expect.poll(() => existsSync(join(st.repo, 'from-clone.txt')), { timeout: 45_000 }).toBe(true)
  expect(await page.evaluate(() => (window as unknown as { __sawProgress: boolean }).__sawProgress)).toBe(true)
  expect(readFileSync(join(st.repo, 'from-clone.txt'), 'utf8')).toContain('second clone')
  await expect(page.getByRole('button', { name: 'Get latest', exact: true })).toBeEnabled()
  await shot('09-sync-pulled')
  await go('history', 'History')
  const rows = page.getByRole('listbox', { name: 'Versions, newest first' }).getByRole('option')
  await expect(rows.first()).toContainText('Second clone commit')
  expect(cli(st.repo, 'log', '--oneline', '-n', '1')).toContain('Second clone commit')
})

test('i. accessibility: no axe violations on Changes, History, Branches, Sync', async () => {
  for (const [view, heading] of [['changes', 'Changes'], ['history', 'History'], ['branches', 'Branches'], ['sync', 'Sync']] as const) {
    await go(view, heading)
    await page.waitForTimeout(500)
    await shot(`10-axe-${view}`)
    await expectNoAxeViolations(page)
  }
})
