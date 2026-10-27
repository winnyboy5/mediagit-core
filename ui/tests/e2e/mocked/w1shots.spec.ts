import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { fixture, healthy, ipcRelease, mockIpc, RECENT, showHandler } from './ipc-mock'

// W1 review screenshots: `SHOTS=1 pnpm e2e w1shots`. Skipped in normal runs.
// Output: ui/.impeccable/review/w1-<view>-<state>-<w>-<theme>.png
test.skip(!process.env.SHOTS, 'screenshots only on demand')

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.lock.owner': JSON.stringify('Mira Okafor') }
const SIZES = [
  { w: 1440, h: 900 },
  { w: 1024, h: 700 },
]

async function shot(page: Page, name: string, theme: string, w: number) {
  mkdirSync('.impeccable/review', { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(350)
  await page.screenshot({ path: `.impeccable/review/w1-${name}-${w}-${theme}.png` })
}

async function open(page: Page, route: string, heading: string, extra: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({
      repo_log: fixture('log_graph'),
      branch_list: fixture('branches'),
      lock_list: fixture('locks'),
      remote_list: fixture('remotes'),
      media_info: fixture('media_png'),
      ...extra,
    }),
    handlers: { repo_show: showHandler() },
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto(`/#${route}`, { waitUntil: "domcontentloaded" })
  await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
}

for (const { w, h } of SIZES) {
  test.describe(`${w}px`, () => {
    test.use({ viewport: { width: w, height: h } })

    test('V3 history', async ({ page }, info) => {
      const theme = info.project.name
      await open(page, '/history', 'History', {}, { repo_extract: [{ resolve: { path: '/tmp/p.png', size: 1363148 } }] })
      await expect(page.getByRole('option')).toHaveCount(11)
      await shot(page, 'V3-history-populated', theme, w)
      await page.getByRole('tab', { name: 'Files at this version' }).click()
      await page.getByRole('tree', { name: 'Files at this version' }).getByRole('treeitem', { name: /hero_preview/ }).click()
      if (w >= 1200) await expect(page.getByRole('img', { name: /Preview of/ })).toBeVisible()
      await shot(page, 'V3-history-tree', theme, w)
      await page.getByRole('searchbox', { name: 'Search versions' }).fill('sam')
      await shot(page, 'V3-history-search', theme, w)
    })

    test('V3 history error', async ({ page }, info) => {
      await open(page, '/history', 'History', { repo_log: { __error: { code: 'error', message: 'Could not read history.' } } })
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V3-history-error', info.project.name, w)
    })

    test('V4 inspector', async ({ page }, info) => {
      const s = fixture<{ modified: string[] }>('status_populated')
      await open(page, '/changes', 'Changes', { repo_status: { ...s, modified: [...s.modified, 'art/hero_preview.png'] } })
      await page.getByRole('tree', { name: 'Files not staged' }).getByRole('treeitem', { name: /hero_preview/ }).click()
      if (w < 1200) await page.getByRole('button', { name: 'Show details panel' }).click()
      await expect(page.getByTestId('media-details')).toContainText('1920')
      await shot(page, 'V4-inspector-image', info.project.name, w)
      await page.getByRole('tree', { name: 'Staged files' }).getByRole('treeitem', { name: /hero\.psd/ }).click()
      await expect(page.getByTestId('preview')).toContainText('No preview')
      await shot(page, 'V4-inspector-psd', info.project.name, w)
    })

    test('V5 branches', async ({ page }, info) => {
      const theme = info.project.name
      await open(page, '/branches', 'Branches', {}, { branch_switch: fixture('branch_switch_progress') })
      await expect(page.locator('[data-branch]')).toHaveCount(3)
      await shot(page, 'V5-branches-populated', theme, w)
      await page.locator('[data-branch="feature/grade"]').getByRole('button', { name: 'Switch to' }).click()
      await ipcRelease(page)
      await expect(page.getByTestId('switch-progress')).toContainText('18 / 40')
      await shot(page, 'V5-branches-busy', theme, w)
    })

    test('V5 branches error and dialog', async ({ page }, info) => {
      const theme = info.project.name
      await open(page, '/branches', 'Branches', {}, { branch_switch: fixture('branch_switch_dirty') })
      await page.locator('[data-branch="review/cut-2"]').getByRole('button', { name: 'Switch to' }).click()
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V5-branches-error', theme, w)
      await page.getByRole('button', { name: 'New branch' }).click()
      await page.getByLabel('Branch name').fill('bad name')
      await shot(page, 'V5-branches-dialog', theme, w)
    })

    test('V6 locks', async ({ page }, info) => {
      const theme = info.project.name
      await open(page, '/locks', 'Locks')
      await expect(page.getByRole('table')).toBeVisible()
      await shot(page, 'V6-locks-populated', theme, w)
      await page.locator('[data-lock="char/rig.fbx"]').getByRole('button', { name: 'Force unlock…' }).click()
      await shot(page, 'V6-locks-force', theme, w)
    })

    test('V6 locks error', async ({ page }, info) => {
      await open(page, '/locks', 'Locks', { lock_list: { __error: { code: 'error', message: 'No remote configured.' } } })
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V6-locks-error', info.project.name, w)
    })

    test('V7 sync', async ({ page }, info) => {
      const theme = info.project.name
      await open(page, '/sync', 'Sync', {}, { repo_push: fixture('push_progress') })
      await expect(page.getByTestId('sync-state')).toBeVisible()
      await shot(page, 'V7-sync-populated', theme, w)
      await page.getByRole('button', { name: 'Send', exact: true }).click()
      await ipcRelease(page)
      await expect(page.getByRole('progressbar', { name: 'Uploading media' })).toBeVisible()
      await shot(page, 'V7-sync-busy', theme, w)
    })

    test('V7 sync failure', async ({ page }, info) => {
      await open(page, '/sync', 'Sync', {}, { repo_push: fixture('push_fail') })
      await page.getByRole('button', { name: 'Send', exact: true }).click()
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V7-sync-error', info.project.name, w)
    })

    test('V7 sync empty', async ({ page }, info) => {
      await open(page, '/sync', 'Sync', { remote_list: { remotes: [] } })
      await expect(page.getByText('No remote yet')).toBeVisible()
      await shot(page, 'V7-sync-empty', info.project.name, w)
    })
  })
}
