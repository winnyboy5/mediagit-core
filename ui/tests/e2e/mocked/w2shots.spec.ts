import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { fixture, healthy, ipcRelease, mockIpc, RECENT, showHandler } from './ipc-mock'

// W2 review screenshots: `SHOTS=1 pnpm e2e w2shots`. Skipped in normal runs.
// Output: ui/.impeccable/review/w2-<view>-<state>-<w>-<theme>.png. Every state is its own test, so each gets a fresh page.
test.skip(!process.env.SHOTS, 'screenshots only on demand')

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const SIZES = [
  { w: 1440, h: 900 },
  { w: 1024, h: 700 },
]
const diffHandler = `(a) => a.from === null ? ${JSON.stringify(fixture('diff_working'))} : ${JSON.stringify(fixture('diff_commits'))}`
const extractHandler = `(a) => ({ path: '/preview/' + a.rev.slice(0, 4) + '.png', size: a.expected ?? 1000 })`
const infoHandler = `(a) => a.path.startsWith('/preview/') ? { format: 'png', width: 1280, height: 720, file_size: 1153433 } : { format: 'png', width: 1920, height: 1080, file_size: 1363148 }`

async function shot(page: Page, name: string, theme: string, w: number) {
  mkdirSync('.impeccable/review', { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(350)
  await page.screenshot({ path: `.impeccable/review/w2-${name}-${w}-${theme}.png` })
}

interface Boot {
  commands?: Record<string, unknown>
  handlers?: Record<string, string>
  scripts?: Record<string, unknown[]>
}

async function open(page: Page, route: string, heading: string, b: Boot = {}) {
  await mockIpc(page, {
    commands: healthy({
      repo_log: fixture('log_graph'),
      branch_list: fixture('branches'),
      tag_list: fixture('tags'),
      stash_list: fixture('stash_list'),
      repo_merge: fixture('merge_conflict'),
      repo_extract: { path: '/tmp/p.png', size: 1363148 },
      media_info: fixture('media_png'),
      ...b.commands,
    }),
    handlers: { repo_show: showHandler(), repo_diff: diffHandler, ...b.handlers },
    scripts: b.scripts as never,
    storage: REPO,
  })
  await page.goto(`/#${route}`, { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
}

const startMerge = async (page: Page) => {
  await page.getByRole('button', { name: 'Start a merge' }).first().click()
  await page.getByRole('dialog', { name: 'Start a merge' }).getByRole('button', { name: 'Start merge' }).click()
}
const hero = (page: Page) => page.getByRole('list', { name: 'Changed files' }).getByRole('button', { name: /hero_preview/ })
const undoMenu = (page: Page) => page.getByTestId('commit-detail').getByRole('button', { name: 'Undo…' })
async function history(page: Page, b: Boot = {}) {
  await open(page, '/history', 'History', b)
  await page.getByRole('option').nth(1).click()
  await expect(undoMenu(page)).toBeEnabled()
}

for (const { w, h } of SIZES) {
  test.describe(`${w}px`, () => {
    test.use({ viewport: { width: w, height: h } })
    const T = (name: string, fn: (page: Page, theme: string) => Promise<void>) => test(name, ({ page }, info) => fn(page, info.project.name))

    // V8 Merge & conflicts
    T('V8 empty', async (page, t) => {
      await open(page, '/merge', 'Merge & conflicts')
      await expect(page.getByText('No merge in progress')).toBeVisible()
      await shot(page, 'V8-merge-empty', t, w)
    })
    T('V8 loading', async (page, t) => {
      await open(page, '/merge', 'Merge & conflicts', { scripts: { branch_list: [{ hold: true }, { resolve: fixture('branches') }] } })
      await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
      await shot(page, 'V8-merge-loading', t, w)
    })
    T('V8 error', async (page, t) => {
      await open(page, '/merge', 'Merge & conflicts', { commands: { branch_list: { __error: { code: 'error', message: 'Cannot read branches.' } } } })
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V8-merge-error', t, w)
    })
    T('V8 dialog', async (page, t) => {
      await open(page, '/merge', 'Merge & conflicts')
      await page.getByRole('button', { name: 'Start a merge' }).first().click()
      await expect(page.getByRole('dialog', { name: 'Start a merge' })).toBeVisible()
      await shot(page, 'V8-merge-dialog', t, w)
    })
    T('V8 conflicts', async (page, t) => {
      await open(page, '/merge', 'Merge & conflicts')
      await startMerge(page)
      await page.locator('[data-file="art/hero.psd"]').getByRole('button', { name: 'Keep mine' }).click()
      await expect(page.getByTestId('merge-summary')).toContainText('1 resolved')
      await shot(page, 'V8-merge-conflicts', t, w)
    })
    T('V8 busy', async (page, t) => {
      await open(page, '/merge', 'Merge & conflicts', { scripts: { repo_merge: fixture('merge_progress') } })
      await startMerge(page)
      await expect(page.getByTestId('merge-progress')).toBeVisible()
      await shot(page, 'V8-merge-busy', t, w)
    })
    T('V8 cancel dialog', async (page, t) => {
      await open(page, '/merge', 'Merge & conflicts')
      await startMerge(page)
      await page.getByRole('button', { name: 'Cancel merge' }).click()
      await expect(page.getByRole('alertdialog')).toBeVisible()
      await shot(page, 'V8-merge-cancel', t, w)
    })

    // V9 Compare
    T('V9 populated', async (page, t) => {
      await open(page, '/compare', 'Compare', { handlers: { repo_extract: extractHandler, media_info: infoHandler } })
      await hero(page).click()
      await expect(page.getByRole('img', { name: /^After/ })).toBeVisible()
      await shot(page, 'V9-compare-side', t, w)
    })
    T('V9 onion', async (page, t) => {
      await open(page, '/compare', 'Compare', { handlers: { repo_extract: extractHandler, media_info: infoHandler } })
      await hero(page).click()
      await page.getByRole('tab', { name: 'Onion skin' }).click()
      await expect(page.getByTestId('onion')).toBeVisible()
      await shot(page, 'V9-compare-onion', t, w)
    })
    T('V9 swipe', async (page, t) => {
      await open(page, '/compare', 'Compare', { handlers: { repo_extract: extractHandler, media_info: infoHandler } })
      await hero(page).click()
      await page.getByRole('tab', { name: 'Swipe' }).click()
      await page.getByRole('slider', { name: 'Move divider' }).fill('60')
      await shot(page, 'V9-compare-swipe', t, w)
    })
    T('V9 text file', async (page, t) => {
      await open(page, '/compare', 'Compare')
      await page.getByRole('list', { name: 'Changed files' }).getByRole('button', { name: /shot_list/ }).click()
      await expect(page.getByText('Text file')).toBeVisible()
      await shot(page, 'V9-compare-text', t, w)
    })
    T('V9 loading', async (page, t) => {
      await open(page, '/compare', 'Compare', { handlers: { repo_diff: '' }, scripts: { repo_diff: [{ hold: true }, { resolve: fixture('diff_working') }] } })
      await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
      await shot(page, 'V9-compare-loading', t, w)
    })
    T('V9 empty', async (page, t) => {
      await open(page, '/compare', 'Compare', { handlers: { repo_diff: `() => (${JSON.stringify(fixture('diff_empty'))})` } })
      await expect(page.getByText('No differences')).toBeVisible()
      await shot(page, 'V9-compare-empty', t, w)
    })
    T('V9 error', async (page, t) => {
      await open(page, '/compare', 'Compare', { handlers: { repo_diff: `() => { throw { code: 'error', message: 'Could not read one of the versions.' } }` } })
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V9-compare-error', t, w)
    })
    T('V9 busy', async (page, t) => {
      const slow = [{ progress: { id: 0, msg: 'Extracting', pos: 40, len: 100, unit: 'bytes', done: false } }, { hold: true }]
      await open(page, '/compare', 'Compare', { handlers: { repo_extract: '', repo_diff: `() => (${JSON.stringify(fixture('diff_commits'))})` }, scripts: { repo_extract: slow } })
      await page.getByLabel('To').click()
      await page.getByRole('option').nth(1).click()
      await page.getByLabel('From').click()
      await page.getByRole('option').nth(1).click()
      await hero(page).click()
      await expect(page.getByTestId('compare-extract')).toBeVisible()
      await shot(page, 'V9-compare-busy', t, w)
    })

    // V10 Tags & Stash
    T('V10 populated', async (page, t) => {
      await open(page, '/tags', 'Tags & Stash')
      await expect(page.getByRole('list', { name: 'Stashed work' })).toBeVisible()
      await shot(page, 'V10-tags-populated', t, w)
    })
    T('V10 loading', async (page, t) => {
      await open(page, '/tags', 'Tags & Stash', { scripts: { tag_list: [{ hold: true }, { resolve: fixture('tags') }], stash_list: [{ hold: true }, { resolve: fixture('stash_list') }] } })
      await expect(page.getByRole('status', { name: 'Loading' }).first()).toBeVisible()
      await shot(page, 'V10-tags-loading', t, w)
    })
    T('V10 empty', async (page, t) => {
      await open(page, '/tags', 'Tags & Stash', { commands: { tag_list: { tags: [] }, stash_list: fixture('stash_empty') } })
      await expect(page.getByText('Nothing set aside')).toBeVisible()
      await shot(page, 'V10-tags-empty', t, w)
    })
    T('V10 error', async (page, t) => {
      await open(page, '/tags', 'Tags & Stash', { commands: { stash_list: { __error: { code: 'error', message: 'Cannot read the stash.' } } } })
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V10-tags-error', t, w)
    })
    T('V10 busy', async (page, t) => {
      await open(page, '/tags', 'Tags & Stash', { scripts: { stash_apply: fixture('stash_apply_progress') } })
      await page.getByRole('button', { name: 'Apply stash 0' }).click()
      await expect(page.getByTestId('stash-progress')).toBeVisible()
      await shot(page, 'V10-tags-busy', t, w)
    })
    T('V10 drop dialog', async (page, t) => {
      await open(page, '/tags', 'Tags & Stash')
      await page.getByRole('button', { name: 'Drop stash 1' }).click()
      await expect(page.getByRole('alertdialog')).toBeVisible()
      await shot(page, 'V10-tags-drop', t, w)
    })

    // Undo actions
    T('Undo menu', async (page, t) => {
      await history(page)
      await undoMenu(page).click()
      await expect(page.getByRole('menuitem').first()).toBeVisible()
      await shot(page, 'undo-menu', t, w)
    })
    T('Undo hard dialog', async (page, t) => {
      await history(page)
      await undoMenu(page).click()
      await page.getByRole('menuitem', { name: /Discard everything/ }).click()
      await page.getByRole('checkbox', { name: /I understand/ }).click()
      await shot(page, 'undo-hard', t, w)
    })
    T('Undo busy', async (page, t) => {
      await history(page, { scripts: { repo_reset_hard: fixture('branch_switch_progress') } })
      await undoMenu(page).click()
      await page.getByRole('menuitem', { name: /Discard everything/ }).click()
      await page.getByRole('checkbox', { name: /I understand/ }).click()
      await page.getByRole('button', { name: 'Discard and go back' }).click()
      await expect(page.getByTestId('undo-progress')).toBeVisible()
      await shot(page, 'undo-busy', t, w)
      await ipcRelease(page)
    })
    T('Undo restore dialog', async (page, t) => {
      await history(page)
      await page.getByRole('tab', { name: 'Files at this version' }).click()
      await page.getByRole('tree', { name: 'Files at this version' }).getByRole('treeitem', { name: /hero_preview/ }).click()
      await page.getByRole('button', { name: 'Restore this file…' }).click()
      await expect(page.getByRole('alertdialog')).toBeVisible()
      await shot(page, 'undo-restore', t, w)
    })
    T('Undo error', async (page, t) => {
      await history(page, { commands: { repo_reset: { __error: { code: 'error', message: 'Cannot move the branch: it is protected.' } } } })
      await undoMenu(page).click()
      await page.getByRole('menuitem', { name: /keep changes ready/ }).click()
      await page.getByRole('alertdialog').getByRole('button', { name: 'Move branch back' }).click()
      await expect(page.getByTestId('commit-detail').getByRole('alert')).toBeVisible()
      await shot(page, 'undo-error', t, w)
    })
  })
}
