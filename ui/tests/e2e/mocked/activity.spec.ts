import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const pill = (page: Page) => page.locator('[data-activity-pill]')
// The rail's quiet Activity button is the way in when nothing is running.
const toggle = (page: Page) => page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Activity/ })

async function boot(page: Page, scripts = {}) {
  await mockIpc(page, { commands: healthy(), storage: REPO, scripts })
  await page.goto('/#/changes')
  await expect(page.getByRole('heading', { name: 'Changes', level: 2 })).toBeVisible()
}

test.describe('Activity bar and drawer', () => {
  test('idle: no pill; the drawer still opens from the rail and explains the empty list', async ({ page }) => {
    await boot(page)
    await expect(pill(page)).toHaveCount(0)
    await toggle(page).click()
    const drawer = page.getByTestId('activity-drawer')
    await expect(drawer).toContainText('Nothing has run yet')
    await expectNoAxeViolations(page)
  })

  test('drawer opens from the bar, closes with Escape and with Close; focus ring is visible', async ({ page }) => {
    await boot(page)
    await toggle(page).focus()
    await expectFocusRing(page)
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('activity-drawer')).toBeVisible()
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'true')
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('activity-drawer')).toHaveCount(0)
    await toggle(page).click()
    await page.getByTestId('activity-drawer').getByRole('button', { name: 'Close' }).click()
    await expect(page.getByTestId('activity-drawer')).toHaveCount(0)
  })

  test('a running op shows in the header pill after the anti-flicker delay; finish lists it and toasts success', async ({ page }) => {
    await boot(page, { repo_add: [{ hold: true }, { resolve: null }] })
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    await expect(pill(page)).toContainText('Staging 6 files')
    // Local ops have no cancel; only push/pull/fetch/clone do.
    await pill(page).click()
    const drawer = page.getByTestId('activity-drawer')
    await expect(drawer.getByRole('heading', { name: 'Running' })).toBeVisible()
    await expect(drawer.getByRole('button', { name: /Cancel/ })).toHaveCount(0)
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(drawer.getByRole('heading', { name: 'Finished' })).toBeVisible()
    await expect(drawer).toContainText('Staged 6 files')
    await expect(page.getByText('Staged 6 files').last()).toBeVisible()
  })

  test('a failed op appears under Failed with code + message and Retry that re-runs it', async ({ page }) => {
    await boot(page, { repo_add: [{ reject: { code: 'E_LOCKED', message: 'Another save is running.' } }] })
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    await toggle(page).click()
    const drawer = page.getByTestId('activity-drawer')
    await expect(drawer.getByRole('heading', { name: 'Failed' })).toBeVisible()
    await expect(drawer).toContainText('E_LOCKED')
    await expect(drawer).toContainText('Another save is running.')
    await expectNoAxeViolations(page)
    await drawer.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_add')).length).toBe(2)
  })

  test('clone in progress: the pill shows the op; the drawer row shows -- until two samples are 1 s apart and offers Cancel', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: REPO, scripts: { repo_clone: fixture('clone_progress') } })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /Clone/ }).click()
    await page.getByLabel('Repository URL').fill('https://studio.example/hero-film')
    await page.getByLabel('Save into').fill('/repos/hero-film')
    await page.getByRole('dialog').getByRole('button', { name: 'Clone', exact: true }).click()
    await ipcRelease(page)
    // The modal clone dialog hides the page from the accessibility tree, so match by CSS.
    await expect(pill(page)).toContainText('Clone hero-film')
    await expect(pill(page).locator('[role="progressbar"]')).toHaveAttribute('aria-valuenow', '100000')
    // The modal dialog blocks pointer input; the pill's own click handler is what is under test.
    await pill(page).dispatchEvent('click')
    const drawer = page.getByTestId('activity-drawer')
    await expect(drawer).toContainText('Downloading media')
    await expect(drawer).toContainText('ETA --')
    await expect(drawer.locator('button', { hasText: 'Cancel' })).toBeVisible()
  })
})
