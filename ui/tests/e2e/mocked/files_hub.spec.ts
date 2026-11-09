import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

// Files hub: the All files explorer, Include checkboxes, and live staging numbers.
const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.changes.mode': JSON.stringify('tree') }
const files = (page: Page) => page.getByRole('tree', { name: 'All files' })

// ls-files leaves ignored files out unless asked.
const lsHandler = (all: unknown[]) => `(a) => ({ files: ${JSON.stringify(all)}.filter((f) => a.ignored || f.status !== 'ignored') })`

async function boot(page: Page, all: unknown[] = fixture<{ files: unknown[] }>('lsfiles_populated').files) {
  await mockIpc(page, { commands: healthy(), storage: REPO, handlers: { repo_ls_files: lsHandler(all) } })
  await page.goto('/#/files/all')
  await expect(files(page)).toBeVisible()
}

test.describe('All files', () => {
  test('lists the working folder: status chips only on changed files, folders say how many changed; axe clean', async ({ page }) => {
    await boot(page)
    const f = files(page)
    await expect(page.getByRole('heading', { level: 1 })).toHaveCount(1)
    await expect(page.getByTestId('files-summary')).toContainText('12 files, 9 changed')
    await expect(f.getByRole('treeitem', { name: /^art\// })).toContainText('1 changed')
    await expect(f.getByRole('treeitem', { name: /hero_preview/ })).not.toContainText(/Edited|New|Deleted/)
    await expect(f.getByRole('treeitem', { name: /hero\.psd/ })).toContainText('Edited')
    await expect(f.getByRole('treeitem', { name: /vo_take1/ })).toContainText('Deleted')
    // An unchanged file has no Include box; a changed one does, ticked when staged.
    await expect(f.getByRole('checkbox', { name: 'Include hero_preview.png' })).toHaveCount(0)
    await expect(f.getByRole('checkbox', { name: 'Include hero.psd' })).toHaveAttribute('aria-checked', 'true')
    await expect(f.getByRole('treeitem', { name: /\.cache/ })).toHaveCount(0)
    expect((await ipcCalls(page, 'repo_ls_files'))[0]!.args).toMatchObject({ repo: '/repos/hero-film', ignored: false })
    await expectNoAxeViolations(page)
  })

  test('the details panel is gone until a file is picked, then shows preview and details', async ({ page }) => {
    await boot(page)
    await expect(page.getByTestId('inspector')).toHaveCount(0)
    await files(page).getByRole('treeitem', { name: /hero_preview/ }).click()
    const insp = page.getByTestId('inspector')
    await expect(insp).toContainText('hero_preview.png')
    await expect(insp.getByTestId('preview')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('find narrows the tree to matching files; no match says so', async ({ page }) => {
    await boot(page)
    await page.getByRole('searchbox', { name: 'Find a file' }).fill('VO take3')
    await expect(files(page).getByRole('treeitem', { name: /vo_take3/ })).toBeVisible()
    await expect(files(page).getByRole('treeitem', { name: /hero/ })).toHaveCount(0)
    await expectNoAxeViolations(page)
    await page.getByRole('searchbox', { name: 'Find a file' }).fill('zzz')
    await expect(page.getByText('No match')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('Show ignored files asks for them and dims them', async ({ page }) => {
    await boot(page)
    await page.getByRole('checkbox', { name: 'Show ignored files' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_ls_files')).length).toBe(2)
    expect((await ipcCalls(page, 'repo_ls_files'))[1]!.args).toMatchObject({ ignored: true })
    await expect(files(page).getByRole('treeitem', { name: /\.cache/ })).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('ticking a changed file includes it; a ticked one excludes it', async ({ page }) => {
    await boot(page)
    await files(page).getByRole('checkbox', { name: 'Include vo_take3.wav' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_add')).length).toBe(1)
    expect((await ipcCalls(page, 'repo_add'))[0]!.args).toMatchObject({ paths: ['audio/vo_take3.wav'] })
    await files(page).getByRole('checkbox', { name: 'Include hero.psd' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_unstage')).length).toBe(1)
  })

  test('the Save a version dock sits under the list and counts included changes', async ({ page }) => {
    await boot(page)
    await expect(page.getByTestId('include-count')).toContainText('3 of 9 changes included')
    await expect(page.getByRole('button', { name: 'Save version' })).toBeDisabled()
  })

  test('100,000 files stay virtualized', async ({ page }) => {
    const many = Array.from({ length: 100_000 }, (_, i) => ({ path: `shots/s${String(i % 500).padStart(3, '0')}/f${i}.exr`, status: 'unchanged', staged: false, size: i }))
    await boot(page, many)
    await expect(page.getByTestId('files-summary')).toContainText('100,000 files')
    const n = await files(page).getByRole('treeitem').count()
    expect(n).toBeGreaterThan(0)
    expect(n).toBeLessThan(100)
  })
})

test.describe('Changes: Include and live staging numbers', () => {
  const bars = [
    { progress: { id: 3, msg: 'env/sky_hdri.exr', pos: 0, len: 4_000_000_000, unit: 'bytes', done: false } },
    { progress: { id: 4, msg: 'files', pos: 0, len: 6, unit: 'items', done: false } },
    { delay: 1100, progress: { id: 3, msg: 'env/sky_hdri.exr', pos: 1_000_000_000, len: 4_000_000_000, unit: 'bytes', done: false } },
    { progress: { id: 4, msg: 'files', pos: 2, len: 6, unit: 'items', done: false } },
    { delay: 1200, progress: { id: 3, msg: 'env/fog_vol.vdb', pos: 2_000_000_000, len: 4_000_000_000, unit: 'bytes', done: false } },
    { progress: { id: 4, msg: 'files', pos: 3, len: 6, unit: 'items', done: false } },
    { hold: true },
    { resolve: { added: 3, modified: 3, deleted: 0, bytes: 4_000_000_000 } },
  ]

  test('files done of total, bytes, rate and time left show in the Changes section and the activity pill', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: { ...REPO, 'mg.changes.mode': JSON.stringify('flat') }, scripts: { repo_add: bars } })
    await page.goto('/#/files/changes')
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    // Progress lives in the header pill (and the row's own bar); the Save dock has no meter.
    await expect(page.getByTestId('staging-meter')).toHaveCount(0)
    const pill = page.locator('[data-activity-pill]')
    await expect(pill.getByTestId('pill-files')).toHaveText('3 of 6 files')
    await expect(pill.getByTestId('pill-bytes')).toHaveText('1.9 GB of 3.7 GB')
    await expect(pill.getByTestId('pill-rate')).toContainText('/s')
    await expect(pill.getByTestId('pill-eta')).toContainText('left')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
  })

  test('with no length reported the pill shows no invented total', async ({ page }) => {
    const script = [{ progress: { id: 1, msg: 'files', pos: 5, len: null, unit: 'items', done: false } }, { hold: true }, { resolve: null }]
    await mockIpc(page, { commands: healthy(), storage: REPO, scripts: { repo_add: script } })
    await page.goto('/#/files/changes')
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    await expect(page.getByTestId('pill-files')).toHaveText('5 files')
    await expect(page.getByTestId('pill-bytes')).toHaveCount(0)
    await expect(page.getByTestId('pill-eta')).toHaveCount(0)
    await ipcRelease(page)
  })

  test('a folder row is partly included until every file below it is', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: REPO })
    await page.goto('/#/files/changes')
    await expect(page.getByRole('tree', { name: 'Changes' }).getByRole('checkbox', { name: 'Include art' })).toHaveAttribute('aria-checked', 'true')
    await expect(page.getByRole('tree', { name: 'Changes' }).getByRole('checkbox', { name: 'Include char' })).toHaveAttribute('aria-checked', 'mixed')
  })
})
