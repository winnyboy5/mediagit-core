import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, pagedLogHandler, RECENT, showHandler } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const oid = (n: number) => n.toString(16).padStart(4, '0').repeat(10)

const list = (page: Page) => page.getByRole('listbox', { name: 'Versions, newest first' })
const rowsOf = (page: Page) => list(page).getByRole('option')

async function boot(page: Page, opts: { commands?: Record<string, unknown>; handlers?: Record<string, string>; scripts?: Record<string, never[]> | Record<string, unknown[]> } = {}) {
  await mockIpc(page, {
    commands: healthy({ repo_log: fixture('log_graph'), ...opts.commands }),
    handlers: { repo_show: showHandler(), ...opts.handlers },
    scripts: opts.scripts as never,
    storage: REPO,
  })
  await page.goto('/#/history', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'History', level: 1 })).toBeVisible()
}

test.describe('History: states', () => {
  test('populated: graph rows, HEAD, refs on the commit, detail of the newest version; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(rowsOf(page)).toHaveCount(11)
    await expect(list(page).locator('svg.graph-cell')).toHaveCount(11)
    await expect(rowsOf(page).first()).toContainText('Merge grade pass into main')
    await expect(rowsOf(page).first()).toContainText('(current version)')
    await expect(rowsOf(page).first()).toHaveAttribute('aria-selected', 'true')
    const detail = page.getByTestId('commit-detail')
    await expect(detail.getByRole('heading', { name: 'Merge grade pass into main' })).toBeVisible()
    await expect(detail).toContainText('Merge of 2 versions')
    await expect(detail).toContainText('Shot 010 and 020 now match the reference.')
    await expect(detail.getByRole('list', { name: 'Names pointing at this version' })).toContainText('main')
    await expect(detail.getByRole('tab', { name: /Changes in this version · 4/ })).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton until the first page arrives', async ({ page }) => {
    await boot(page, { scripts: { repo_log: [{ hold: true }, { resolve: fixture('log_graph') }] } })
    await expect(rowsOf(page)).toHaveCount(0)
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(rowsOf(page)).toHaveCount(11)
  })

  test('empty: a repository with no saved versions', async ({ page }) => {
    await boot(page, { commands: { repo_log: { commits: [], refs: [] } } })
    await expect(page.getByText('No saved versions yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('error: code + message and Retry that asks again', async ({ page }) => {
    await boot(page, { commands: { repo_log: { __error: { code: 'error', message: 'Could not read history.' } } } })
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('Could not read history.')
    await expect(alert).toContainText('error')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'repo_log')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_log')).length).toBeGreaterThan(before)
  })

  test('busy: the list stays visible and marked busy while a refresh runs', async ({ page }) => {
    await boot(page)
    await expect(rowsOf(page)).toHaveCount(11)
    await page.getByRole('button', { name: 'Refresh history' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_log')).length).toBe(2)
    await expect(rowsOf(page)).toHaveCount(11)
  })
})

test.describe('History: interaction', () => {
  test('keyboard: arrows move the selection and load that version; focus ring is visible', async ({ page }) => {
    await boot(page)
    await list(page).focus()
    await expectFocusRing(page)
    await page.keyboard.press('ArrowDown')
    await expect(rowsOf(page).nth(1)).toHaveAttribute('aria-selected', 'true')
    await expect.poll(async () => (await ipcCalls(page, 'repo_show')).map((c) => c.args.rev)).toContain(oid(1))
    await page.keyboard.press('End')
    await expect(rowsOf(page).last()).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press('Home')
    await page.keyboard.press('ArrowUp')
    await expect(rowsOf(page).first()).toHaveAttribute('aria-selected', 'true')
  })

  test('click selects a version', async ({ page }) => {
    await boot(page)
    await rowsOf(page).nth(7).click()
    await expect(rowsOf(page).nth(7)).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByTestId('commit-detail')).toContainText('v1.0')
  })

  test('search filters by message or author and hides the graph; clearing restores it', async ({ page }) => {
    await boot(page)
    const search = page.getByRole('searchbox', { name: 'Search versions' })
    await search.fill('sam')
    await expect(rowsOf(page)).toHaveCount(3)
    await expect(list(page).locator('svg.graph-cell')).toHaveCount(0)
    await expect(page.getByText('The graph is hidden while you search.')).toBeVisible()
    await expectNoAxeViolations(page)
    await search.fill('zzzz')
    await expect(page.getByText('No versions match "zzzz".')).toBeVisible()
    await search.fill('')
    await expect(list(page).locator('svg.graph-cell')).toHaveCount(11)
  })

  test('this branch only drops versions HEAD does not contain', async ({ page }) => {
    const log = fixture<{ refs: { kind: string; oid: string }[] }>('log_graph')
    const refs = log.refs.map((r) => (r.kind === 'head' ? { ...r, oid: oid(1) } : r))
    await boot(page, { commands: { repo_log: { ...log, refs } } })
    await expect(rowsOf(page)).toHaveCount(11)
    await page.getByRole('button', { name: 'This branch only' }).click()
    await expect(rowsOf(page)).toHaveCount(7)
    await expect(rowsOf(page).filter({ hasText: 'Final grade on shot_010' })).toHaveCount(0)
    await page.getByRole('button', { name: 'All branches' }).click()
    await expect(rowsOf(page)).toHaveCount(11)
  })

  test('pages 200 at a time while scrolling and says when history ends', async ({ page }) => {
    await boot(page, { handlers: { repo_log: pagedLogHandler(450) } })
    await expect(rowsOf(page).first()).toBeVisible()
    const skips = async () => (await ipcCalls(page, 'repo_log')).map((c) => c.args.skip)
    expect(await skips()).toEqual([0])
    for (let i = 0; i < 3; i++) {
      await list(page).evaluate((el) => (el.scrollTop = el.scrollHeight))
      await page.waitForTimeout(250)
    }
    await expect.poll(skips).toEqual([0, 200, 400])
    await expect(page.getByText('That is the whole history.')).toBeVisible()
  })

  test('files at this version: lazy tree, folders, pick a file to preview it', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1500 })
    await boot(page, { scripts: { repo_extract: [{ resolve: { path: '/tmp/mediagit-desktop-preview/p.png', size: 1363148 } }] } })
    await page.getByRole('tab', { name: 'Files at this version' }).click()
    const tree = page.getByRole('tree', { name: 'Files at this version' })
    // the list is virtualized: not every row is in the DOM, but the set size says how many exist
    await expect(tree.getByRole('treeitem').first()).toHaveAttribute('aria-setsize', String(8 + 5)) // 5 folders + 8 files
    await expect(tree.getByRole('treeitem', { name: /^art\// })).toBeVisible()
    expect((await ipcCalls(page, 'repo_show')).filter((c) => c.args.tree === true)).toHaveLength(1)
    await expectNoAxeViolations(page)
    await tree.getByRole('treeitem', { name: /hero_preview/ }).click()
    const insp = page.getByTestId('inspector')
    await expect(insp.getByRole('img', { name: 'Preview of hero_preview.png' })).toBeVisible()
    await expect(insp).toContainText('Version')
    const call = (await ipcCalls(page, 'repo_extract'))[0]!
    expect(call.args).toMatchObject({ rev: oid(0), path: 'art/hero_preview.png', expected: 1363148 })
    await expectNoAxeViolations(page)
  })

  test('a large file shows determinate extraction progress in the inspector', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1500 })
    await boot(page, { scripts: { repo_extract: fixture('extract_big') } })
    await page.getByRole('tab', { name: 'Files at this version' }).click()
    await page.getByRole('tree', { name: 'Files at this version' }).getByRole('treeitem', { name: /shot_010/ }).click()
    const prev = page.getByTestId('preview')
    await expect(prev).toContainText('Getting shot_010.mp4 from history')
    const bar = prev.getByRole('progressbar')
    await expect(bar).toHaveAttribute('aria-valuenow', String(40 * 1024 * 1024))
    await expect(bar).toHaveAttribute('aria-valuemax', String(402 * 1024 * 1024))
    await expect(prev).toContainText('--') // no rate until two samples are 1 s apart
    await ipcRelease(page)
    await expect(bar).toHaveAttribute('aria-valuenow', String(300 * 1024 * 1024))
    await ipcRelease(page)
    await expect(prev).not.toContainText('Getting shot_010.mp4')
  })

  test('a deleted file says so instead of previewing', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1500 })
    await boot(page)
    await page.getByRole('tree', { name: 'Files this version changed' }).getByRole('treeitem', { name: /vo_take1/ }).click()
    await expect(page.getByTestId('preview')).toContainText('deleted here')
    expect(await ipcCalls(page, 'repo_extract')).toHaveLength(0)
  })

  test('a branch name on the version offers a switch, confirmed in a dialog', async ({ page }) => {
    await boot(page)
    await rowsOf(page).nth(2).click() // Final grade: feature/grade
    await page.getByRole('button', { name: 'Switch to feature/grade' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Switch to feature/grade?')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Switch branch' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'branch_switch')).map((c) => c.args.name)).toEqual(['feature/grade'])
  })
})
