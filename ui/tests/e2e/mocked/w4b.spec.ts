import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, mockIpc, RECENT, showHandler } from './ipc-mock'

// Every option of the History, Branches, Merge, Compare and Advanced views reaches its
// command with the right arguments; risky ones ask first and have a safe default.
const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const OID = '0001000100010001000100010001000100010001'

const REMOTE = { current: null, branches: [{ name: 'origin/feat-a', oid: OID, remote: true, upstream: null, ahead: null, behind: null, protected: false }] }
const branchList = `(a) => a.remote ? ${JSON.stringify(REMOTE)} : ${JSON.stringify(fixture('branches'))}`

async function boot(page: Page, route: string, heading: string, commands: Record<string, unknown> = {}, handlers: Record<string, string> = {}) {
  await mockIpc(page, {
    commands: healthy({
      repo_log: fixture('log_graph'),
      branch_list: fixture('branches'),
      reflog_list: fixture('reflog'),
      branch_create: {},
      branch_switch: fixture('branch_switch_progress'),
      branch_delete: {},
      branch_rename: {},
      branch_protect: { name: 'main', protected: true, changed: true, require_reviews: false },
      branch_show: { name: 'feature/grade', oid: OID, remote: false, upstream: null, ahead: null, behind: null, protected: false },
      repo_merge: { action: 'merge', status: 'merged', oid: OID, conflicts: [] },
      repo_revert: fixture('revert_ok'),
      repo_diff: fixture('diff_empty'),
      ...commands,
    }),
    handlers: { repo_show: showHandler(), ...handlers },
    storage: REPO,
  })
  await page.goto(`/#${route}`, { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
}
const args = async (page: Page, cmd: string) => (await ipcCalls(page, cmd)).map((c) => c.args)
const row = (page: Page, name: string) => page.locator(`[data-branch="${name}"]`)
const more = (scope: ReturnType<Page['locator']>) => scope.getByText('More options').click()

test.describe('Branches: options', () => {
  test('new branch: start point, upstream and tracking reach the command', async ({ page }) => {
    await boot(page, '/branches', 'Branches')
    await page.getByRole('button', { name: 'New branch' }).click()
    const dlg = page.getByRole('dialog', { name: 'New branch' })
    await dlg.getByLabel('Branch name').fill('hotfix')
    await more(dlg)
    await dlg.getByLabel('Start from').fill('main')
    await dlg.getByLabel('Follow this server branch').fill('origin/main')
    await dlg.getByLabel('Following').click()
    await page.getByRole('option', { name: /Do not follow/ }).click()
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Create', exact: true }).click()
    await expect.poll(() => args(page, 'branch_create')).toMatchObject([{ name: 'hotfix', opts: { startPoint: 'main', setUpstream: 'origin/main', tracking: 'none' } }])
  })

  test('new branch: a bad upstream is explained and blocks Create', async ({ page }) => {
    await boot(page, '/branches', 'Branches')
    await page.getByRole('button', { name: 'New branch' }).click()
    const dlg = page.getByRole('dialog', { name: 'New branch' })
    await dlg.getByLabel('Branch name').fill('hotfix')
    await more(dlg)
    await dlg.getByLabel('Follow this server branch').fill('main')
    await expect(dlg).toContainText('Write it as remote/branch')
    await expect(dlg.getByRole('button', { name: 'Create', exact: true })).toBeDisabled()
  })

  test('server branches: start working on one (create + track), forget one after asking', async ({ page }) => {
    await boot(page, '/branches', 'Branches', {}, { branch_list: branchList })
    const r = page.locator('[data-remote="origin/feat-a"]')
    await expect(r).toBeVisible()
    await expectNoAxeViolations(page)
    await r.getByRole('button', { name: 'Start working on it' }).click()
    await expect.poll(() => args(page, 'branch_switch')).toMatchObject([{ name: 'origin/feat-a', opts: { create: true, track: true } }])
    await r.getByRole('button', { name: /Forget origin\/feat-a/ }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Only the copy on this computer is removed')
    await dlg.getByRole('button', { name: 'Forget branch' }).click()
    await expect.poll(() => args(page, 'branch_delete')).toMatchObject([{ name: 'origin/feat-a', opts: { remote: true } }])
  })

  test('delete is "only if merged" by default; deleting anyway is a danger option', async ({ page }) => {
    await boot(page, '/branches', 'Branches')
    await row(page, 'feature/grade').getByRole('button', { name: 'Delete feature/grade' }).click()
    let dlg = page.getByRole('dialog', { name: 'Delete feature/grade?' })
    await expect(dlg).toContainText('only deleted if its work is already part of another branch')
    await dlg.getByRole('button', { name: 'Delete branch' }).click()
    await expect.poll(() => args(page, 'branch_delete')).toMatchObject([{ name: 'feature/grade', opts: { mergedOnly: true } }])
    expect((await args(page, 'branch_delete'))[0]!.opts).not.toHaveProperty('force')

    await row(page, 'review/cut-2').getByRole('button', { name: 'Delete review/cut-2' }).click()
    dlg = page.getByRole('dialog', { name: 'Delete review/cut-2?' })
    await more(dlg)
    await dlg.getByRole('checkbox', { name: /Delete even if its work is not merged/ }).click()
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Delete branch' }).click()
    await expect.poll(async () => (await args(page, 'branch_delete')).length).toBe(2)
    expect((await args(page, 'branch_delete'))[1]).toMatchObject({ opts: { force: true, confirmed: true } })
  })

  test('rename over an existing branch needs the option and a confirmation', async ({ page }) => {
    await boot(page, '/branches', 'Branches')
    await row(page, 'feature/grade').getByRole('button', { name: 'Rename feature/grade' }).click()
    const dlg = page.getByRole('dialog', { name: 'Rename feature/grade' })
    await dlg.getByLabel('New name').fill('review/cut-2')
    await expect(dlg).toContainText('A branch with this name exists')
    await expect(dlg.getByRole('button', { name: 'Rename', exact: true })).toBeDisabled()
    await more(dlg)
    await dlg.getByRole('checkbox', { name: /Replace the branch that already has this name/ }).click()
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    const ask = page.getByRole('alertdialog')
    await expect(ask).toContainText('review/cut-2 already exists')
    expect(await ipcCalls(page, 'branch_rename')).toHaveLength(0)
    await ask.getByRole('button', { name: 'Replace branch' }).click()
    await expect.poll(() => args(page, 'branch_rename')).toMatchObject([{ from: 'feature/grade', to: 'review/cut-2', force: true, confirmed: true }])
  })

  test('protect with reviews, then remove protection after asking; details dialog', async ({ page }) => {
    await boot(page, '/branches', 'Branches')
    await row(page, 'feature/grade').getByRole('button', { name: 'Protect feature/grade' }).click()
    const dlg = page.getByRole('dialog', { name: 'Protect feature/grade' })
    await dlg.getByRole('checkbox', { name: 'Ask for a review first' }).click()
    await dlg.getByRole('button', { name: 'Protect branch' }).click()
    await expect.poll(() => args(page, 'branch_protect')).toMatchObject([{ name: 'feature/grade', requireReviews: true, unprotect: false }])

    await row(page, 'main').getByRole('button', { name: 'Remove protection from main' }).click()
    const ask = page.getByRole('alertdialog')
    await expect(ask).toContainText('Anyone can then delete this branch')
    expect(await ipcCalls(page, 'branch_protect')).toHaveLength(1)
    await ask.getByRole('button', { name: 'Remove protection' }).click()
    await expect.poll(async () => (await args(page, 'branch_protect'))[1]).toMatchObject({ name: 'main', unprotect: true, confirmed: true })

    await row(page, 'feature/grade').getByRole('button', { name: 'Details of feature/grade' }).click()
    await expect(page.getByTestId('branch-detail')).toContainText('Latest version')
    expect(await args(page, 'branch_show')).toMatchObject([{ name: 'feature/grade' }])
    await expectNoAxeViolations(page)
  })

  test('a switch blocked by unsaved changes can be forced only after a warning', async ({ page }) => {
    await boot(page, '/branches', 'Branches', {}, { branch_switch: `() => { throw { code: 'error', message: 'Unsaved changes would be overwritten.' } }` })
    await row(page, 'feature/grade').getByRole('button', { name: 'Switch to', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('Unsaved changes would be overwritten')
    await page.getByRole('button', { name: 'Switch anyway…' }).click()
    const ask = page.getByRole('alertdialog')
    await expect(ask).toContainText('thrown away and cannot be brought back')
    expect(await ipcCalls(page, 'branch_switch')).toHaveLength(1)
    await ask.getByRole('button', { name: 'Discard changes and switch' }).click()
    await expect.poll(async () => (await args(page, 'branch_switch')).at(-1)).toMatchObject({ name: 'feature/grade', opts: { force: true, confirmed: true } })
  })
})

test.describe('Merge: options', () => {
  test('message, squash and no-commit reach the command; fast-forward-only excludes the others', async ({ page }) => {
    await boot(page, '/merge', 'Merge & conflicts')
    await page.getByRole('button', { name: 'Start a merge' }).first().click()
    const dlg = page.getByRole('dialog', { name: 'Start a merge' })
    await more(dlg)
    await dlg.getByLabel('Message for the merge version').fill('Bring in grade')
    await dlg.getByRole('checkbox', { name: 'Only if it can simply catch up' }).click()
    await expect(dlg.getByRole('checkbox', { name: 'Combine into one change' })).toBeDisabled()
    await expect(dlg.getByRole('checkbox', { name: 'Always create a merge version' })).toBeDisabled()
    await dlg.getByRole('checkbox', { name: 'Only if it can simply catch up' }).click()
    await dlg.getByRole('checkbox', { name: 'Combine into one change' }).click()
    await dlg.getByRole('checkbox', { name: 'Let me look before it is saved' }).click()
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Start merge' }).click()
    await expect.poll(() => args(page, 'repo_merge')).toMatchObject([
      { opts: { noFf: false, ffOnly: false, squash: true, noCommit: true, message: 'Bring in grade' } },
    ])
  })
})

test.describe('History: filters and undo options', () => {
  test('the filter form is read by the CLI, not the browser; clearing restores everything', async ({ page }) => {
    await boot(page, '/history', 'History')
    await page.getByRole('button', { name: 'Filters' }).click()
    const form = page.getByRole('form', { name: 'Filter versions' })
    await form.getByLabel('Saved by').fill('mira')
    await form.getByLabel('Message contains').fill('grade')
    await form.getByLabel('From date').fill('2026-01-01')
    await form.getByLabel('Until date').fill('2026-12-31')
    await form.getByLabel('Only these files or folders').fill('art/hero.psd, video')
    await form.getByLabel('Start from').click()
    await page.getByRole('option', { name: 'Branch main' }).click()
    await expectNoAxeViolations(page)
    await form.getByRole('button', { name: 'Apply filters' }).click()
    await expect
      .poll(async () => (await args(page, 'repo_log')).at(-1))
      .toMatchObject({ all: false, filter: { revision: 'main', author: 'mira', grep: 'grade', since: '2026-01-01', until: '2026-12-31', paths: ['art/hero.psd', 'video'] } })
    await expect(page.getByText('The graph is hidden while you search or filter.')).toBeVisible()
    await form.getByRole('button', { name: 'Clear' }).click()
    await expect.poll(async () => (await args(page, 'repo_log')).at(-1)).toMatchObject({ all: true })
    expect((await args(page, 'repo_log')).at(-1)).not.toHaveProperty('filter')
  })

  test('undo: several versions, a message and no-commit; a stopped undo can continue or skip', async ({ page }) => {
    await boot(page, '/history', 'History')
    await page.getByTestId('commit-detail').getByRole('button', { name: 'Undo…' }).click()
    await page.getByRole('menuitem', { name: /Undo this version/ }).click()
    const dlg = page.getByRole('dialog')
    await more(dlg)
    await dlg.getByLabel('Message for the new version').fill('Undo grade')
    await dlg.getByRole('checkbox', { name: 'Let me look before it is saved' }).click()
    const first = dlg.getByRole('group', { name: /Also undo these versions/ }).getByRole('checkbox').first()
    await first.click()
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: /Undo 2 versions/ }).click()
    await expect.poll(async () => (await args(page, 'repo_revert')).at(0)).toMatchObject({ opts: { noCommit: true, message: 'Undo grade' } })
    const sent = (await args(page, 'repo_revert'))[0]!
    expect(sent.revs).toHaveLength(2)
  })

  test('a revert that stops on clashes offers continue, skip and abandon', async ({ page }) => {
    await boot(page, '/history', 'History', { repo_revert: { action: 'revert', status: 'conflict', commits: [], conflicts: ['a.psd'] }, repo_revert_control: fixture('revert_ok') })
    await page.getByTestId('commit-detail').getByRole('button', { name: 'Undo…' }).click()
    await page.getByRole('menuitem', { name: /Undo this version/ }).click()
    await page.getByRole('dialog').getByRole('button', { name: /Undo this version/ }).click()
    const banner = page.getByTestId('revert-stopped')
    await expect(banner.getByRole('button', { name: 'Continue undo' })).toBeVisible()
    await banner.getByRole('button', { name: 'Skip this version' }).click()
    await expect.poll(() => args(page, 'repo_revert_control')).toMatchObject([{ action: 'skip' }])
  })
})

test.describe('Compare: staged changes', () => {
  test('"Ready to save" asks for diff --cached', async ({ page }) => {
    await boot(page, '/compare', 'Compare', { repo_diff: fixture('diff_working') })
    await page.getByLabel('To').click()
    await page.getByRole('option', { name: 'Ready to save (staged)' }).click()
    await expect.poll(async () => (await args(page, 'repo_diff')).at(-1)).toMatchObject({ cached: true })
    await expect(page.getByLabel('From')).toContainText('Latest saved version')
    await expectNoAxeViolations(page)
  })
})

test.describe('Advanced: options', () => {
  const tab = (page: Page, name: string | RegExp) => page.getByRole('tab', { name }).click()

  test('rebase: branch and keep-empty; copy: no-commit and note the source', async ({ page }) => {
    await boot(page, '/advanced', 'Advanced', { rebase_start: fixture('rebase_done'), pick_start: fixture('pick_done') })
    const rb = page.locator('[data-panel="rebase"]')
    await more(rb)
    await rb.getByLabel('Branch to rebase').click()
    await page.getByRole('option', { name: 'review/cut-2' }).click()
    await rb.getByRole('checkbox', { name: /Keep versions that end up changing nothing/ }).click()
    await expectNoAxeViolations(page)
    await rb.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
    await expect.poll(() => args(page, 'rebase_start')).toMatchObject([{ branch: 'review/cut-2', keepEmpty: true }])

    await tab(page, /Copy a version/)
    const pk = page.locator('[data-panel="pick"]')
    await more(pk)
    await pk.getByRole('checkbox', { name: 'Let me look before it is saved' }).click()
    await pk.getByRole('checkbox', { name: 'Note where it was copied from' }).click()
    await pk.getByLabel('Copy this version').click()
    await page.getByRole('option', { name: /Merge grade pass into main/ }).click()
    await pk.getByRole('button', { name: 'Copy version' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Copy version' }).click()
    await expect.poll(() => args(page, 'pick_start')).toMatchObject([{ opts: { noCommit: true, appendMessage: true } }])
  })

  test('bisect: restarting a running one asks first; mark another version; move the branch asks first', async ({ page }) => {
    await boot(page, '/advanced', 'Advanced', { bisect_start: fixture('bisect_testing'), bisect_mark: fixture('bisect_testing2'), bisect_reset: fixture('bisect_reset') })
    await tab(page, /Find the break/)
    const bp = page.locator('[data-panel="bisect"]')
    await more(bp)
    await bp.getByRole('checkbox', { name: /Throw away a bisect that is already running/ }).click()
    await expectNoAxeViolations(page)
    await bp.getByRole('button', { name: 'Start bisect' }).click()
    const ask = page.getByRole('alertdialog')
    await expect(ask).toContainText('Everything you marked in it is forgotten')
    expect(await ipcCalls(page, 'bisect_start')).toHaveLength(0)
    await ask.getByRole('button', { name: 'Start a new bisect' }).click()
    await expect.poll(() => args(page, 'bisect_start')).toMatchObject([{ restart: true, confirmed: true }])

    const st = page.getByTestId('bisect-state')
    await more(st)
    await st.getByRole('button', { name: 'Mark it bad' }).click()
    await expect.poll(async () => (await args(page, 'bisect_mark')).at(0)).toMatchObject({ verdict: 'bad' })
    expect(typeof (await args(page, 'bisect_mark'))[0]!.commit).toBe('string')

    await st.getByRole('button', { name: /End bisect and move my branch/ }).click()
    const move = page.getByRole('alertdialog')
    await expect(move).toContainText('Versions after it leave the branch')
    expect(await ipcCalls(page, 'bisect_reset')).toHaveLength(0)
    await move.getByRole('button', { name: 'End and move branch' }).click()
    await expect.poll(() => args(page, 'bisect_reset')).toMatchObject([{ confirmed: true }])
  })

  test('undo log: every ref; trimming and forgetting are confirmed and name what is lost', async ({ page }) => {
    const all = { refs: [{ reference: 'main', entries: fixture<{ entries: unknown[] }>('reflog').entries.slice(0, 1) }] }
    await boot(page, '/advanced', 'Advanced', { reflog_all: all, reflog_expire: { action: 'expire', changed: true, expired: 4, refs: ['main'] }, reflog_delete: { action: 'delete', changed: true, expired: 0, refs: ['HEAD'] } })
    await tab(page, /Undo log/)
    const rp = page.locator('[data-panel="reflog"]')
    await rp.getByLabel('Look at').click()
    await page.getByRole('option', { name: /Every branch and tag/ }).click()
    await expect(page.getByTestId('reflog-all')).toContainText('main')
    expect(await args(page, 'reflog_all')).toMatchObject([{ count: 20 }])
    await rp.getByLabel('Look at').click()
    await page.getByRole('option', { name: /HEAD/ }).click()

    await rp.getByText('Clean up this log').click()
    await rp.getByLabel('Entries to keep').fill('10')
    await expectNoAxeViolations(page)
    await rp.getByRole('button', { name: 'Remove older entries…' }).click()
    const ask = page.getByRole('alertdialog')
    await expect(ask).toContainText('You will not be able to restore to those positions')
    expect(await ipcCalls(page, 'reflog_expire')).toHaveLength(0)
    await ask.getByRole('button', { name: 'Remove entries' }).click()
    await expect.poll(() => args(page, 'reflog_expire')).toMatchObject([{ reference: 'HEAD', keep: 10, confirmed: true }])

    await rp.getByRole('button', { name: /Forget all of HEAD/ }).click()
    await expect(page.getByRole('alertdialog')).toContainText('Its whole undo log is deleted')
    expect(await ipcCalls(page, 'reflog_delete')).toHaveLength(0)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Forget it all' }).click()
    await expect.poll(() => args(page, 'reflog_delete')).toMatchObject([{ reference: 'HEAD', confirmed: true }])
  })
})
