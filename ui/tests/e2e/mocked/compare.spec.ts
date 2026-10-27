import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const HEAD = '0000000000000000000000000000000000000000'
const file = (page: Page, name: string | RegExp) => page.getByRole('list', { name: 'Changed files' }).getByRole('button', { name })
const pane = (page: Page) => page.getByTestId('compare-pane')

// Working folder against HEAD gets the working report; two versions get the commits report.
const diffHandler = `(a) => a.from === null ? ${JSON.stringify(fixture('diff_working'))} : ${JSON.stringify(fixture('diff_commits'))}`
// Each side extracts to its own path; the "after" side reports different properties.
const extractHandler = `(a) => ({ path: '/preview/' + a.rev.slice(0, 4) + '.png', size: a.expected ?? 1000 })`
const infoHandler = `(a) => a.path.startsWith('/preview/') ? { format: 'png', width: 1280, height: 720, file_size: 1153433 } : { format: 'png', width: 1920, height: 1080, file_size: 1363148 }`

async function boot(page: Page, commands: Record<string, unknown> = {}, handlers: Record<string, string> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({ repo_log: fixture('log_graph'), ...commands }),
    handlers: { repo_diff: diffHandler, repo_extract: extractHandler, media_info: infoHandler, ...handlers },
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto('/#/compare', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Compare', level: 1 })).toBeVisible()
}

test.describe('Compare: states', () => {
  test('populated: working folder against the latest version; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(file(page, /hero_preview/)).toBeVisible()
    await expect(page.getByRole('list', { name: 'Changed files' }).getByRole('listitem')).toHaveCount(4)
    await expect(page.getByLabel('From')).toBeDisabled()
    await expect(page.getByLabel('From')).toContainText('Latest saved version')
    await expect(page.getByLabel('To')).toContainText('Working folder')
    await expect(pane(page)).toContainText('Pick a file')
    expect((await ipcCalls(page, 'repo_diff')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', from: null, to: null }])
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton until the comparison arrives', async ({ page }) => {
    await boot(page, {}, { repo_diff: '' }, { repo_diff: [{ hold: true }, { resolve: fixture('diff_working') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(file(page, /hero_preview/)).toBeVisible()
  })

  test('empty: no differences', async ({ page }) => {
    await boot(page, {}, { repo_diff: `() => (${JSON.stringify(fixture('diff_empty'))})` })
    await expect(page.getByText('No differences')).toBeVisible()
    await expect(page.getByText('Your working folder matches the latest saved version.')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('empty: no saved versions to compare', async ({ page }) => {
    await boot(page, { repo_log: { commits: [], refs: [] } })
    await expect(page.getByText('Nothing to compare')).toBeVisible()
    await expect(page.getByLabel('To')).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('error: the comparison fails, Retry asks again', async ({ page }) => {
    await boot(page, {}, { repo_diff: `() => { throw { code: 'error', message: 'Could not read one of the versions.' } }` })
    await expect(page.getByRole('alert')).toContainText('Could not read one of the versions')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'repo_diff')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_diff')).length).toBeGreaterThan(before)
  })

  test('error: history cannot be read', async ({ page }) => {
    await boot(page, { repo_log: { __error: { code: 'error', message: 'Could not read history.' } } })
    await expect(page.getByRole('alert')).toContainText('Could not read history')
    await expectNoAxeViolations(page)
  })
})

test.describe('Compare: files and previews', () => {
  test('an image: side by side, both extracted, text/binary and properties shown', async ({ page }) => {
    await boot(page)
    await file(page, /hero_preview/).click()
    await expect(file(page, /hero_preview/)).toHaveAttribute('aria-pressed', 'true')
    await expect(pane(page).getByRole('img', { name: 'Before: art/hero_preview.png' })).toBeVisible()
    await expect(pane(page).getByRole('img', { name: 'After: art/hero_preview.png' })).toBeVisible()
    await expect(pane(page)).toContainText('Before · 0000000')
    await expect(pane(page)).toContainText('After · Working folder')
    await expect(pane(page)).toContainText('Binary file')
    const props = pane(page).getByRole('table')
    await expect(props.getByRole('row', { name: /Size/ })).toContainText('1.1 MB')
    await expect(props.getByRole('row', { name: /Width/ })).toContainText('1280')
    await expect(props.getByRole('row', { name: /Width/ })).toContainText('1920')
    // Only the working side skips extraction: HEAD's copy is extracted once.
    expect((await ipcCalls(page, 'repo_extract')).map((c) => c.args.rev)).toEqual([HEAD])
    await expectNoAxeViolations(page)
  })

  test('"Only what changed" hides unchanged properties', async ({ page }) => {
    await boot(page)
    await file(page, /hero_preview/).click()
    const props = pane(page).getByRole('table')
    await expect(props.getByRole('row', { name: /Format/ })).toHaveCount(0)
    await pane(page).getByRole('checkbox', { name: 'Only what changed' }).click()
    await expect(props.getByRole('row', { name: /Format/ })).toHaveCount(1)
  })

  test('onion skin blends the sides with a slider', async ({ page }) => {
    await boot(page)
    await file(page, /hero_preview/).click()
    await pane(page).getByRole('tab', { name: 'Onion skin' }).click()
    const slider = pane(page).getByRole('slider', { name: 'Blend before and after' })
    await slider.fill('30')
    await expect(pane(page).getByTestId('onion').locator('img.top')).toHaveCSS('opacity', '0.3')
    await expectNoAxeViolations(page)
  })

  test('swipe moves a divider with a slider', async ({ page }) => {
    await boot(page)
    await file(page, /hero_preview/).click()
    await pane(page).getByRole('tab', { name: 'Swipe' }).click()
    await pane(page).getByRole('slider', { name: 'Move divider' }).fill('70')
    const clip = await pane(page).getByTestId('swipe').locator('img.top').evaluate((el) => (el as HTMLElement).style.clipPath)
    expect(clip).toContain('70%')
    await expectNoAxeViolations(page)
  })

  test('a text file: marked as text, no picture, sizes still compared', async ({ page }) => {
    await boot(page)
    await file(page, /shot_list/).click()
    await expect(pane(page)).toContainText('Text file')
    await expect(pane(page)).toContainText('Line-by-line changes are not shown here yet')
    await expect(pane(page)).toContainText('No picture preview')
    await expect(pane(page).getByRole('img')).toHaveCount(0)
    await expect(pane(page).getByRole('table').getByRole('row', { name: /Size/ })).toContainText('2.0 KB')
    expect(await ipcCalls(page, 'repo_extract')).toHaveLength(0)
    await expectNoAxeViolations(page)
  })

  test('an added image has only an after side', async ({ page }) => {
    await boot(page)
    await file(page, /new_concept/).click()
    await expect(pane(page).getByRole('img', { name: 'After: art/new_concept.png' })).toBeVisible()
    await expect(pane(page).getByRole('img', { name: /^Before/ })).toHaveCount(0)
    await expect(pane(page)).toContainText('new file')
  })

  test('busy: both sides show an extraction bar while they load', async ({ page }) => {
    const slow = [
      { progress: { id: 0, msg: 'Extracting', pos: 40, len: 100, unit: 'bytes', done: false } },
      { hold: true },
      { resolve: { path: '/preview/x.png', size: 100 } },
    ]
    await boot(page, {}, { repo_extract: '', repo_diff: `() => (${JSON.stringify(fixture('diff_commits'))})` }, { repo_extract: slow })
    // To = newest version first (this enables From), then From = the one before it.
    await page.getByLabel('To').click()
    await page.getByRole('option').nth(1).click()
    await page.getByLabel('From').click()
    await page.getByRole('option').nth(1).click()
    await file(page, /hero_preview/).click()
    const bars = page.getByTestId('compare-extract')
    await expect(bars.getByRole('progressbar')).toHaveCount(2)
    await expect(bars.getByRole('progressbar').first()).toHaveAttribute('aria-valuenow', '40')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bars).toHaveCount(0)
    await expect(pane(page).getByRole('img', { name: /^Before/ })).toBeVisible()
  })

  test('an extraction failure is shown plainly', async ({ page }) => {
    await boot(page, {}, { repo_extract: `() => { throw { code: 'error', message: 'Blob is missing.' } }` })
    await file(page, /hero_preview/).click()
    await expect(pane(page).getByRole('alert')).toContainText('Could not extract a side of this file: Blob is missing.')
    await expectNoAxeViolations(page)
  })
})

test.describe('Compare: two versions', () => {
  test('choosing two versions asks for exactly those', async ({ page }) => {
    await boot(page)
    await page.getByLabel('To').click()
    await page.getByRole('option', { name: /0001000 · Fix audio sync/ }).click()
    await expect(page.getByLabel('From')).toBeEnabled()
    // From starts at the newest version; move it to an older one.
    await page.getByLabel('From').click()
    await page.getByRole('option', { name: /0002000 · Final grade/ }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_diff')).at(-1)?.args).toEqual({
      repo: '/repos/hero-film',
      from: '0002000200020002000200020002000200020002',
      to: '0001000100010001000100010001000100010001',
    })
    await expect(page.getByRole('list', { name: 'Changed files' }).getByRole('listitem')).toHaveCount(4)
    await expectNoAxeViolations(page)
  })

  test('the same version on both sides is explained, not compared', async ({ page }) => {
    await boot(page)
    await page.getByLabel('To').click()
    await page.getByRole('option', { name: /0000000 · Merge grade/ }).click()
    await expect(page.getByText('Pick two different versions')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('file buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await file(page, /hero_preview/).focus()
    await expectFocusRing(page)
    await page.keyboard.press('Tab')
    await expect(file(page, /new_concept/)).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(pane(page)).toContainText('art/new_concept.png')
  })
})
