import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcEmit, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.changes.mode': JSON.stringify('flat') }

const unstaged = (page: Page) => page.getByRole('tree', { name: 'Files not staged' })
const staged = (page: Page) => page.getByRole('tree', { name: 'Staged files' })

async function boot(page: Page, extra: Record<string, unknown> = {}, storage: Record<string, string> = {}, scripts = {}) {
  await mockIpc(page, { commands: healthy(extra), storage: { ...REPO, ...storage }, scripts })
  await page.goto('/#/changes')
  await expect(page.getByRole('heading', { name: 'Changes', level: 1 })).toBeVisible()
}

test.describe('Changes: states', () => {
  test('populated: both groups, counts, nav badge, branch; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(page.getByRole('heading', { name: /Staged · 3 files/ })).toBeVisible()
    await expect(page.getByRole('heading', { name: /Not staged · 6 files/ })).toBeVisible()
    await expect(staged(page).getByRole('treeitem')).toHaveCount(3)
    await expect(unstaged(page).getByRole('treeitem')).toHaveCount(6)
    const nav = page.getByRole('navigation', { name: 'Main' })
    await expect(nav.getByText('9 changed files', { exact: false })).toHaveCount(0) // label is on the badge, not text
    await expect(nav.getByLabel('9 changed files')).toBeVisible()
    await expect(page.getByTestId('branch-indicator')).toContainText('main')
    await expect(page.getByRole('button', { name: 'Save version (commit)' })).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('empty: no changes state', async ({ page }) => {
    await boot(page, { repo_status: fixture('status_empty') })
    await expect(page.getByText('No changes')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Save version (commit)' })).toHaveCount(0)
    await expectNoAxeViolations(page)
  })

  test('error: names the problem and offers no Retry when retrying cannot help', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ repo_status: fixture('status_error') }), storage: REPO })
    await page.goto('/#/changes')
    const alert = page.getByRole('alert')
    await expect(alert.getByRole('heading')).toContainText('Not a MediaGit repository')
    await expect(alert).not.toContainText('error')
    await expect(alert.getByRole('button', { name: 'Retry' })).toHaveCount(0)
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton while status is pending, then rows', async ({ page }) => {
    await boot(page, {}, {}, { repo_status: [{ hold: true }, { resolve: fixture('status_populated') }] })
    await expect(unstaged(page)).toHaveCount(0)
    await expect(page.getByRole('status').first()).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(unstaged(page).getByRole('treeitem')).toHaveCount(6)
  })

  test('busy: staging holds the controls disabled and shows the op in the bar', async ({ page }) => {
    await boot(page, {}, {}, { repo_add: [{ hold: true }, { resolve: null }] })
    await page.getByRole('button', { name: 'Stage all', exact: true }).click()
    const section = page.getByRole('region', { name: /Not staged/ })
    await expect(section.getByRole('button', { name: 'Stage all', exact: true })).toBeDisabled()
    await expect(page.locator('[data-activity-bar]')).toContainText('Staging 6 files')
    await expect(page.locator('section.changes')).toHaveAttribute('aria-busy', 'true')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(section.getByRole('button', { name: 'Stage all', exact: true })).toBeEnabled()
    expect((await ipcCalls(page, 'repo_add'))[0]!.args).toEqual({ repo: '/repos/hero-film', paths: ['.'] })
  })
})

test.describe('Changes: tree and flat', () => {
  test('Tree groups by folder and collapses; Flat lists paths; choice persists', async ({ page }) => {
    await boot(page, {}, { 'mg.changes.mode': JSON.stringify('tree') })
    await expect(unstaged(page).getByRole('treeitem', { name: /audio\// })).toBeVisible()
    await unstaged(page).getByRole('treeitem', { name: /audio\// }).click()
    await page.keyboard.press('ArrowLeft')
    await expect(unstaged(page).getByRole('treeitem', { name: /audio\// })).toHaveAttribute('aria-expanded', 'false')
    await page.getByRole('button', { name: 'Flat' }).click()
    await expect(unstaged(page).getByRole('treeitem', { name: /audio\/vo_take3.wav/ })).toBeVisible()
    await page.reload()
    await expect(page.getByRole('button', { name: 'Flat' })).toHaveAttribute('data-state', 'on')
  })

  test('100,000 files stay virtualized', async ({ page }) => {
    const untracked = Array.from({ length: 100_000 }, (_, i) => `shots/s${String(i % 500).padStart(3, '0')}/f${i}.exr`)
    await boot(page, { repo_status: { ...fixture<object>('status_empty'), untracked } })
    await expect(page.getByRole('heading', { name: /Not staged · 100000 files/ })).toBeVisible()
    const rendered = await unstaged(page).getByRole('treeitem').count()
    expect(rendered).toBeGreaterThan(0)
    expect(rendered).toBeLessThan(100)
    await unstaged(page).focus()
    await page.keyboard.press('End')
    await expect(unstaged(page).getByRole('treeitem').last()).toContainText('f')
  })
})

test.describe('Changes: selection and inspector', () => {
  test('single select fills the inspector: preview, details, lock, history', async ({ page }) => {
    await boot(page)
    const insp = page.getByTestId('inspector')
    await expect(insp).toContainText('Select a file to see its details.')
    await unstaged(page).getByRole('treeitem', { name: /vo_take3/ }).click()
    await expect(insp.getByText('vo_take3.wav').first()).toBeVisible()
    await expect(insp).toContainText('Audio')
    await expect(insp).toContainText('New, not staged')
    for (const id of ['preview', 'media-details', 'lock-state', 'file-history']) await expect(insp.getByTestId(id)).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('Ctrl and Shift build a multi-selection summarised in the inspector', async ({ page }) => {
    await boot(page)
    const list = unstaged(page)
    await list.getByRole('treeitem').nth(0).click()
    await list.getByRole('treeitem').nth(2).click({ modifiers: ['Shift'] })
    const sum = page.getByTestId('selection-summary')
    await expect(sum).toContainText('3 files selected')
    await list.getByRole('treeitem').nth(4).click({ modifiers: ['Control'] })
    await expect(sum).toContainText('4 files selected')
    await expect(page.getByRole('heading', { name: 'By type' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Stage selected (4)' })).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('details panel hides and shows; below 1200px it is an overlay', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Hide details panel' }).first().click()
    await expect(page.getByTestId('inspector')).toHaveCount(0)
    await page.getByRole('button', { name: 'Show details panel' }).click()
    await expect(page.getByTestId('inspector')).toBeVisible()
    await page.setViewportSize({ width: 1024, height: 700 })
    await expect(page.getByTestId('inspector')).toHaveCount(0)
    await page.getByRole('button', { name: 'Show details panel' }).click()
    const box = await page.getByTestId('inspector').boundingBox()
    expect(box!.x + box!.width).toBeLessThanOrEqual(1024)
    await expectNoAxeViolations(page)
  })
})

test.describe('Changes: stage and commit', () => {
  test('Stage selected sends only the selected paths', async ({ page }) => {
    await boot(page)
    await unstaged(page).getByRole('treeitem', { name: /vo_take3/ }).click()
    await page.getByRole('button', { name: 'Stage selected (1)' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_add')).length).toBe(1)
    expect((await ipcCalls(page, 'repo_add'))[0]!.args).toEqual({ repo: '/repos/hero-film', paths: ['audio/vo_take3.wav'] })
  })

  test('commit needs staged files and a message; Ctrl+Enter saves it', async ({ page }) => {
    await boot(page)
    const save = page.getByRole('button', { name: 'Save version (commit)' })
    await expect(save).toBeDisabled()
    await page.getByLabel('Message').fill('Grade pass on shot_010')
    await expect(save).toBeEnabled()
    await page.getByLabel('Message').press('Control+Enter')
    await expect.poll(async () => (await ipcCalls(page, 'repo_commit')).length).toBe(1)
    expect((await ipcCalls(page, 'repo_commit'))[0]!.args).toEqual({ repo: '/repos/hero-film', message: 'Grade pass on shot_010' })
    await expect(page.getByLabel('Message')).toHaveValue('')
  })

  test('a failing commit keeps the message and shows code + message', async ({ page }) => {
    await boot(page, { repo_commit: { __error: { code: 'E_HOOK', message: 'Pre-commit check failed.' } } })
    await page.getByLabel('Message').fill('Try')
    await page.getByRole('button', { name: 'Save version (commit)' }).click()
    await expect(page.getByRole('alert')).toContainText('E_HOOK')
    await expect(page.getByLabel('Message')).toHaveValue('Try')
  })
})

test.describe('Changes: refresh and shell', () => {
  test('window focus and repo-changed both re-read status', async ({ page }) => {
    await boot(page)
    const n = async () => (await ipcCalls(page, 'repo_status')).length
    const a = await n()
    await page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await expect.poll(n).toBeGreaterThan(a)
    const b = await n()
    await ipcEmit(page, 'repo-changed')
    await expect.poll(n).toBeGreaterThan(b)
  })

  test('Ctrl+2 goes to History, Ctrl+1 back', async ({ page }) => {
    await boot(page)
    await page.keyboard.press('Control+2')
    await expect(page).toHaveURL(/#\/history$/)
    await page.keyboard.press('Control+1')
    await expect(page).toHaveURL(/#\/changes$/)
  })

  test('keyboard: list takes focus with a ring; arrows move, Space toggles', async ({ page }) => {
    await boot(page)
    await unstaged(page).focus()
    await expectFocusRing(page)
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowDown')
    await expect(unstaged(page).getByRole('treeitem').nth(1)).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press('Space')
    await expect(unstaged(page).getByRole('treeitem').nth(1)).toHaveAttribute('aria-selected', 'false')
  })

  test('Welcome via switcher: no repository open redirects to Welcome', async ({ page }) => {
    await mockIpc(page, { commands: healthy() })
    await page.goto('/#/changes')
    await expect(page).toHaveURL(/#\/welcome$/)
  })
})
