import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const C = '0004000400040004000400040004000400040004'
const OLDEST = fixture<{ commits: { oid: string }[] }>('log_graph').commits.at(-1)!.oid
// Copying starts with no version chosen (an empty copy is refused), so tests choose one.
async function chooseVersion(page: Page, name: string | RegExp = /Merge grade pass into main/) {
  await page.getByLabel('Copy this version').click()
  await page.getByRole('option', { name }).click()
}
const file = (page: Page, p: string) => page.locator(`[data-file="${p}"]`)

async function boot(page: Page, tab: string, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}, handlers: Record<string, string> = {}) {
  await mockIpc(page, {
    commands: healthy({ repo_log: fixture('log_graph'), branch_list: fixture('branches'), reflog_list: fixture('reflog'), ...commands }),
    scripts: scripts as never,
    handlers,
    storage: REPO,
  })
  await page.goto('/#/advanced', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Advanced', level: 2 })).toBeVisible()
  if (tab !== 'Rebase') await page.getByRole('tab', { name: tab }).click()
}

test.describe('Advanced: rebase', () => {
  test('idle: choose a branch, confirm, then it runs; no axe violations', async ({ page }) => {
    await boot(page, 'Rebase', { rebase_start: fixture('rebase_done') })
    await expect(page.getByLabel('Rebase onto')).toContainText('feature/grade')
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Rebase onto feature/grade?')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'rebase_start')).toHaveLength(0)
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
    await expect(page.getByTestId('rebase-result')).toContainText('Rebased.')
    expect((await ipcCalls(page, 'rebase_start')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', upstream: 'feature/grade' }])
  })

  test('only one branch: nothing to rebase onto', async ({ page }) => {
    await boot(page, 'Rebase', { branch_list: { current: 'main', branches: [fixture<{ branches: unknown[] }>('branches').branches[0]] } })
    await expect(page.getByText('There is no other branch to rebase onto.')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Start rebase…' })).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('busy: a replaying bar with a count, then done', async ({ page }) => {
    await boot(page, 'Rebase', {}, { rebase_start: fixture('seq_progress') })
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
    const bar = page.getByTestId('rebase-progress')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '3')
    await expect(bar).toContainText('0 / 3')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bar).toContainText('1 / 3')
    await ipcRelease(page)
    await expect(page.getByTestId('rebase-result')).toContainText('Rebased.')
  })

  test('a clash lists the files; each is decided with the replayed version as "theirs"; continue finishes', async ({ page }) => {
    await boot(page, 'Rebase', { rebase_start: fixture('rebase_conflict'), sequencer_control: fixture('rebase_done') })
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
    const sum = page.getByTestId('rebase-summary')
    await expect(sum).toContainText('2 of 2 files need a decision · 0 resolved')
    await expect(page.getByRole('list', { name: 'Clashing files' }).getByRole('listitem')).toHaveCount(2)
    await expect(page.getByRole('button', { name: 'Continue rebase' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await file(page, 'art/hero.psd').getByRole('button', { name: 'Keep mine' }).click()
    await file(page, 'scenes/opening.blend').getByRole('button', { name: 'Keep theirs' }).click()
    await expect(sum).toContainText('0 of 2 files need a decision · 2 resolved')
    expect((await ipcCalls(page, 'merge_resolve')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', path: 'art/hero.psd', side: 'ours', theirs: C },
      { repo: '/repos/hero-film', path: 'scenes/opening.blend', side: 'theirs', theirs: C },
    ])
    await page.getByRole('button', { name: 'Continue rebase' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sequencer_control')).map((c) => [c.args.command, c.args.action])).toEqual([['rebase', 'continue']])
    await expect(page.getByTestId('rebase-result')).toContainText('Rebased.')
  })

  test('skip and cancel go through their own commands; cancel asks first', async ({ page }) => {
    await boot(page, 'Rebase', { rebase_start: fixture('rebase_conflict'), sequencer_control: fixture('rebase_aborted') })
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
    await page.getByRole('button', { name: 'Cancel rebase' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Everything done so far is undone')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel', exact: true }).click()
    expect(await ipcCalls(page, 'sequencer_control')).toHaveLength(0)
    await page.getByRole('button', { name: 'Cancel rebase' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel rebase' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sequencer_control')).map((c) => c.args.action)).toEqual(['abort'])
    await expect(page.getByTestId('rebase-result')).toContainText('Cancelled.')
  })

  test('skip this version', async ({ page }) => {
    await boot(page, 'Rebase', { rebase_start: fixture('rebase_conflict'), sequencer_control: fixture('rebase_done') })
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
    await page.getByRole('button', { name: 'Skip this version' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sequencer_control')).map((c) => c.args.action)).toEqual(['skip'])
  })

  test('a failed start is shown plainly and dismissable', async ({ page }) => {
    await boot(page, 'Rebase', { rebase_start: { __error: { code: 'error', message: 'You have unsaved changes.' } } })
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('You have unsaved changes.')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
  })

  test('a rebase begun elsewhere can be continued from the idle state', async ({ page }) => {
    await boot(page, 'Rebase', { sequencer_control: fixture('rebase_done') })
    await page.getByRole('button', { name: 'Continue rebase' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sequencer_control')).map((c) => c.args.action)).toEqual(['continue'])
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page, 'Rebase')
    await page.getByRole('button', { name: 'Start rebase…' }).focus()
    await expectFocusRing(page)
  })
})

test.describe('Advanced: cherry-pick', () => {
  test('copies the chosen version after a confirm', async ({ page }) => {
    await boot(page, 'Cherry-pick', { pick_start: fixture('pick_done') })
    await expect(page.getByRole('button', { name: 'Copy version' })).toBeDisabled()
    await chooseVersion(page)
    await expect(page.getByLabel('Copy this version')).toContainText('Merge grade pass into main')
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'Copy version' }).click()
    await expect(page.getByRole('alertdialog')).toContainText('Copy this version?')
    await page.getByRole('alertdialog').getByRole('button', { name: 'Copy version' }).click()
    await expect(page.getByTestId('pick-result')).toContainText('Copied.')
    expect((await ipcCalls(page, 'pick_start')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', commits: ['0000000000000000000000000000000000000000'] }])
  })

  test('versions can come from another branch: pick the branch, then one of its versions', async ({ page }) => {
    await boot(page, 'Cherry-pick', { pick_start: fixture('pick_done') })
    await expect(page.getByLabel('From branch')).toContainText('All branches')
    await page.getByLabel('From branch').click()
    await page.getByRole('option', { name: 'review/cut-2' }).click()
    // Only what review/cut-2 reaches is offered; the merge on main is not.
    await page.getByLabel('Copy this version').click()
    await expect(page.getByRole('option', { name: /Grade pass: warmer shadows/ })).toBeVisible()
    await expect(page.getByRole('option', { name: /Merge grade pass into main/ })).toHaveCount(0)
    await page.getByRole('option', { name: /Grade pass: warmer shadows/ }).click()
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'Copy version' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Copy version' }).click()
    await expect(page.getByTestId('pick-result')).toBeVisible()
    expect((await ipcCalls(page, 'pick_start')).map((c) => c.args)).toMatchObject([{ commits: [C] }])
  })

  test('a clash is decided per file, then continued with the cherry-pick command', async ({ page }) => {
    await boot(page, 'Cherry-pick', { pick_start: fixture('pick_conflict'), sequencer_control: fixture('pick_done') })
    await chooseVersion(page)
    await page.getByRole('button', { name: 'Copy version' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Copy version' }).click()
    await expect(page.getByTestId('pick-summary')).toContainText('1 of 1 files need a decision')
    await file(page, 'audio/vo_take3.wav').getByRole('button', { name: 'Keep theirs' }).click()
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'Continue copy' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sequencer_control')).map((c) => [c.args.command, c.args.action])).toEqual([['cherry-pick', 'continue']])
  })

  test('busy: a bar while it copies', async ({ page }) => {
    await boot(page, 'Cherry-pick', {}, { pick_start: fixture('seq_progress') })
    await chooseVersion(page)
    await page.getByRole('button', { name: 'Copy version' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Copy version' }).click()
    await expect(page.getByTestId('pick-progress').getByRole('progressbar')).toHaveAttribute('aria-valuemax', '3')
    await ipcRelease(page)
    await ipcRelease(page)
    await expect(page.getByTestId('pick-result')).toBeVisible()
  })
})

test.describe('Advanced: bisect', () => {
  const mark = `(a) => a.verdict === 'bad' ? ${JSON.stringify(fixture('bisect_complete'))} : ${JSON.stringify(fixture('bisect_testing2'))}`

  test('idle: start with the oldest version as "good"; no axe violations', async ({ page }) => {
    await boot(page, 'Bisect', { bisect_start: fixture('bisect_testing') })
    await expect(page.getByLabel('A version that was working')).toContainText(OLDEST.slice(0, 7))
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'Start bisect' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'bisect_start')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', bad: 'HEAD', good: OLDEST }])
  })

  test('testing: the candidate, steps left, and three verdicts', async ({ page }) => {
    await boot(page, 'Bisect', { bisect_start: fixture('bisect_testing') })
    await page.getByRole('button', { name: 'Start bisect' }).click()
    const st = page.getByTestId('bisect-state')
    await expect(st).toContainText('Test this version: 0003000')
    await expect(st).toContainText('About 3 steps left.')
    await expect(st.getByRole('group', { name: 'Does it work?' }).getByRole('button')).toHaveText(['It works (good)', 'It is broken (bad)', 'Cannot tell (skip)'])
    await expectNoAxeViolations(page)
  })

  test('verdicts narrow it down until the first bad version is named', async ({ page }) => {
    await boot(page, 'Bisect', { bisect_start: fixture('bisect_testing') }, {}, { bisect_mark: mark })
    await page.getByRole('button', { name: 'Start bisect' }).click()
    await page.getByRole('button', { name: 'It works (good)' }).click()
    const st = page.getByTestId('bisect-state')
    await expect(st).toContainText('About 1 step left.')
    await expect(st).toContainText('Skipped')
    await page.getByRole('button', { name: 'It is broken (bad)' }).click()
    await expect(st).toContainText('The first bad version is 0002000')
    await expect(page.getByRole('button', { name: 'It works (good)' })).toHaveCount(0)
    await expectNoAxeViolations(page)
    expect((await ipcCalls(page, 'bisect_mark')).map((c) => [c.args.verdict, c.args.commit])).toEqual([['good', null], ['bad', null]])
  })

  test('waiting for a good version says what to do next', async ({ page }) => {
    await boot(page, 'Bisect', { bisect_start: fixture('bisect_waiting') })
    await page.getByRole('button', { name: 'Start bisect' }).click()
    await expect(page.getByTestId('bisect-state')).toContainText('Now choose a version that worked.')
    await expectNoAxeViolations(page)
  })

  test('ending asks first, then puts files back', async ({ page }) => {
    await boot(page, 'Bisect', { bisect_start: fixture('bisect_testing'), bisect_reset: fixture('bisect_reset') })
    await page.getByRole('button', { name: 'Start bisect' }).click()
    await page.getByRole('button', { name: 'End bisect…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('End the bisect?')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'bisect_reset')).toHaveLength(0)
    await page.getByRole('button', { name: 'End bisect…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'End bisect' }).click()
    await expect(page.getByText('Bisect ended. Your files are back where they were.')).toBeVisible()
  })

  test('busy: starting shows a bar while it checks a version out', async ({ page }) => {
    await boot(page, 'Bisect', {}, { bisect_start: fixture('seq_progress') })
    await page.getByRole('button', { name: 'Start bisect' }).click()
    await expect(page.getByTestId('bisect-progress').getByRole('progressbar')).toHaveAttribute('aria-valuemax', '3')
    await expectNoAxeViolations(page)
  })

  test('a failed step is shown plainly', async ({ page }) => {
    await boot(page, 'Bisect', { bisect_start: { __error: { code: 'error', message: 'That version is not an ancestor of the bad one.' } } })
    await page.getByRole('button', { name: 'Start bisect' }).click()
    await expect(page.getByRole('alert')).toContainText('not an ancestor')
    await expectNoAxeViolations(page)
  })
})

test.describe('Advanced: reflog', () => {
  test('populated: newest first, the current position cannot be restored; no axe violations', async ({ page }) => {
    await boot(page, 'Reflog')
    const rows = page.getByRole('list', { name: 'Reflog, newest first' }).getByRole('listitem')
    await expect(rows).toHaveCount(3)
    await expect(rows.first()).toContainText('merge feature/grade: Fast-forward')
    await expect(page.getByRole('button', { name: 'Restore to @0' })).toBeDisabled()
    await expect(page.getByRole('button', { name: 'Restore to @1' })).toBeEnabled()
    expect((await ipcCalls(page, 'reflog_list'))[0]!.args).toEqual({ repo: '/repos/hero-film', reference: 'HEAD', count: 100 })
    await expectNoAxeViolations(page)
  })

  test('loading, empty and error states', async ({ page }) => {
    await boot(page, 'Reflog', {}, { reflog_list: [{ hold: true }, { resolve: fixture('reflog') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(page.getByRole('button', { name: 'Restore to @1' })).toBeVisible()
  })

  test('empty', async ({ page }) => {
    await boot(page, 'Reflog', { reflog_list: fixture('reflog_empty') })
    await expect(page.getByText('No history for this ref')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('error: Retry asks again', async ({ page }) => {
    await boot(page, 'Reflog', { reflog_list: { __error: { code: 'error', message: 'Cannot read the reflog.' } } })
    await expect(page.getByRole('alert')).toContainText('Cannot read the reflog')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'reflog_list')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'reflog_list')).length).toBeGreaterThan(before)
  })

  test('choosing another ref lists it', async ({ page }) => {
    await boot(page, 'Reflog')
    await page.getByLabel('Look at').click()
    await page.getByRole('option', { name: 'feature/grade' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'reflog_list')).at(-1)?.args.reference).toBe('feature/grade')
  })

  test('restore to here goes through the soft-reset dialog and then reloads the list', async ({ page }) => {
    await boot(page, 'Reflog', { repo_reset: fixture('reset_ok') })
    await page.getByRole('button', { name: 'Restore to @1' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Move this branch back to 0001000?')
    await expect(dlg).toContainText('your files are not touched')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'repo_reset')).toHaveLength(0)
    const before = (await ipcCalls(page, 'reflog_list')).length
    await page.getByRole('button', { name: 'Restore to @1' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Move branch back' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_reset')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', rev: '0001'.repeat(10), mode: 'soft' }])
    await expect.poll(async () => (await ipcCalls(page, 'reflog_list')).length).toBeGreaterThan(before)
  })
})
