import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { fixture, healthy, mockIpc, RECENT, showHandler } from './ipc-mock'

// W4b (views-a) review screenshots: `SHOTS=1 pnpm e2e w4bashots`. Skipped in normal runs.
// Output: ui/.impeccable/review/w4b-<view>-<state>-<w>-<theme>.png.
test.skip(!process.env.SHOTS, 'screenshots only on demand')

const SIZES = [
  { w: 1440, h: 900 },
  { w: 1024, h: 700 },
]
const THEMES = ['light', 'dark'] as const
const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.changes.mode': JSON.stringify('flat') }
const TWO = { remotes: [{ name: 'origin', url: 'https://mediagit.studio.example/hero-film' }, { name: 'backup', url: 'https://backup.example/hero' }] }

async function shot(page: Page, name: string, theme: string, w: number) {
  mkdirSync('.impeccable/review', { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(350)
  await page.screenshot({ path: `.impeccable/review/w4b-${name}-${w}-${theme}.png` })
}

for (const { w, h } of SIZES) {
  for (const theme of THEMES) {
    const go = async (page: Page, hash: string, commands: Record<string, unknown> = {}) => {
      await page.setViewportSize({ width: w, height: h })
      await mockIpc(page, { commands: healthy({ remote_list: TWO, lock_list: fixture('locks'), branch_list: fixture('branches'), repo_log: fixture('log_graph'), reflog_list: fixture('reflog'), repo_revert: fixture('revert_ok'), repo_merge: fixture('merge_conflict'), config_list: fixture('config_list'), config_schema: fixture('config_schema'), sparse_list: fixture('sparse_off'), repo_diff: fixture('diff_working'), ...commands }),
        handlers: {
          repo_show: showHandler(),
          repo_extract: `(a) => ({ path: '/preview/' + a.rev.slice(0, 4) + '.png', size: a.expected ?? 1000 })`,
          media_info: `() => ({ format: 'png', width: 1920, height: 1080, file_size: 1363148 })`,
        }, storage: { ...(hash.includes('welcome') ? { 'mg.recent': RECENT } : REPO), 'mg.theme': JSON.stringify(theme) }, dialog: '/repos/new' })
      await page.goto(hash, { waitUntil: 'domcontentloaded' })
    }
    const name = (s: string) => `${s} ${w} ${theme}`

    test(name('sync-push-options'), async ({ page }) => {
      await go(page, '/#/sync')
      await page.getByTestId('sync-more').getByText('More options').click()
      await page.getByTestId('push-risk').getByRole('button', { name: 'Overwrite safely' }).click()
      await shot(page, 'sync-push-options', theme, w)
    })
    test(name('sync-push-confirm'), async ({ page }) => {
      await go(page, '/#/sync')
      await page.getByTestId('sync-more').getByText('More options').click()
      await page.getByTestId('push-risk').getByRole('button', { name: 'Overwrite anyway' }).click()
      await page.getByRole('button', { name: 'Send', exact: true }).first().click()
      await shot(page, 'sync-push-confirm', theme, w)
    })
    test(name('sync-add-remote'), async ({ page }) => {
      await go(page, '/#/sync')
      await page.getByRole('button', { name: 'Add remote' }).first().click()
      await page.getByRole('dialog').getByText('More options').click()
      await shot(page, 'sync-add-remote', theme, w)
    })
    test(name('welcome-init'), async ({ page }) => {
      await go(page, '/#/welcome')
      await page.getByRole('button', { name: /New repository/ }).click()
      await page.getByRole('dialog').getByText('More options').click()
      await shot(page, 'welcome-init', theme, w)
    })
    test(name('locks-lock'), async ({ page }) => {
      await go(page, '/#/locks')
      await page.getByRole('button', { name: 'Lock a file' }).click()
      await page.getByRole('dialog').getByText('More options').click()
      await shot(page, 'locks-lock', theme, w)
    })
    test(name('changes-options'), async ({ page }) => {
      await go(page, '/#/changes')
      await page.getByTestId('stage-more').getByText('Staging options').click()
      await page.getByTestId('commit-more').getByText('More options').click()
      await shot(page, 'changes-options', theme, w)
    })

    const row = (page: Page, n: string) => page.locator(`[data-branch="${n}"]`)
    const branchBtn = (page: Page, label: string) => row(page, 'feature/grade').getByRole('button', { name: label })
    test(name('branches-list'), async ({ page }) => {
      await go(page, '/#/branches')
      await expect(row(page, 'main')).toBeVisible()
      await shot(page, 'branches-list', theme, w)
    })
    test(name('branches-new-more'), async ({ page }) => {
      await go(page, '/#/branches')
      await page.getByRole('button', { name: 'New branch' }).click()
      await page.getByRole('dialog').getByText('More options').click()
      await shot(page, 'branches-new-more', theme, w)
    })
    test(name('branches-details'), async ({ page }) => {
      await go(page, '/#/branches')
      await branchBtn(page, 'Details of feature/grade').click()
      await shot(page, 'branches-details', theme, w)
    })
    test(name('branches-rename-overwrite'), async ({ page }) => {
      await go(page, '/#/branches')
      await branchBtn(page, 'Rename feature/grade').click()
      const dlg = page.getByRole('dialog', { name: 'Rename feature/grade' })
      await dlg.getByLabel('New name').fill('main')
      await dlg.getByText('More options').click()
      await dlg.getByRole('checkbox', { name: 'Replace the branch that already has this name' }).click()
      await shot(page, 'branches-rename-overwrite', theme, w)
    })
    test(name('branches-delete-anyway'), async ({ page }) => {
      await go(page, '/#/branches')
      await branchBtn(page, 'Delete feature/grade').click()
      const dlg = page.getByRole('dialog', { name: 'Delete feature/grade?' })
      await dlg.getByText('More options').click()
      await dlg.getByRole('checkbox', { name: 'Delete even if its work is not merged anywhere' }).click()
      await expect(dlg).toContainText('Work that only this branch has will be lost')
      await shot(page, 'branches-delete-anyway', theme, w)
    })
    test(name('history-filters'), async ({ page }) => {
      await go(page, '/#/history')
      await expect(page.getByRole('option').first()).toBeVisible()
      await page.getByRole('button', { name: 'Filters' }).click()
      await expect(page.getByRole('form', { name: 'Filter versions' })).toBeVisible()
      await shot(page, 'history-filters', theme, w)
    })
    test(name('history-undo-options'), async ({ page }) => {
      await go(page, '/#/history')
      await expect(page.getByRole('option')).toHaveCount(11)
      await page.getByRole('option').nth(1).click()
      await page.getByTestId('commit-detail').getByRole('button', { name: 'Undo…' }).click()
      await page.getByRole('menuitem', { name: 'Undo this version (adds a new one)' }).click()
      const dlg = page.getByRole('dialog')
      await dlg.getByText('More options').click()
      const boxes = dlg.getByRole('group', { name: 'Also undo these versions' }).getByRole('checkbox')
      await boxes.nth(0).click()
      await boxes.nth(1).click()
      await expect(dlg.getByRole('button', { name: 'Undo 3 versions' })).toBeVisible()
      await shot(page, 'history-undo-options', theme, w)
    })
    test(name('advanced-reflog-cleanup'), async ({ page }) => {
      await go(page, '/#/advanced')
      await page.getByRole('tab', { name: 'Reflog' }).click()
      await page.getByText('Clean up this log').click()
      await expect(page.getByLabel('Entries to keep')).toHaveValue('30')
      await shot(page, 'advanced-reflog-cleanup', theme, w)
    })
    test(name('merge-more'), async ({ page }) => {
      await go(page, '/#/merge')
      await page.getByRole('button', { name: 'Start a merge' }).first().click()
      await page.getByRole('dialog', { name: 'Start a merge' }).getByText('More options').click()
      await shot(page, 'merge-more', theme, w)
    })
    test(name('compare-staged'), async ({ page }) => {
      await go(page, '/#/compare')
      await page.getByLabel('To').click()
      await page.getByRole('option', { name: 'Ready to save (staged)' }).click()
      await page.getByRole('list', { name: 'Changed files' }).getByRole('button').first().click()
      await shot(page, 'compare-staged', theme, w)
    })
    test(name('sync-push-delete'), async ({ page }) => {
      await go(page, '/#/sync')
      await page.getByTestId('sync-more').getByText('More options').click()
      await page.getByRole('checkbox', { name: 'Delete these branches on the remote', exact: true }).click()
      await page.getByTestId('push-delete').scrollIntoViewIfNeeded()
      await shot(page, 'sync-push-delete', theme, w)
    })
    test(name('settings-dirty'), async ({ page }) => {
      await go(page, '/#/settings')
      const r = page.getByTestId('config').locator('[data-key="performance.upload_concurrency"]')
      await r.getByLabel('Files sent at once').fill('3')
      await expect(r).toContainText('Not saved')
      await r.scrollIntoViewIfNeeded()
      await shot(page, 'settings-dirty', theme, w)
    })
  }
}
