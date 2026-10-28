import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const card = (page: Page, id: string) => page.getByTestId(id)

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({ config_list: fixture('config_list'), sparse_list: fixture('sparse_off'), config_set: fixture('config_list'), config_unset: fixture('config_list'), ...commands }),
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto('/#/settings', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
}
const root = (page: Page, attr: string) => page.evaluate((a) => document.documentElement.getAttribute(a), attr)

test.describe('Settings: states', () => {
  test('populated: appearance, author from config, set values listed, sparse off; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(card(page, 'identity').getByLabel('Name')).toHaveValue('Mira Okafor')
    await expect(card(page, 'identity').getByLabel('Email')).toHaveValue('mira@studio.example')
    const rows = card(page, 'config').getByRole('row')
    await expect(rows).toHaveCount(4) // header + 3 values set; an unset key is not listed
    await expect(card(page, 'config')).toContainText('performance.upload_concurrency')
    await expect(card(page, 'config')).not.toContainText('performance.download_concurrency')
    await expect(card(page, 'sparse')).toContainText('Off: every file is on disk')
    await expect(card(page, 'sparse').getByRole('button', { name: 'Keep every file…' })).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton while settings are read', async ({ page }) => {
    await boot(page, {}, { config_list: [{ hold: true }, { resolve: fixture('config_list') }] })
    await expect(card(page, 'identity').getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(card(page, 'identity').getByLabel('Name')).toHaveValue('Mira Okafor')
  })

  test('empty: nothing changed from the defaults', async ({ page }) => {
    await boot(page, { config_list: fixture('config_empty') })
    await expect(card(page, 'config')).toContainText('Nothing has been changed from the defaults.')
    await expect(card(page, 'identity').getByLabel('Name')).toHaveValue('')
    await expect(card(page, 'identity').getByRole('button', { name: 'Save' })).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('error: settings cannot be read, Retry asks again', async ({ page }) => {
    await boot(page, { config_list: { __error: { code: 'error', message: 'Cannot read the configuration.' } } })
    await expect(card(page, 'identity').getByRole('alert')).toContainText('Cannot read the configuration')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'config_list')).length
    await card(page, 'identity').getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'config_list')).length).toBeGreaterThan(before)
  })
})

test.describe('Settings: appearance', () => {
  test('theme override applies at once, persists, and "Match system" returns control to the OS', async ({ page }) => {
    await boot(page)
    expect(await root(page, 'data-theme')).toBeNull()
    await card(page, 'appearance').getByRole('button', { name: 'Dark' }).click()
    expect(await root(page, 'data-theme')).toBe('dark')
    expect(await page.evaluate(() => localStorage.getItem('mg.theme'))).toBe('"dark"')
    await expect(page.locator('body')).toHaveCSS('background-color', /rgb\(\d+, \d+, \d+\)/)
    await expectNoAxeViolations(page)
    await page.reload()
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    expect(await root(page, 'data-theme')).toBe('dark')
    await card(page, 'appearance').getByRole('button', { name: 'Light' }).click()
    expect(await root(page, 'data-theme')).toBe('light')
    await expectNoAxeViolations(page)
    await card(page, 'appearance').getByRole('button', { name: 'Match system' }).click()
    expect(await root(page, 'data-theme')).toBeNull()
  })

  test('compact density makes rows 28px and comfortable returns them to 36px', async ({ page }) => {
    await boot(page)
    const rowH = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--row-h').trim())
    expect(await rowH()).toBe('36px')
    await card(page, 'appearance').getByRole('button', { name: 'Compact' }).click()
    expect(await root(page, 'data-density')).toBe('compact')
    expect(await rowH()).toBe('28px')
    expect(await page.evaluate(() => localStorage.getItem('mg.density'))).toBe('"compact"')
    await expectNoAxeViolations(page)
    await card(page, 'appearance').getByRole('button', { name: 'Comfortable' }).click()
    expect(await root(page, 'data-density')).toBeNull()
    expect(await rowH()).toBe('36px')
  })
})

test.describe('Settings: identity and config', () => {
  test('author: validates the email, then saves both keys', async ({ page }) => {
    await boot(page)
    const id = card(page, 'identity')
    const save = id.getByRole('button', { name: 'Save' })
    await expect(save).toBeDisabled()
    await id.getByLabel('Email').fill('not-an-email')
    await expect(id).toContainText('Use an address like you@example.com')
    await expect(save).toBeDisabled()
    await expectNoAxeViolations(page)
    await id.getByLabel('Name').fill('Mira O.')
    await id.getByLabel('Email').fill('mira.o@studio.example')
    await save.click()
    await expect.poll(async () => (await ipcCalls(page, 'config_set')).map((c) => [c.args.key, c.args.value])).toEqual([
      ['author.name', 'Mira O.'],
      ['author.email', 'mira.o@studio.example'],
    ])
    await expect.poll(async () => (await ipcCalls(page, 'config_list')).length).toBeGreaterThan(1)
  })

  test('a failed save is shown plainly and dismissable', async ({ page }) => {
    await boot(page, { config_set: { __error: { code: 'error', message: 'author.email must look like an address.' } } })
    await card(page, 'identity').getByLabel('Name').fill('Someone Else')
    await card(page, 'identity').getByRole('button', { name: 'Save' }).click()
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('author.email must look like an address.')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
  })

  test('Reset puts one setting back to its default', async ({ page }) => {
    await boot(page)
    await card(page, 'config').getByRole('button', { name: 'Reset performance.upload_concurrency' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'config_unset')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', key: 'performance.upload_concurrency' },
    ])
  })
})

test.describe('Settings: sparse checkout', () => {
  test('on: patterns are listed in the editor; applying asks first and sends them as folders', async ({ page }) => {
    await boot(page, { sparse_list: fixture('sparse_on'), sparse_set: fixture('sparse_set') })
    const sp = card(page, 'sparse')
    await expect(sp).toContainText('On, by folder')
    await expect(sp.getByLabel('Folders or patterns')).toHaveValue('art\naudio')
    await expectNoAxeViolations(page)
    await sp.getByLabel('Folders or patterns').fill('art\nvideo\n')
    await sp.getByRole('button', { name: 'Apply…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('removed from this computer')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'sparse_set')).toHaveLength(0)
    await sp.getByRole('button', { name: 'Apply…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Apply' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sparse_set')).map((c) => c.args)).toMatchObject([
      { repo: '/repos/hero-film', patterns: ['art', 'video'], patternMode: false },
    ])
    await expect(page.getByText('12 files added, 340 removed').first()).toBeVisible()
  })

  test('pattern mode is sent when ticked', async ({ page }) => {
    await boot(page, { sparse_set: fixture('sparse_set') })
    const sp = card(page, 'sparse')
    await sp.getByLabel('Folders or patterns').fill('*.psd')
    await sp.getByRole('checkbox', { name: 'Treat lines as patterns, not folders' }).click()
    await sp.getByRole('button', { name: 'Apply…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Apply' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sparse_set')).map((c) => c.args.patternMode)).toEqual([true])
  })

  test('invalid lines are refused with a reason', async ({ page }) => {
    await boot(page)
    const sp = card(page, 'sparse')
    await expect(sp.getByRole('button', { name: 'Apply…' })).toBeDisabled()
    await sp.getByLabel('Folders or patterns').fill('../outside')
    await expect(sp).toContainText('Patterns cannot start with a dash')
    await expect(sp.getByRole('button', { name: 'Apply…' })).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('busy: a files bar while the folder is rewritten', async ({ page }) => {
    await boot(page, { sparse_list: fixture('sparse_on') }, { sparse_set: fixture('sparse_progress') })
    const sp = card(page, 'sparse')
    await sp.getByRole('button', { name: 'Apply…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Apply' }).click()
    const bar = page.getByTestId('sparse-progress')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '352')
    await expect(bar).toContainText('0 / 352 files')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bar).toContainText('100 / 352 files')
    await ipcRelease(page)
    await expect(bar).toHaveCount(0)
  })

  test('keep every file asks first, then disables sparse checkout', async ({ page }) => {
    await boot(page, { sparse_list: fixture('sparse_on'), sparse_disable: fixture('sparse_off') })
    await card(page, 'sparse').getByRole('button', { name: 'Keep every file…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Every file in the project is brought back')
    await dlg.getByRole('button', { name: 'Bring everything back' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'sparse_disable')).length).toBe(1)
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await card(page, 'identity').getByLabel('Name').focus()
    await expectFocusRing(page)
  })
})
