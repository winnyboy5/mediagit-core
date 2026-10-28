import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const PW = 'S3cret-pw-Zebra!'
const CODE = 'RC-7QK2-M9XD-4TVB-HE8A'

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, {
    commands: healthy({ auth_run: fixture('auth_in'), key_status: fixture('key_on'), ...commands }),
    scripts: scripts as never,
    storage: REPO,
  })
  await page.goto('/#/security', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Security', level: 1 })).toBeVisible()
}

const account = (page: Page) => page.getByTestId('account')
const enc = (page: Page) => page.getByTestId('encryption')

/** Nothing secret is on the page, in browser storage or in the activity list. */
async function expectNoTrace(page: Page, ...secrets: string[]) {
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: 'Activity' }).click()
  const seen = await page.evaluate(() => document.body.innerText + JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }))
  for (const s of secrets) expect(seen).not.toContain(s)
}

test.describe('Security: states', () => {
  test('signed in and encrypted: identity, grants, key source; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(account(page)).toContainText('Signed in as mira')
    await expect(account(page)).toContainText('mira@studio.example')
    await expect(account(page).getByRole('list', { name: 'Projects you can reach' })).toContainText('trailer-cut')
    await expect(enc(page)).toContainText('This repository is encrypted.')
    await expect(enc(page)).toContainText('OS keychain')
    await expect(enc(page)).toContainText('Set up')
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton until both answers arrive', async ({ page }) => {
    await boot(page, {}, { auth_run: [{ hold: true }, { resolve: fixture('auth_in') }], key_status: [{ hold: true }, { resolve: fixture('key_on') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(account(page)).toContainText('Signed in as mira')
  })

  test('signed out: sign in offered', async ({ page }) => {
    await boot(page, { auth_run: fixture('auth_out') })
    await expect(account(page)).toContainText('You are not signed in to https://mediagit.studio.example.')
    await expect(account(page).getByRole('button', { name: 'Sign in' })).toBeEnabled()
    await expectNoAxeViolations(page)
  })

  test('a server without sign-in says so', async ({ page }) => {
    await boot(page, { auth_run: fixture('auth_open') })
    await expect(account(page)).toContainText('does not ask for sign-in')
    await expectNoAxeViolations(page)
  })

  test('error: the account cannot be read, the key card still works; Retry asks again', async ({ page }) => {
    await boot(page, { auth_run: { __error: { code: 'error', message: 'The server did not answer.' } } })
    await expect(account(page).getByRole('alert')).toContainText('The server did not answer')
    await expect(enc(page)).toContainText('This repository is encrypted.')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'auth_run')).length
    await account(page).getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'auth_run')).length).toBeGreaterThan(before)
  })

  test('not encrypted: turn on offered', async ({ page }) => {
    await boot(page, { key_status: fixture('key_off') })
    await expect(enc(page)).toContainText('stored as they are')
    await expect(enc(page).getByRole('button', { name: 'Turn on encryption…' })).toBeEnabled()
    await expectNoAxeViolations(page)
  })

  test('encrypted but no key on this machine: says so and offers recovery', async ({ page }) => {
    await boot(page, { key_status: fixture('key_locked') })
    await expect(enc(page)).toContainText('No key available on this machine')
    await expect(enc(page)).toContainText('cannot be read here')
    await expectNoAxeViolations(page)
  })
})

test.describe('Security: sign in and out', () => {
  test('sign in validates the server, hands the password over once and clears the field', async ({ page }) => {
    await boot(page, { auth_login: fixture('auth_in'), auth_run: fixture('auth_out') })
    await account(page).getByRole('button', { name: 'Sign in' }).click()
    const dlg = page.getByRole('dialog', { name: 'Sign in' })
    const pw = dlg.getByLabel('Password')
    await expect(pw).toHaveAttribute('type', 'password')
    await expect(dlg.getByRole('button', { name: 'Sign in' })).toBeDisabled()
    await dlg.getByLabel('Server address (optional)').fill('ftp://nope')
    await expect(dlg).toContainText('Use an http(s) address')
    await dlg.getByLabel('Server address (optional)').fill('')
    await dlg.getByLabel('Username or email').fill('mira')
    await pw.fill(PW)
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Sign in' }).click()
    await expect(dlg).toHaveCount(0)
    await expect.poll(async () => (await ipcCalls(page, 'auth_login')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', server: null, username: 'mira', password: PW },
    ])
    await account(page).getByRole('button', { name: 'Sign in' }).click()
    await expect(page.getByRole('dialog').getByLabel('Password')).toHaveValue('')
    await expectNoTrace(page, PW)
  })

  test('a failed sign-in shows the reason, keeps no password and offers no Retry', async ({ page }) => {
    await boot(page, { auth_run: fixture('auth_out'), auth_login: { __error: { code: 'error', message: 'Wrong username or password.' } } })
    await account(page).getByRole('button', { name: 'Sign in' }).click()
    const dlg = page.getByRole('dialog', { name: 'Sign in' })
    await dlg.getByLabel('Password').fill(PW)
    await dlg.getByRole('button', { name: 'Sign in' }).click()
    await expect(page.getByRole('alert')).toContainText('Wrong username or password.')
    await expectNoAxeViolations(page)
    await expectNoTrace(page, PW)
    await expect(page.getByRole('button', { name: 'Retry' })).toHaveCount(0)
  })

  test('sign out asks first, then signs out', async ({ page }) => {
    await boot(page, { auth_run: fixture('auth_in') })
    await account(page).getByRole('button', { name: 'Sign out…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Sign out?')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect((await ipcCalls(page, 'auth_run')).filter((c) => c.args.action === 'logout')).toHaveLength(0)
    await account(page).getByRole('button', { name: 'Sign out…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Sign out', exact: true }).click()
    await expect.poll(async () => (await ipcCalls(page, 'auth_run')).filter((c) => c.args.action === 'logout').map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', action: 'logout', server: null, all: false },
    ])
  })
})

test.describe('Security: encryption', () => {
  test('turn on: shows the recovery code once, in a dialog that only "I saved it" closes', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    await boot(page, { key_status: fixture('key_off'), key_init: fixture('key_init') })
    await enc(page).getByRole('button', { name: 'Turn on encryption…' }).click()
    const init = page.getByRole('dialog', { name: 'Turn on encryption' })
    await expect(init.getByLabel('Passphrase (optional)')).toHaveAttribute('type', 'password')
    await expectNoAxeViolations(page)
    await init.getByRole('button', { name: 'Turn on encryption' }).click()

    const dlg = page.getByRole('dialog', { name: 'Save your recovery code' })
    await expect(dlg.getByTestId('recovery-code')).toHaveText(CODE)
    await expect(dlg.getByRole('button', { name: 'Done' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await page.keyboard.press('Escape')
    await expect(dlg).toBeVisible()
    await dlg.getByRole('button', { name: 'Copy code' }).click()
    await expect(dlg.getByRole('button', { name: 'Copied' })).toBeVisible()
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(CODE)
    await dlg.getByRole('checkbox', { name: 'I saved it' }).click()
    await dlg.getByRole('button', { name: 'Done' }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
    expect((await ipcCalls(page, 'key_init')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', passphrase: null }])
    await expectNoTrace(page, CODE)
  })

  test('turn on with a passphrase passes it on and clears the field', async ({ page }) => {
    await boot(page, { key_status: fixture('key_off'), key_init: fixture('key_init') })
    await enc(page).getByRole('button', { name: 'Turn on encryption…' }).click()
    const init = page.getByRole('dialog', { name: 'Turn on encryption' })
    await init.getByLabel('Passphrase (optional)').fill(PW)
    await init.getByRole('button', { name: 'Turn on encryption' }).click()
    await expect(page.getByTestId('recovery-code')).toBeVisible()
    expect((await ipcCalls(page, 'key_init')).map((c) => c.args.passphrase)).toEqual([PW])
    await page.getByRole('checkbox', { name: 'I saved it' }).click()
    await page.getByRole('button', { name: 'Done' }).click()
    await expectNoTrace(page, PW, CODE)
  })

  test('a failed turn on shows the reason and no recovery dialog', async ({ page }) => {
    await boot(page, { key_status: fixture('key_off'), key_init: { __error: { code: 'error', message: 'Encryption is already set up.' } } })
    await enc(page).getByRole('button', { name: 'Turn on encryption…' }).click()
    await page.getByRole('dialog', { name: 'Turn on encryption' }).getByRole('button', { name: 'Turn on encryption' }).click()
    await expect(page.getByRole('alert')).toContainText('Encryption is already set up.')
    await expect(page.getByTestId('recovery-code')).toHaveCount(0)
    await expectNoAxeViolations(page)
  })

  test('recover: the code is a password field, goes on once, and the fields clear', async ({ page }) => {
    await boot(page, { key_status: fixture('key_locked'), key_recover: { ...fixture<object>('key_on'), action: 'recover' } })
    await enc(page).getByRole('button', { name: 'Recover with a code…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Recover the key' })
    await expect(dlg.getByLabel('Recovery code')).toHaveAttribute('type', 'password')
    await expect(dlg.getByRole('button', { name: 'Recover' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Recovery code').fill(CODE)
    await dlg.getByRole('button', { name: 'Recover' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'key_recover')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', code: CODE, passphrase: null }])
    await enc(page).getByRole('button', { name: 'Recover with a code…' }).click()
    await expect(page.getByRole('dialog').getByLabel('Recovery code')).toHaveValue('')
    await expectNoTrace(page, CODE)
  })

  test('cancelling a dialog throws away what was typed', async ({ page }) => {
    await boot(page, { auth_run: fixture('auth_out') })
    await account(page).getByRole('button', { name: 'Sign in' }).click()
    await page.getByRole('dialog').getByLabel('Password').fill(PW)
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
    await account(page).getByRole('button', { name: 'Sign in' }).click()
    await expect(page.getByRole('dialog').getByLabel('Password')).toHaveValue('')
    expect(await ipcCalls(page, 'auth_login')).toHaveLength(0)
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page, { key_status: fixture('key_off') })
    await enc(page).getByRole('button', { name: 'Turn on encryption…' }).focus()
    await expectFocusRing(page)
  })
})
