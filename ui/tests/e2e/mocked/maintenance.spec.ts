import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const card = (page: Page, t: string) => page.locator(`[data-task="${t}"]`)

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}, handlers: Record<string, string> = {}) {
  await mockIpc(page, {
    commands: healthy({ repo_gc: fixture('gc_dry'), repo_fsck: fixture('fsck_ok'), repo_verify: fixture('fsck_ok'), ...commands }),
    scripts: scripts as never,
    handlers,
    storage: REPO,
  })
  await page.goto('/#/maintenance', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Maintenance', level: 1 })).toBeVisible()
}

test.describe('Maintenance: states', () => {
  test('idle: three tasks, none run yet; preview is the default for clean-up; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(card(page, 'gc')).toContainText('Never run')
    await expect(card(page, 'gc').getByRole('checkbox', { name: 'Preview only' })).toBeChecked()
    await expect(card(page, 'gc').getByRole('button', { name: 'Preview clean-up' })).toBeEnabled()
    await expect(card(page, 'fsck').getByRole('button', { name: 'Run integrity check' })).toBeEnabled()
    await expect(card(page, 'verify').getByRole('button', { name: 'Verify' })).toBeEnabled()
    await expectNoAxeViolations(page)
  })

  test('a preview reports what would be freed and changes nothing', async ({ page }) => {
    await boot(page)
    await card(page, 'gc').getByRole('button', { name: 'Preview clean-up' }).click()
    const res = page.getByTestId('gc-result')
    await expect(res).toContainText('Preview: this would free 1.2 GB')
    await expect(res).toContainText('5000')
    await expect(card(page, 'gc')).toContainText('Last run')
    await expectNoAxeViolations(page)
    expect((await ipcCalls(page, 'repo_gc')).map((c) => c.args)).toMatchObject([{ repo: '/repos/hero-film', dryRun: true, confirmed: false, passphrase: null }])
  })

  test('a real clean-up is gated by a confirm dialog and sent as confirmed', async ({ page }) => {
    await boot(page, { repo_gc: fixture('gc_done') })
    await card(page, 'gc').getByRole('checkbox', { name: 'Preview only' }).click()
    await card(page, 'gc').getByRole('button', { name: 'Clean up…' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Clean up storage?')
    await expect(dlg).toContainText('cannot be brought back')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'repo_gc')).toHaveLength(0)
    await card(page, 'gc').getByRole('button', { name: 'Clean up…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Clean up', exact: true }).click()
    await expect(page.getByTestId('gc-result')).toContainText('Freed 1.2 GB')
    expect((await ipcCalls(page, 'repo_gc')).map((c) => c.args)).toMatchObject([{ dryRun: false, confirmed: true }])
  })

  test('after a preview, "Clean up now" goes through the same confirm', async ({ page }) => {
    await boot(page)
    await card(page, 'gc').getByRole('button', { name: 'Preview clean-up' }).click()
    await page.getByRole('button', { name: 'Clean up now…' }).click()
    await expect(page.getByRole('alertdialog')).toContainText('Clean up storage?')
    expect(await ipcCalls(page, 'repo_gc')).toHaveLength(1)
  })

  test('a clean-up that hit problems lists them', async ({ page }) => {
    await boot(page, { repo_gc: fixture('gc_errors') })
    await card(page, 'gc').getByRole('checkbox', { name: 'Preview only' }).click()
    await card(page, 'gc').getByRole('button', { name: 'Clean up…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Clean up', exact: true }).click()
    await expect(page.getByRole('list', { name: 'Clean-up problems' })).toContainText('pack-07.pack: in use')
    await expectNoAxeViolations(page)
  })

  test('busy: determinate progress while running, buttons locked', async ({ page }) => {
    await boot(page, {}, { repo_gc: fixture('gc_progress') })
    await card(page, 'gc').getByRole('button', { name: 'Preview clean-up' }).click()
    const bar = page.getByTestId('gc-progress')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '5000')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0')
    await expect(card(page, 'fsck').getByRole('button', { name: 'Run integrity check' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '2500')
    await ipcRelease(page)
    await expect(bar).toHaveCount(0)
    await expect(page.getByTestId('gc-result')).toBeVisible()
  })

  test('integrity check: healthy result, deep option sent', async ({ page }) => {
    await boot(page)
    await card(page, 'fsck').getByRole('button', { name: 'Run integrity check' }).click()
    const res = page.getByTestId('fsck-result')
    await expect(res).toContainText('No problems found')
    await expect(res).toContainText('48213')
    await expectNoAxeViolations(page)
    await card(page, 'fsck').getByRole('checkbox', { name: 'Deep check (slow)' }).click()
    await card(page, 'fsck').getByRole('button', { name: 'Run integrity check' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_fsck')).map((c) => c.args.deep)).toEqual([false, true])
  })

  test('integrity check that finds damage lists it, plainly', async ({ page }) => {
    await boot(page, { repo_fsck: fixture('fsck_bad') })
    await card(page, 'fsck').getByRole('button', { name: 'Run integrity check' }).click()
    const res = page.getByTestId('fsck-result')
    await expect(res).toContainText('Problems found')
    await expect(page.getByRole('list', { name: 'Check integrity findings' })).toContainText('does not match its hash')
    await expectNoAxeViolations(page)
  })

  test('busy: the check shows its objects bar', async ({ page }) => {
    await boot(page, {}, { repo_fsck: fixture('fsck_progress') })
    await card(page, 'fsck').getByRole('button', { name: 'Run integrity check' }).click()
    await expect(page.getByTestId('fsck-progress').getByRole('progressbar')).toHaveAttribute('aria-valuemax', '48213')
    await ipcRelease(page)
    await ipcRelease(page)
    await expect(page.getByTestId('fsck-result')).toContainText('No problems found')
  })

  test('verify: quick option sent, result shown', async ({ page }) => {
    await boot(page)
    await card(page, 'verify').getByRole('checkbox', { name: 'Quick check' }).click()
    await card(page, 'verify').getByRole('button', { name: 'Verify' }).click()
    await expect(page.getByTestId('verify-result')).toContainText('No problems found')
    expect((await ipcCalls(page, 'repo_verify')).map((c) => c.args)).toMatchObject([{ quick: true }])
  })

  test('a failed run is shown plainly and dismissable', async ({ page }) => {
    await boot(page, { repo_verify: { __error: { code: 'error', message: 'Another operation holds the repository lock.' } } })
    await card(page, 'verify').getByRole('button', { name: 'Verify' }).click()
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('Another operation holds the repository lock.')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
  })

  test('an encrypted repository asks for the passphrase and the check runs again with it', async ({ page }) => {
    const h = `(a) => { if (!a.passphrase) throw { code: 'needs_input', message: 'passphrase needed' }; return ${JSON.stringify(fixture('fsck_ok'))} }`
    await boot(page, {}, {}, { repo_fsck: h })
    await card(page, 'fsck').getByRole('button', { name: 'Run integrity check' }).click()
    const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
    await dlg.getByLabel('Passphrase').fill('tr0ub4dor&3')
    await dlg.getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByTestId('fsck-result')).toContainText('No problems found')
    expect((await ipcCalls(page, 'repo_fsck')).map((c) => c.args.passphrase)).toEqual([null, 'tr0ub4dor&3'])
    // The activity list shows labels only, never the passphrase.
    await page.getByRole('button', { name: 'Activity' }).click()
    expect(await page.evaluate(() => document.body.innerText)).not.toContain('tr0ub4dor')
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await card(page, 'gc').getByRole('button', { name: 'Preview clean-up' }).focus()
    await expectFocusRing(page)
  })
})
