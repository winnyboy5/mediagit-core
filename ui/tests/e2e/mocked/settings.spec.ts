import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const card = (page: Page, id: string) => page.getByTestId(id)

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({ config_list: fixture('config_list'), config_schema: fixture('config_schema'), sparse_list: fixture('sparse_off'), config_set: fixture('config_list'), config_unset: fixture('config_list'), ...commands }),
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto('/#/settings', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'General', level: 2 })).toBeVisible()
}
const root = (page: Page, attr: string) => page.evaluate((a) => document.documentElement.getAttribute(a), attr)

test.describe('Settings: states', () => {
  test('populated: appearance, author from config, set values listed, sparse off; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(card(page, 'identity').getByLabel('Name')).toHaveValue('Mira Okafor')
    await expect(card(page, 'identity').getByLabel('Email')).toHaveValue('mira@studio.example')
    // Every schema key is a field; a set one shows its value, an unset one is empty (default shown as hint).
    await expect(card(page, 'config').getByLabel('Files sent at once')).toHaveValue('8')
    await expect(card(page, 'config').getByLabel('Files received at once')).toHaveValue('')
    await expect(card(page, 'config')).toContainText('performance.download_concurrency')
    await expect(card(page, 'config')).not.toContainText('author.name')
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
    await expect(card(page, 'config').getByLabel('Files sent at once')).toHaveValue('')
    await expect(card(page, 'config').getByRole('button', { name: 'Reset performance.upload_concurrency' })).toBeDisabled()
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
    await expect(page.getByRole('heading', { name: 'General', level: 2 })).toBeVisible()
    expect(await root(page, 'data-theme')).toBe('dark')
    await card(page, 'appearance').getByRole('button', { name: 'Light' }).click()
    expect(await root(page, 'data-theme')).toBe('light')
    await expectNoAxeViolations(page)
    await card(page, 'appearance').getByRole('button', { name: 'Match system' }).click()
    expect(await root(page, 'data-theme')).toBeNull()
  })

  test('compact density makes rows 28px and comfortable returns them to 40px', async ({ page }) => {
    await boot(page)
    const rowH = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--row-h').trim())
    expect(await rowH()).toBe('40px')
    await card(page, 'appearance').getByRole('button', { name: 'Compact' }).click()
    expect(await root(page, 'data-density')).toBe('compact')
    expect(await rowH()).toBe('28px')
    expect(await page.evaluate(() => localStorage.getItem('mg.density'))).toBe('"compact"')
    await expectNoAxeViolations(page)
    await card(page, 'appearance').getByRole('button', { name: 'Comfortable' }).click()
    expect(await root(page, 'data-density')).toBeNull()
    expect(await rowH()).toBe('40px')
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

test.describe('Settings: every setting, from the CLI schema', () => {
  const ENTRIES = [
    ...(fixture<{ entries: unknown[] }>('config_list').entries ?? []),
    { key: 'storage.create_dirs', value: null, secret: false },
    { key: 'storage.secret_access_key', value: 'set', secret: true },
    { key: 'remotes.origin.url', value: 'https://mediagit.studio.example/hero-film', secret: false },
    { key: 'remotes.origin.token', value: null, secret: true },
    { key: 'remotes.backup.url', value: null, secret: false },
  ]
  const list = { action: 'list', entries: ENTRIES }
  const cfg = (page: Page) => card(page, 'config')

  test('groups, one field per key, a field per remote, secrets never shown; no axe violations', async ({ page }) => {
    await boot(page, { config_list: list })
    await expect(cfg(page).getByRole('heading', { name: 'Speed' })).toBeVisible()
    await expect(cfg(page).getByRole('heading', { name: 'Storage' })).toBeVisible()
    await expect(cfg(page).getByRole('heading', { name: 'Remote servers' })).toBeVisible()
    await expect(cfg(page).getByLabel('origin: Server address')).toHaveValue('https://mediagit.studio.example/hero-film')
    await expect(cfg(page).locator('[data-key="remotes.backup.url"]')).toBeVisible()
    await expect(cfg(page).getByRole('checkbox', { name: 'Create missing folders' })).toBeChecked() // unset: the default is true
    const secret = cfg(page).getByLabel('S3 secret key')
    await expect(secret).toHaveValue('')
    await expect(secret).toHaveAttribute('type', 'password')
    await expect(cfg(page).locator('[data-key="storage.secret_access_key"]')).toContainText('set')
    await expectNoAxeViolations(page)
  })

  test('a whole number is checked before sending, and saved with its key', async ({ page }) => {
    await boot(page, { config_list: list })
    const row = cfg(page).locator('[data-key="performance.download_concurrency"]')
    await row.getByLabel('Files received at once').fill('0')
    await expect(row).toContainText('At least 1')
    await expect(row.getByRole('button', { name: 'Save' })).toBeDisabled()
    await row.getByLabel('Files received at once').fill('abc')
    await expect(row).toContainText('Use a whole number')
    await row.getByLabel('Files received at once').fill('16')
    await row.getByRole('button', { name: 'Save' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'config_set')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', key: 'performance.download_concurrency', value: '16', secret: false },
    ])
  })

  test('a switch and a path are saved by their type', async ({ page }) => {
    await boot(page, { config_list: list })
    const row = cfg(page).locator('[data-key="storage.create_dirs"]')
    await expect(row.getByRole('button', { name: 'Save' })).toBeDisabled()
    await row.getByRole('checkbox', { name: 'Create missing folders' }).click()
    await row.getByRole('button', { name: 'Save' }).click()
    const path = cfg(page).locator('[data-key="storage.credentials_path"]')
    await path.getByLabel('Google key file').fill('C:\\keys\\sa.json')
    await path.getByRole('button', { name: 'Save' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'config_set')).map((c) => [c.args.key, c.args.value, c.args.secret])).toEqual([
      ['storage.create_dirs', 'false', false],
      ['storage.credentials_path', 'C:\\keys\\sa.json', false],
    ])
  })

  test('a secret is sent privately, emptied at once, and never reaches the page or the activity list', async ({ page }) => {
    await boot(page, { config_list: list })
    const row = cfg(page).locator('[data-key="remotes.origin.token"]')
    await row.getByLabel('origin: Access token').fill('tok-SECRET-9f2')
    await row.getByRole('button', { name: 'Save' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'config_set')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', key: 'remotes.origin.token', value: 'tok-SECRET-9f2', secret: true },
    ])
    await expect(row.getByLabel('origin: Access token')).toHaveValue('')
    await page.getByRole('button', { name: 'Activity' }).click()
    const seen = await page.evaluate(() => document.body.innerText + JSON.stringify({ ...localStorage }))
    expect(seen).not.toContain('SECRET-9f2')
  })

  test('Reset on a set secret clears it; an unset key has nothing to reset', async ({ page }) => {
    await boot(page, { config_list: list })
    await cfg(page).getByRole('button', { name: 'Reset storage.secret_access_key' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'config_unset')).map((c) => c.args.key)).toEqual(['storage.secret_access_key'])
    await expect(cfg(page).getByRole('button', { name: 'Reset remotes.origin.token' })).toBeDisabled()
  })

  test('a refused value shows the CLI reason and keeps the field', async ({ page }) => {
    await boot(page, { config_list: list, config_set: { __error: { code: 'error', message: 'storage.file_permissions must be an octal string like 0644' } } })
    const row = cfg(page).locator('[data-key="storage.file_permissions"]')
    await row.getByLabel('Who can open stored files').fill('rwx')
    await row.getByRole('button', { name: 'Save' }).click()
    await expect(page.getByRole('alert')).toContainText('must be an octal string')
    await expect(row.getByLabel('Who can open stored files')).toHaveValue('rwx')
    await expectNoAxeViolations(page)
  })

  test('if the list of settings cannot be read, the form says so instead of showing a stale one', async ({ page }) => {
    await boot(page, { config_list: list, config_schema: { __error: { code: 'error', message: 'schema unavailable' } } })
    await expect(card(page, 'no-schema')).toBeVisible()
    await expectNoAxeViolations(page)
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
