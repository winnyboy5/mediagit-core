import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const row = (page: Page, path: string) => page.locator(`[data-file="${path}"]`)

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({ branch_list: fixture('branches'), repo_merge: fixture('merge_conflict'), repo_merge_control: fixture('merge_done'), ...commands }),
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto('/#/merge', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Merge & conflicts', level: 1 })).toBeVisible()
}

/** Open the dialog and start merging feature/grade (the first other branch). */
async function startMerge(page: Page) {
  await page.getByRole('button', { name: 'Start a merge' }).first().click()
  const dlg = page.getByRole('dialog', { name: 'Start a merge' })
  await dlg.getByRole('button', { name: 'Start merge' }).click()
  return dlg
}

test.describe('Merge: states', () => {
  test('empty: nothing in progress, start offered; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(page.getByText('No merge in progress')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Start a merge' }).first()).toBeEnabled()
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton until branches arrive', async ({ page }) => {
    await boot(page, {}, { branch_list: [{ hold: true }, { resolve: fixture('branches') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Start a merge' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(page.getByText('No merge in progress')).toBeVisible()
  })

  test('only one branch: nothing to merge, start is disabled with a reason', async ({ page }) => {
    await boot(page, { branch_list: { current: 'main', branches: [fixture<{ branches: unknown[] }>('branches').branches[0]] } })
    await expect(page.getByText('There is no other branch to merge yet.')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Start a merge' }).first()).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('error: branches cannot be read, Retry asks again', async ({ page }) => {
    await boot(page, { branch_list: { __error: { code: 'error', message: 'Cannot read branches.' } } })
    await expect(page.getByRole('alert')).toContainText('Cannot read branches')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'branch_list')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'branch_list')).length).toBeGreaterThan(before)
  })

  test('dialog: choose branch, strategy and merge style; no axe violations', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Start a merge' }).first().click()
    const dlg = page.getByRole('dialog', { name: 'Start a merge' })
    await expect(dlg.getByLabel('Merge this branch')).toContainText('feature/grade')
    await expect(dlg.getByLabel('When files clash')).toContainText('Ask me about each conflicting file')
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Merge this branch').click()
    await page.getByRole('option', { name: 'review/cut-2' }).click()
    await dlg.getByLabel('When files clash').click()
    await page.getByRole('option', { name: 'Keep their version of every conflict' }).click()
    await dlg.getByRole('checkbox', { name: 'Always create a merge version' }).click()
    await dlg.getByRole('button', { name: 'Start merge' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_merge')).map((c) => c.args)).toMatchObject([
      { repo: '/repos/hero-film', branch: 'review/cut-2', strategy: 'theirs', noFf: true },
    ])
  })

  test('a failed start shows the reason on the page and can be dismissed', async ({ page }) => {
    await boot(page, { repo_merge: { __error: { code: 'error', message: 'Cannot merge: unsaved changes would be overwritten.' } } })
    await startMerge(page)
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('unsaved changes would be overwritten')
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
  })
})

test.describe('Merge: conflicts', () => {
  test('conflict list: reasons, summary, Finish disabled until every file is decided', async ({ page }) => {
    await boot(page)
    await startMerge(page)
    await expect(page.getByTestId('merge-summary')).toContainText('Merging feature/grade into main')
    await expect(page.getByTestId('merge-summary')).toContainText('4 of 4 files need a decision · 0 resolved')
    await expect(page.getByRole('list', { name: 'Conflicting files' }).getByRole('listitem')).toHaveCount(4)
    await expect(row(page, 'art/hero.psd')).toContainText('Both changed')
    await expect(row(page, 'char/rig.fbx')).toContainText('Changed by you, deleted by them')
    await expect(row(page, 'audio/vo_take3.wav')).toContainText('Deleted by you, changed by them')
    await expect(page.getByRole('button', { name: 'Finish merge' })).toBeDisabled()
    await expect(page.getByRole('progressbar', { name: 'Conflicts resolved' })).toHaveAttribute('aria-valuenow', '0')
    await expectNoAxeViolations(page)
  })

  test('a side that does not exist cannot be kept', async ({ page }) => {
    await boot(page)
    await startMerge(page)
    await expect(row(page, 'char/rig.fbx').getByRole('button', { name: 'Keep theirs' })).toBeDisabled()
    await expect(row(page, 'char/rig.fbx').getByRole('button', { name: 'Keep mine' })).toBeEnabled()
    await expect(row(page, 'audio/vo_take3.wav').getByRole('button', { name: 'Keep mine' })).toBeDisabled()
    await expect(row(page, 'audio/vo_take3.wav').getByRole('button', { name: 'Keep theirs' })).toBeEnabled()
  })

  test('resolving every file enables Finish; Finish merges and returns to the empty state', async ({ page }) => {
    await boot(page)
    await startMerge(page)
    await row(page, 'art/hero.psd').getByRole('button', { name: 'Keep mine' }).click()
    await expect(row(page, 'art/hero.psd')).toContainText('Kept mine')
    await expect(row(page, 'art/hero.psd').getByRole('button', { name: 'Keep mine' })).toHaveAttribute('aria-pressed', 'true')
    await expect(page.getByTestId('merge-summary')).toContainText('3 of 4 files need a decision · 1 resolved')
    await row(page, 'scenes/opening.blend').getByRole('button', { name: 'Keep theirs' }).click()
    await row(page, 'char/rig.fbx').getByRole('button', { name: 'Keep mine' }).click()
    await expect(page.getByRole('button', { name: 'Finish merge' })).toBeDisabled()
    await row(page, 'audio/vo_take3.wav').getByRole('button', { name: 'Keep theirs' }).click()
    await expect(row(page, 'audio/vo_take3.wav')).toContainText('Kept theirs')
    await expect(page.getByRole('button', { name: 'Finish merge' })).toBeEnabled()
    expect((await ipcCalls(page, 'merge_resolve')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', path: 'art/hero.psd', side: 'ours', theirs: '0002000200020002000200020002000200020002' },
      { repo: '/repos/hero-film', path: 'scenes/opening.blend', side: 'theirs', theirs: '0002000200020002000200020002000200020002' },
      { repo: '/repos/hero-film', path: 'char/rig.fbx', side: 'ours', theirs: '0002000200020002000200020002000200020002' },
      { repo: '/repos/hero-film', path: 'audio/vo_take3.wav', side: 'theirs', theirs: '0002000200020002000200020002000200020002' },
    ])
    await page.getByRole('button', { name: 'Finish merge' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_merge_control')).map((c) => c.args.action)).toEqual(['continue'])
    await expect(page.getByTestId('merge-result')).toContainText('Done: merged')
    await expect(page.getByText('No merge in progress')).toBeVisible()
  })

  test('a failed resolve is shown plainly and the file stays undecided', async ({ page }) => {
    await boot(page, { merge_resolve: { __error: { code: 'error', message: 'Could not write art/hero.psd.' } } })
    await startMerge(page)
    await row(page, 'art/hero.psd').getByRole('button', { name: 'Keep mine' }).click()
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('Could not write art/hero.psd.')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
    await expect(row(page, 'art/hero.psd')).not.toContainText('Kept mine')
  })

  test('cancel merge asks first, explains, then aborts', async ({ page }) => {
    await boot(page, { repo_merge_control: fixture('merge_aborted') })
    await startMerge(page)
    await page.getByRole('button', { name: 'Cancel merge' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Cancel this merge?')
    await expect(dlg).toContainText('go back to how they were')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel', exact: true }).click()
    expect(await ipcCalls(page, 'repo_merge_control')).toHaveLength(0)
    await page.getByRole('button', { name: 'Cancel merge' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel merge' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_merge_control')).map((c) => c.args.action)).toEqual(['abort'])
    await expect(page.getByTestId('merge-result')).toContainText('Merge cancelled')
  })

  test('busy: a merge shows a determinate files bar, then finishes', async ({ page }) => {
    await boot(page, {}, { repo_merge: fixture('merge_progress') })
    await startMerge(page)
    const bar = page.getByTestId('merge-progress')
    await expect(bar).toContainText('Merge feature/grade')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '40')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '18')
    await ipcRelease(page)
    await expect(page.getByTestId('merge-result')).toContainText('Done: merged')
  })

  test('a merge started elsewhere: Finish is offered on the empty state', async ({ page }) => {
    await boot(page)
    await page.getByText('Merge started in another tool?').click()
    await page.getByRole('button', { name: 'Finish merge' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_merge_control')).map((c) => c.args.action)).toEqual(['continue'])
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await startMerge(page)
    await row(page, 'art/hero.psd').getByRole('button', { name: 'Keep mine' }).focus()
    // The mouse was used to open the dialog; switch the modality to keyboard first.
    await page.keyboard.press('Shift+Tab')
    await page.keyboard.press('Tab')
    await expect(row(page, 'art/hero.psd').getByRole('button', { name: 'Keep mine' })).toBeFocused()
    await expectFocusRing(page)
    await page.keyboard.press('Tab')
    await expect(row(page, 'art/hero.psd').getByRole('button', { name: 'Keep theirs' })).toBeFocused()
    await expectFocusRing(page)
  })
})
