import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const stash = (page: Page, i: number) => page.locator(`[data-stash="${i}"]`)

/** The tag form sits behind a "New tag" disclosure; tag actions in one "Actions for tag NAME" menu. */
const newTag = (page: Page) => page.getByText('New tag', { exact: true }).click()
async function tagMenu(page: Page, name: string, item: string) {
  await page.locator(`[data-tag="${name}"]`).getByRole('button', { name: `Actions for tag ${name}` }).click()
  await page.getByRole('menuitem', { name: item }).click()
}

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({ tag_list: fixture('tags'), stash_list: fixture('stash_list'), ...commands }),
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto('/#/tags-stash', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Tags & Stash', level: 2 })).toBeVisible()
}

test.describe('Tags & Stash: states', () => {
  test('populated: tags and stashes listed; no axe violations', async ({ page }) => {
    await boot(page)
    const tags = page.getByRole('list', { name: 'Tags' })
    await expect(tags.getByRole('listitem')).toHaveCount(3)
    await expect(tags).toContainText('v0.3')
    await expect(tags).toContainText('Rig v2 export')
    await expect(tags).toContainText('Plain tag')
    await expect(page.getByRole('list', { name: 'Stashed work' }).getByRole('listitem')).toHaveCount(2)
    await expect(stash(page, 0)).toContainText('WIP lighting')
    await expect(stash(page, 0)).toContainText('main')
    await expect(stash(page, 1)).toContainText('Audio experiment')
    await expectNoAxeViolations(page)
  })

  test('loading: skeletons until both lists arrive', async ({ page }) => {
    await boot(page, {}, { tag_list: [{ hold: true }, { resolve: fixture('tags') }], stash_list: [{ hold: true }, { resolve: fixture('stash_list') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toHaveCount(2)
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(page.getByRole('list', { name: 'Tags' })).toBeVisible()
    await expect(page.getByRole('list', { name: 'Stashed work' })).toBeVisible()
  })

  test('empty: no tags and nothing stashed', async ({ page }) => {
    await boot(page, { tag_list: { tags: [] }, stash_list: fixture('stash_empty') })
    await expect(page.getByText('No tags yet')).toBeVisible()
    await expect(page.getByText('Nothing set aside')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Remove all' })).toHaveCount(0)
    await expectNoAxeViolations(page)
  })

  test('error: each list fails on its own and Retry asks again', async ({ page }) => {
    await boot(page, { stash_list: { __error: { code: 'error', message: 'Cannot read the stash.' } } })
    await expect(page.getByRole('list', { name: 'Tags' })).toBeVisible()
    await expect(page.getByRole('alert')).toContainText('Cannot read the stash')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'stash_list')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'stash_list')).length).toBeGreaterThan(before)
  })

  test('error: tags fail while the stash still works', async ({ page }) => {
    await boot(page, { tag_list: { __error: { code: 'error', message: 'Cannot read tags.' } } })
    await expect(page.getByRole('alert')).toContainText('Cannot read tags')
    await expect(page.getByRole('list', { name: 'Stashed work' })).toBeVisible()
    await expectNoAxeViolations(page)
  })
})

test.describe('Tags & Stash: tags', () => {
  const created = { action: 'create', created: { name: 'v1.0', oid: '00'.repeat(20), annotated: true, message: 'Final' }, deleted: [], missing: [] }

  test('create: validates the name, then tags the latest version with the note', async ({ page }) => {
    await boot(page, { tag_create: created })
    await newTag(page)
    const create = page.getByRole('button', { name: 'Tag the latest version' })
    await expect(create).toBeDisabled()
    await page.getByLabel('Tag name').fill('bad name')
    await expect(page.getByText('Cannot contain spaces')).toBeVisible()
    await page.getByLabel('Tag name').fill('v0.3')
    await expect(page.getByText('A tag with this name exists')).toBeVisible()
    await expect(create).toBeDisabled()
    await expectNoAxeViolations(page)
    await page.getByLabel('Tag name').fill('v1.0')
    await page.getByLabel('Note (optional)').fill(' Final ')
    await create.click()
    await expect.poll(async () => (await ipcCalls(page, 'tag_create')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', name: 'v1.0', options: { commit: null, message: 'Final', tagger: null, email: null, force: false, confirmed: false } },
    ])
    await expect(page.getByLabel('Tag name')).toHaveValue('')
    await expect.poll(async () => (await ipcCalls(page, 'tag_list')).length).toBeGreaterThan(1)
  })

  test('a failed create is shown plainly', async ({ page }) => {
    await boot(page, { tag_create: { __error: { code: 'error', message: 'Tag v1.0 already exists.' } } })
    await newTag(page)
    await page.getByLabel('Tag name').fill('v1.0')
    await page.getByRole('button', { name: 'Tag the latest version' }).click()
    await expect(page.getByRole('alert')).toContainText('Tag v1.0 already exists.')
    await expectNoAxeViolations(page)
  })

  test('delete asks first and explains; cancel changes nothing', async ({ page }) => {
    await boot(page, { tag_delete: { action: 'delete', created: null, deleted: ['v0.2'], missing: [] } })
    await tagMenu(page, 'v0.2', 'Delete tag…')
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Delete tag v0.2?')
    await expect(dlg).toContainText('Only the name is removed.')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'tag_delete')).toHaveLength(0)
    await tagMenu(page, 'v0.2', 'Delete tag…')
    await page.getByRole('alertdialog').getByRole('button', { name: 'Delete tag' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'tag_delete')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', name: 'v0.2' }])
  })
})

test.describe('Tags & Stash: stash actions', () => {
  test('save: message and untracked option go to the CLI, the form clears', async ({ page }) => {
    await boot(page, { stash_save: { action: 'save', index: 0, oid: 'aa'.repeat(20), message: 'WIP', files_updated: null, stashes: [] } })
    await page.getByLabel('Message (optional)').fill('  Grade pass  ')
    await page.getByRole('checkbox', { name: 'Include new files' }).click()
    await page.getByRole('button', { name: 'Stash my changes' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'stash_save')).map((c) => c.args)).toMatchObject([
      { repo: '/repos/hero-film', message: 'Grade pass', includeUntracked: true, paths: [] },
    ])
    await expect(page.getByLabel('Message (optional)')).toHaveValue('')
    await expect.poll(async () => (await ipcCalls(page, 'stash_list')).length).toBeGreaterThan(1)
  })

  test('save is disabled, with the reason, when nothing is changed', async ({ page }) => {
    await boot(page, { repo_status: fixture('status_empty') })
    await expect(page.getByRole('button', { name: 'Stash my changes' })).toBeDisabled()
    await expect(page.getByText('You have no unsaved changes to set aside.')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('busy: apply shows determinate files progress, then reports', async ({ page }) => {
    await boot(page, {}, { stash_apply: fixture('stash_apply_progress') })
    await stash(page, 0).getByRole('button', { name: 'Apply stash 0' }).click()
    const bar = page.getByTestId('stash-progress')
    await expect(bar).toContainText('Apply stash 0: 0 / 3 files')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '3')
    await expect(stash(page, 1).getByRole('button', { name: 'Apply stash 1' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '2')
    await ipcRelease(page)
    await expect(bar).toHaveCount(0)
    await expect(page.getByText('Applied to 3 files').first()).toBeVisible()
    expect((await ipcCalls(page, 'stash_apply')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', index: 0, restoreIndex: false, force: false }])
  })

  test('pop calls pop with the entry index', async ({ page }) => {
    await boot(page)
    await stash(page, 1).getByRole('button', { name: 'Pop stash 1' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'stash_pop')).map((c) => c.args.index)).toEqual([1])
  })

  test('drop asks first and explains; cancel changes nothing', async ({ page }) => {
    await boot(page)
    await stash(page, 1).getByRole('button', { name: 'Drop stash 1' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Drop stash 1?')
    await expect(dlg).toContainText('You cannot get them back.')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'stash_drop')).toHaveLength(0)
    await stash(page, 1).getByRole('button', { name: 'Drop stash 1' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Drop stash' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'stash_drop')).map((c) => c.args.index)).toEqual([1])
  })

  test('remove all asks first, then clears', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Remove all' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Remove every stash?')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Remove all' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'stash_clear')).length).toBe(1)
  })

  test('a failed action is shown plainly and dismissable', async ({ page }) => {
    await boot(page, { stash_pop: { __error: { code: 'error', message: 'Stash 0 clashes with your changes.' } } })
    await stash(page, 0).getByRole('button', { name: 'Pop stash 0' }).click()
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('Stash 0 clashes with your changes.')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await stash(page, 0).getByRole('button', { name: 'Apply stash 0' }).focus()
    await expectFocusRing(page)
    await page.keyboard.press('Tab')
    await expect(stash(page, 0).getByRole('button', { name: 'Pop stash 0' })).toBeFocused()
  })
})
