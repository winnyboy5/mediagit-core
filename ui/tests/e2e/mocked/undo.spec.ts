import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT, showHandler } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const SECOND = '0001000100010001000100010001000100010001'

const detail = (page: Page) => page.getByTestId('commit-detail')
const undoMenu = (page: Page) => detail(page).getByRole('button', { name: 'Undo…' })

async function boot(page: Page, route: string, heading: string, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({
      repo_log: fixture('log_graph'),
      repo_extract: { path: '/tmp/p.png', size: 1363148 },
      repo_reset: fixture('reset_ok'),
      repo_reset_hard: { mode: 'hard', oid: SECOND, unstaged: null },
      repo_revert: fixture('revert_ok'),
      repo_restore_file: {},
      ...commands,
    }),
    handlers: { repo_show: showHandler() },
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto(`/#${route}`, { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
}

/** History with the second version selected, so the undo target differs from HEAD. */
async function history(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await boot(page, '/history', 'History', commands, scripts)
  await expect(page.getByRole('option')).toHaveCount(11)
  await page.getByRole('option').nth(1).click()
  await expect(undoMenu(page)).toBeEnabled()
}

const choose = async (page: Page, name: string) => {
  await undoMenu(page).click()
  await page.getByRole('menuitem', { name }).click()
}

test.describe('Undo: menu and dialogs', () => {
  test('the Undo menu lists the four actions, the destructive one last and marked; no axe violations', async ({ page }) => {
    await history(page)
    await undoMenu(page).click()
    const items = page.getByRole('menuitem')
    await expect(items).toHaveText([
      'Undo this version (adds a new one)',
      'Move branch back here, keep changes ready to save',
      'Move branch back here, keep changes in my files',
      'Discard everything back to here…',
    ])
    await expectNoAxeViolations(page)
    await page.keyboard.press('Escape')
    await expect(items).toHaveCount(0)
    expect(await ipcCalls(page, 'repo_reset')).toHaveLength(0)
  })

  test('move back, keep changes ready: explains, then resets softly to the chosen version', async ({ page }) => {
    await history(page)
    await choose(page, 'Move branch back here, keep changes ready to save')
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Move this branch back to 0001000?')
    await expect(dlg).toContainText('your files are not touched')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'repo_reset')).toHaveLength(0)
    await choose(page, 'Move branch back here, keep changes ready to save')
    await page.getByRole('alertdialog').getByRole('button', { name: 'Move branch back' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_reset')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', rev: SECOND, mode: 'soft' },
    ])
    await expect(page.getByText('Branch moved; your files were not touched').first()).toBeVisible()
  })

  test('move back, keep changes in files: mixed reset', async ({ page }) => {
    await history(page)
    await choose(page, 'Move branch back here, keep changes in my files')
    await expect(page.getByRole('alertdialog')).toContainText('not yet staged')
    await page.getByRole('alertdialog').getByRole('button', { name: 'Move branch back' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_reset')).map((c) => c.args.mode)).toEqual(['mixed'])
  })

  test('revert asks first, then adds an undo version and reloads history', async ({ page }) => {
    await history(page)
    const before = (await ipcCalls(page, 'repo_log')).length
    await choose(page, 'Undo this version (adds a new one)')
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Undo Fix audio sync on VO take 2?')
    await expect(dlg).toContainText('Nothing is deleted')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Undo this version' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_revert')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', rev: SECOND }])
    await expect.poll(async () => (await ipcCalls(page, 'repo_log')).length).toBeGreaterThan(before)
    await expect(page.getByText('Undone with a new version').first()).toBeVisible()
  })

  test('a revert that stops on clashes names the files and can be abandoned', async ({ page }) => {
    await history(page, { repo_revert: fixture('revert_conflict'), repo_revert_control: { action: 'abort', status: 'aborted', commits: [], conflicts: [] } })
    await choose(page, 'Undo this version (adds a new one)')
    await page.getByRole('alertdialog').getByRole('button', { name: 'Undo this version' }).click()
    const banner = page.getByTestId('revert-stopped')
    await expect(banner).toContainText('1 file clashes (art/hero_preview.png)')
    await expectNoAxeViolations(page)
    await banner.getByRole('button', { name: 'Abandon undo' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_revert_control')).map((c) => c.args.action)).toEqual(['abort'])
    await expect(banner).toHaveCount(0)
  })

  test('discard everything: the dialog counts what is lost and gates the button on a checkbox', async ({ page }) => {
    await history(page)
    await choose(page, 'Discard everything back to here…')
    const dlg = page.getByRole('dialog', { name: 'Discard everything back to 0001000?' })
    await expect(dlg).toContainText('6 files with unsaved changes will be lost.')
    await expect(dlg).toContainText('Your files are replaced with how they were at 0001000.')
    await expect(dlg).toContainText('Versions after it leave this branch.')
    const go = dlg.getByRole('button', { name: 'Discard and go back' })
    await expect(go).toBeDisabled()
    await expectNoAxeViolations(page)
    await dlg.getByRole('checkbox', { name: 'I understand my unsaved work will be lost' }).click()
    await expect(go).toBeEnabled()
    await dlg.getByRole('button', { name: 'Keep my files' }).click()
    await expect(dlg).toHaveCount(0)
    expect(await ipcCalls(page, 'repo_reset_hard')).toHaveLength(0)
    await choose(page, 'Discard everything back to here…')
    await expect(page.getByRole('button', { name: 'Discard and go back' })).toBeDisabled()
    await page.getByRole('checkbox', { name: 'I understand my unsaved work will be lost' }).click()
    await page.getByRole('button', { name: 'Discard and go back' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_reset_hard')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', rev: SECOND }])
    expect(await ipcCalls(page, 'repo_reset')).toHaveLength(0)
  })

  test('busy: a hard reset shows determinate files progress', async ({ page }) => {
    await history(page, {}, { repo_reset_hard: fixture('branch_switch_progress') })
    await choose(page, 'Discard everything back to here…')
    await page.getByRole('checkbox', { name: 'I understand my unsaved work will be lost' }).click()
    await page.getByRole('button', { name: 'Discard and go back' }).click()
    const bar = page.getByTestId('undo-progress')
    await expect(bar).toContainText('Go back to 0001000 and replace files · 0 / 40 files')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '40')
    await expect(undoMenu(page)).toBeDisabled()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bar).toContainText('18 / 40 files')
    await ipcRelease(page)
    await expect(bar).toHaveCount(0)
  })

  test('a failed undo is shown plainly and dismissable', async ({ page }) => {
    await history(page, { repo_reset: { __error: { code: 'error', message: 'Cannot move the branch: it is protected.' } } })
    await choose(page, 'Move branch back here, keep changes ready to save')
    await page.getByRole('alertdialog').getByRole('button', { name: 'Move branch back' }).click()
    const alert = detail(page).getByRole('alert')
    await expect(alert).toContainText('Cannot move the branch: it is protected.')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
  })

  test('the menu button is keyboard reachable with a visible focus ring', async ({ page }) => {
    await history(page)
    await undoMenu(page).focus()
    await page.keyboard.press('Shift+Tab')
    await page.keyboard.press('Tab')
    await expect(undoMenu(page)).toBeFocused()
    await expectFocusRing(page)
    await page.keyboard.press('Enter')
    await expect(page.getByRole('menuitem').first()).toBeVisible()
  })
})

test.describe('Undo: restore one file', () => {
  async function pickFile(page: Page) {
    await history(page)
    await page.getByRole('tab', { name: 'Files at this version' }).click()
    await page.getByRole('tree', { name: 'Files at this version' }).getByRole('treeitem', { name: /hero_preview/ }).click()
    await expect(page.getByTestId('picked-file')).toContainText('art/hero_preview.png')
  }

  test('restoring replaces the working file after a warning', async ({ page }) => {
    await pickFile(page)
    await detail(page).getByRole('button', { name: 'Restore this file…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Restore art/hero_preview.png?')
    await expect(dlg).toContainText('Unsaved changes to this file are lost.')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'repo_restore_file')).toHaveLength(0)
    await detail(page).getByRole('button', { name: 'Restore this file…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Restore file' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_restore_file')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', rev: SECOND, path: 'art/hero_preview.png' },
    ])
    await expect(page.getByText('Restored from 0001000').first()).toBeVisible()
  })

  test('no file picked: no restore row', async ({ page }) => {
    await history(page)
    await expect(page.getByTestId('picked-file')).toHaveCount(0)
  })
})

test.describe('Undo: Changes view', () => {
  test('Discard all unsaved changes goes through the hard-reset dialog against HEAD', async ({ page }) => {
    await boot(page, '/changes', 'Changes')
    await page.getByRole('button', { name: 'More actions' }).click()
    await expectNoAxeViolations(page)
    await page.getByRole('menuitem', { name: 'Discard all unsaved changes…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Discard everything back to the latest version?' })
    await expect(dlg).toContainText('6 files with unsaved changes will be lost.')
    await expect(dlg).not.toContainText('Versions after it leave this branch.')
    await expectNoAxeViolations(page)
    await dlg.getByRole('checkbox', { name: 'I understand my unsaved work will be lost' }).click()
    await dlg.getByRole('button', { name: 'Discard and go back' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_reset_hard')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', rev: 'HEAD' }])
  })

  test('with nothing changed the menu is disabled', async ({ page }) => {
    await boot(page, '/changes', 'Changes', { repo_status: fixture('status_empty') })
    await expect(page.getByRole('button', { name: 'More actions' })).toBeDisabled()
  })
})
