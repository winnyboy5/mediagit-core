import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, mockIpc, RECENT, showHandler } from './ipc-mock'

// W4b design-review fixes: plain-words hints, danger grouping, live counts, dirty settings.

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const PUSHED = { remote: 'origin', result: 'pushed', ref_updates: [], skipped_up_to_date: 0, bytes_uploaded: 0, objects_sent: 0 }

async function open(page: Page, route: string, heading: string, commands: Record<string, unknown> = {}) {
  await mockIpc(page, {
    commands: healthy({
      remote_list: fixture('remotes'),
      repo_push: PUSHED,
      repo_log: fixture('log_graph'),
      branch_list: fixture('branches'),
      reflog_list: fixture('reflog'),
      repo_revert: fixture('revert_ok'),
      repo_fsck: fixture('fsck_ok'),
      repo_verify: fixture('fsck_bad'),
      config_list: fixture('config_list'),
      config_schema: fixture('config_schema'),
      sparse_list: fixture('sparse_off'),
      tag_list: fixture('tags'),
      stash_list: fixture('stash_list'),
      ...commands,
    }),
    handlers: { repo_show: showHandler() },
    storage: REPO,
  })
  await page.goto(`/#${route}`, { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
}

test.describe('Sync: push options', () => {
  test('each overwrite mode says in a sentence what it does, flag second', async ({ page }) => {
    await open(page, '/sync', 'Sync')
    await page.getByTestId('sync-more').getByText('More options').click()
    const hint = page.getByTestId('overwrite-hint')
    await expect(hint).toContainText('Nothing is sent if someone else sent work')
    await page.getByTestId('push-risk').getByRole('button', { name: 'Overwrite safely', exact: true }).click()
    await expect(hint).toContainText('only if nobody sent new work since you last checked')
    await expect(hint.locator('code.flag')).toHaveText('--force-with-lease')
    await expectNoAxeViolations(page)
  })

  test('deleting lives in its own danger group and switches the overwrite control off', async ({ page }) => {
    await open(page, '/sync', 'Sync')
    await page.getByTestId('sync-more').getByText('More options').click()
    const del = page.getByTestId('push-delete')
    await expect(page.getByTestId('push-risk').getByRole('checkbox', { name: 'Delete these branches on the remote' })).toHaveCount(0)
    await del.getByRole('checkbox', { name: 'Delete these branches on the remote' }).click()
    for (const b of await page.getByTestId('push-risk').getByRole('button').all()) await expect(b).toBeDisabled()
    await expectNoAxeViolations(page)
  })
})

test.describe('Branches: delete dialog', () => {
  test('the description follows "delete even if not merged"', async ({ page }) => {
    await open(page, '/branches', 'Branches')
    await page.locator('[data-branch="feature/grade"]').getByRole('button', { name: 'Delete feature/grade' }).click()
    const dlg = page.getByRole('dialog', { name: 'Delete feature/grade?' })
    await expect(dlg).toContainText('already part of another branch')
    await dlg.getByText('More options').click()
    await dlg.getByRole('checkbox', { name: 'Delete even if its work is not merged anywhere' }).click()
    await expect(dlg).toContainText('will be lost from History')
    await expect(dlg).not.toContainText('already part of another branch')
  })
})

test.describe('Undo: revert dialog', () => {
  test('the button counts the versions that will be undone', async ({ page }) => {
    await open(page, '/history', 'History')
    await expect(page.getByRole('option')).toHaveCount(11)
    await page.getByRole('option').nth(1).click()
    await page.getByTestId('commit-detail').getByRole('button', { name: 'Undo…' }).click()
    await page.getByRole('menuitem', { name: 'Undo this version (adds a new one)' }).click()
    const dlg = page.getByRole('dialog')
    await expect(dlg.getByRole('button', { name: 'Undo this version' })).toBeVisible()
    await dlg.getByText('More options').click()
    await dlg.getByRole('group', { name: 'Also undo these versions' }).getByRole('checkbox').first().click()
    await expect(dlg.getByRole('button', { name: 'Undo 2 versions' })).toBeVisible()
    await expect(dlg).not.toContainText('COMMITS')
  })
})

test.describe('Reflog: clean-up', () => {
  test('opens valid: 30 is filled in and no error shows until touched', async ({ page }) => {
    await open(page, '/advanced', 'Advanced')
    await page.getByRole('tab', { name: 'Reflog' }).click()
    await page.getByText('Clean up this log').click()
    await expect(page.getByLabel('Entries to keep')).toHaveValue('30')
    await expect(page.getByRole('button', { name: 'Remove older entries…' })).toBeEnabled()
    await expect(page.getByText('Whole number from 0 to 999999')).toHaveCount(0)
  })
})

test.describe('Maintenance: verify result', () => {
  test('problems show in the result card and no toast covers them', async ({ page }) => {
    await open(page, '/maintenance', 'Maintenance')
    await page.locator('[data-task="verify"]').getByRole('button', { name: 'Verify' }).click()
    await expect(page.getByTestId('verify-result')).toContainText('Problems found')
    await expect(page.getByRole('button', { name: 'Dismiss', exact: true })).toHaveCount(0)
  })
})

test.describe('Settings: plain labels and unsaved changes', () => {
  test('friendly label, env var only on the key line, Not saved, and a leave warning', async ({ page }) => {
    await open(page, '/settings', 'Settings')
    const cfg = page.getByTestId('config')
    await expect(cfg.getByLabel('Files sent at once')).toBeVisible()
    const row = cfg.locator('[data-key="performance.upload_concurrency"]')
    await expect(row.locator('.note')).not.toContainText('MEDIAGIT_')
    await expect(row.locator('.key')).toContainText('MEDIAGIT_UPLOAD_CONCURRENCY')
    await expect(row).not.toContainText('Not saved')
    await row.getByLabel('Files sent at once').fill('3')
    await expect(row).toContainText('Not saved')
    await page.getByRole('link', { name: 'Changes' }).first().click()
    const dlg = page.getByRole('alertdialog', { name: 'Leave without saving?' })
    await expect(dlg).toBeVisible()
    await dlg.getByRole('button', { name: 'Keep editing' }).click()
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    await page.getByRole('link', { name: 'Changes' }).first().click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Leave and discard' }).click()
    await expect(page.getByRole('heading', { name: 'Changes', level: 1 })).toBeVisible()
  })
})

test.describe('Tags: disabled Tagger fields', () => {
  test('look disabled and say why', async ({ page }) => {
    await open(page, '/tags', 'Tags & Stash')
    await page.locator('form.save').getByText('More options').click()
    const f = page.getByLabel('Tagger name')
    await expect(f).toBeDisabled()
    await expect(page.getByText('Add a note first to set these.').first()).toBeVisible()
    await expect(f).toHaveCSS('cursor', 'not-allowed')
    expect(await ipcCalls(page, 'tag_create')).toHaveLength(0)
  })
})
