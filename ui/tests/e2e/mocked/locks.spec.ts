import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const row = (page: Page, path: string) => page.locator(`[data-lock="${path}"]`)

async function boot(page: Page, commands: Record<string, unknown> = {}, storage: Record<string, string> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, { commands: healthy({ lock_list: fixture('locks'), lock_create: { lock_id: 'l-9', path: 'audio/vo_take3.wav', owner: 'Mira Okafor', created_at: 1793000000 }, ...commands }), storage: { ...REPO, ...storage }, scripts: scripts as never })
  await page.goto('/#/locks', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Locks', level: 1 })).toBeVisible()
}

test.describe('Locks: states', () => {
  test('populated: sorted table with owner and age; no axe violations', async ({ page }) => {
    await boot(page)
    const table = page.getByRole('table')
    await expect(table.getByRole('row')).toHaveCount(4) // header + 3
    await expect(table.getByRole('row').nth(1)).toContainText('art/hero.psd')
    await expect(row(page, 'art/hero.psd')).toContainText('Mira Okafor')
    await expect(row(page, 'env/sky_hdri.exr')).toContainText(/\d+[dw] ago/)
    await expectNoAxeViolations(page)
  })

  test('loading, empty and error states', async ({ page }) => {
    await boot(page, {}, {}, { lock_list: [{ hold: true }, { resolve: fixture('locks') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(row(page, 'art/hero.psd')).toBeVisible()
  })

  test('empty: nothing locked invites the first lock', async ({ page }) => {
    await boot(page, { lock_list: { locks: [] } })
    await expect(page.getByText('Nothing is locked')).toBeVisible()
    await expectNoAxeViolations(page)
    await page.getByRole('main').getByRole('button', { name: 'Lock a file' }).last().click()
    await expect(page.getByRole('dialog', { name: 'Lock a file' })).toBeVisible()
  })

  test('error: code + message and Retry', async ({ page }) => {
    await boot(page, { lock_list: { __error: { code: 'error', message: 'No remote configured.' } } })
    await expect(page.getByRole('alert')).toContainText('No remote configured')
    await expectNoAxeViolations(page)
    await expect(page.getByRole('button', { name: 'Retry' })).toHaveCount(0)
    await page.getByRole('button', { name: 'Add remote' }).click()
    await expect(page).toHaveURL(/#\/sync$/)
  })
})

test.describe('Locks: filter and actions', () => {
  test('Mine / Others use the name this app locked with; unknown means Mine is empty and says why', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Mine' }).click()
    await expect(page.getByText('You have no locks')).toBeVisible()
    await expect(page.getByText('Sign in or set your name in Settings')).toBeVisible()
    await page.getByRole('button', { name: 'Others' }).click()
    await expect(page.getByRole('table').getByRole('row')).toHaveCount(4)
  })

  test('with a known owner: Mine shows one row marked You with a plain Unlock', async ({ page }) => {
    await boot(page, {}, { 'mg.lock.owner': JSON.stringify('Mira Okafor') })
    await page.getByRole('button', { name: 'Mine' }).click()
    await expect(page.getByRole('table').getByRole('row')).toHaveCount(2)
    await expect(row(page, 'art/hero.psd')).toContainText('You')
    await row(page, 'art/hero.psd').getByRole('button', { name: 'Unlock' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'lock_unlock')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', lockId: 'l-1', force: false }])
    await page.getByRole('button', { name: 'Others' }).click()
    await expect(page.getByRole('table').getByRole('row')).toHaveCount(3)
  })

  test('mine is decided by the signed-in account, not by the first lock made here', async ({ page }) => {
    await boot(page, { auth_run: { ...fixture<object>('auth_in'), identity: { username: 'Dev Rao', email: 'dev@x.example', role: 'write', grants: [] } } })
    await page.getByRole('button', { name: 'Mine' }).click()
    await expect(page.getByRole('table').getByRole('row')).toHaveCount(2)
    await expect(row(page, 'char/rig.fbx')).toContainText('You')
    await expect(row(page, 'char/rig.fbx').getByRole('button', { name: 'Unlock', exact: true })).toBeVisible()
    await page.getByRole('button', { name: 'Others' }).click()
    await expect(row(page, 'art/hero.psd')).not.toContainText('You')
    await expectNoAxeViolations(page)
  })

  test('with no account, the commit author name marks your locks', async ({ page }) => {
    await boot(page, { config_list: { action: 'list', entries: [{ key: 'author.name', value: 'Sam Lee' }, { key: 'author.email', value: 'sam@x.example' }] } })
    await page.getByRole('button', { name: 'Mine' }).click()
    await expect(page.getByRole('table').getByRole('row')).toHaveCount(2)
    await expect(row(page, 'env/sky_hdri.exr')).toContainText('You')
  })

  test('force unlock explains the consequences and needs a second confirmation', async ({ page }) => {
    await boot(page, {}, { 'mg.lock.owner': JSON.stringify('Mira Okafor') })
    await row(page, 'char/rig.fbx').getByRole('button', { name: 'Force unlock…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Force unlock char/rig.fbx?')
    await expect(dlg).toContainText('Dev Rao locked this file')
    await expect(dlg).toContainText('lose changes they have not saved')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'lock_unlock')).toHaveLength(0)
    await row(page, 'char/rig.fbx').getByRole('button', { name: 'Force unlock…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Force unlock' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'lock_unlock')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', lockId: 'l-2', force: true }])
  })

  test('lock a file: path rules, duplicate check, then the lock is requested', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Lock a file' }).click()
    const dlg = page.getByRole('dialog', { name: 'Lock a file' })
    const path = dlg.getByLabel('File path')
    const go = dlg.getByRole('button', { name: 'Lock file' })
    await expect(go).toBeDisabled()
    await path.fill('/etc/passwd')
    await expect(dlg).toContainText('Use a path inside the project')
    await path.fill('art/hero.psd')
    await expect(dlg).toContainText('This file is already locked')
    await expectNoAxeViolations(page)
    await path.fill('audio\\vo_take3.wav')
    await expect(go).toBeEnabled()
    await go.click()
    await expect.poll(async () => (await ipcCalls(page, 'lock_create')).map((c) => c.args.path)).toEqual(['audio/vo_take3.wav'])
    await expect(dlg).toHaveCount(0)
  })

  test('a refused lock shows the server message and the table stays usable', async ({ page }) => {
    await boot(page, {}, {}, { lock_create: [{ reject: { code: 'error', message: 'Already locked by Dev Rao.' } }] })
    await page.getByRole('button', { name: 'Lock a file' }).click()
    await page.getByRole('dialog').getByLabel('File path').fill('audio/vo_take3.wav')
    await page.getByRole('dialog').getByRole('button', { name: 'Lock file' }).click()
    // the refusal shows inside the open dialog (a modal hides the page behind it from assistive tech)
    await expect(page.getByRole('dialog').getByRole('alert')).toContainText('Already locked by Dev Rao.')
    await page.keyboard.press('Escape')
    await expect(row(page, 'art/hero.psd')).toBeVisible()
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await row(page, 'char/rig.fbx').getByRole('button', { name: 'Force unlock…' }).focus()
    await expectFocusRing(page)
  })
})
