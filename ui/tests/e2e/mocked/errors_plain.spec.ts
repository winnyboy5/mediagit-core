import { expect, test, type Page } from '@playwright/test'
import { healthy, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const RAW_STATUS = 'POST /admin/repos failed with status: 400 Bad Request'

async function boot(page: Page, route: string, heading: string, commands: Record<string, unknown>) {
  await mockIpc(page, { commands: healthy(commands), storage: REPO })
  await page.goto(route, { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: heading, level: 2 })).toBeVisible()
}

/** No visible text on the page is a raw code or an HTTP status line. */
async function expectPlain(page: Page, ...codes: string[]) {
  const text = await page.locator('body').innerText()
  for (const c of codes) expect(text).not.toContain(c)
  expect(text).not.toMatch(/failed with status|\b[45]\d\d (Bad Request|Not Found|Internal Server Error|Conflict)/)
}

test.describe('errors are plain words', () => {
  test('an error page shows a sentence, hides the code, and keeps raw text under Details', async ({ page }) => {
    await boot(page, '/#/branches', 'Branches', {
      branch_list: { __error: { code: 'server_error', message: 'The server hit a problem. Try again in a moment.', details: RAW_STATUS } },
    })
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('The server hit a problem')
    await expectPlain(page, 'server_error', 'empty_commit')
    await expect(alert.getByText('Details', { exact: true })).toBeVisible()
    await expect(alert.locator('pre')).toBeHidden()
    await alert.getByText('Details', { exact: true }).click()
    await expect(alert.locator('pre')).toContainText('failed with status')
  })

  test('an unmapped code with a raw status line shows a generic sentence', async ({ page }) => {
    await boot(page, '/#/branches', 'Branches', { branch_list: { __error: { code: 'E_WEIRD', message: RAW_STATUS } } })
    await expect(page.getByRole('alert')).toContainText('Something went wrong')
    await expectPlain(page, 'E_WEIRD')
  })

  test('a 401 with a server reason shows the reason and the sign-in action', async ({ page }) => {
    await boot(page, '/#/branches', 'Branches', {
      branch_list: { __error: { code: 'not_signed_in', message: 'Invalid credentials. Sign in and try again (mediagit auth login <server>).' } },
    })
    await expect(page.getByRole('alert').getByRole('heading')).toHaveText('Invalid credentials')
    await expect(page.getByRole('alert').locator('p')).toHaveText('Sign in and try again.')
    await expectPlain(page, 'not_signed_in', 'mediagit auth login')
  })
})
