import { execFileSync } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { chromium, expect, test, type Browser, type Page } from '@playwright/test'
import { readState, type E2EState } from './setup'

// Gate G4, wave W6: the file watcher in the REAL app. A file written from outside
// must reach the Files badge with no focus change and no manual refresh.

let st: E2EState
let browser: Browser
let page: Page

test.beforeAll(async () => {
  st = readState()
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${st.cdpPort}`)
  const ctx = browser.contexts()[0]
  if (!ctx) throw new Error('WebView2 exposed no browser context over CDP')
  page = ctx.pages()[0] ?? (await ctx.waitForEvent('page'))
  await page.waitForLoadState('domcontentloaded')
})

test.afterAll(async () => {
  await browser?.close().catch(() => {})
})

test('watcher: an outside write updates the Files count within 3 s, no refresh', async () => {
  const repo = join(st.root, 'watch-repo')
  mkdirSync(repo)
  execFileSync(st.cli, ['init', '-q'], { cwd: repo, env: { ...process.env, ...st.env }, encoding: 'utf8' })

  await page.evaluate(
    ([p, recent]) => {
      localStorage.setItem('mg.repo', JSON.stringify(p))
      localStorage.setItem('mg.recent', recent!)
    },
    [repo, JSON.stringify([{ path: repo, name: 'watch-repo', openedAt: Date.now(), branch: 'main' }])] as [string, string],
  )
  await page.evaluate(() => { window.location.hash = '#/files/changes' })
  await page.reload()
  const nav = page.getByRole('navigation', { name: 'Main' })
  await expect(nav).toBeVisible()
  await expect(page.getByTestId('repo-switcher')).toContainText('watch-repo')
  await expect(page.getByText('No changes')).toBeVisible()
  await expect(nav.getByLabel(/changed files?$/)).toHaveCount(0)
  // Let the repo-open path (which starts the watcher) settle.
  await page.waitForTimeout(1500)

  writeFileSync(join(repo, 'shot-010.txt'), 'written by the test, not the app\n')
  await expect(nav.getByLabel('1 changed file')).toBeVisible({ timeout: 3000 })

  rmSync(join(repo, 'shot-010.txt'))
  await expect(nav.getByLabel(/changed files?$/)).toHaveCount(0, { timeout: 3000 })
})
