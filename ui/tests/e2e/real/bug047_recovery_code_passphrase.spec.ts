import { execFileSync } from 'node:child_process'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium, expect, test, type Browser, type Page } from '@playwright/test'
import { goTo } from './nav'
import { readState, type E2EState } from './setup'

// BUG-047, real app.
// On a new empty repository, turn encryption on with a passphrase. The "Save your recovery
// code" dialog is on screen from the first sample and the unlock prompt is held until the
// person ticks "I saved it" and closes it; then the prompt opens, unlocking works, and the
// card reads "Set up" (never "Saved").
test.describe('BUG-047: the recovery code shows with a passphrase', () => {
  let st: E2EState
  let browser: Browser
  let page: Page

  test.beforeAll(async () => {
    st = readState()
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${st.cdpPort}`)
    const ctx = browser.contexts()[0]
    if (!ctx) throw new Error('WebView2 exposed no browser context over CDP')
    page = ctx.pages()[0] ?? (await ctx.waitForEvent('page'))
    await page.waitForLoadState('domcontentloaded')
  })
  test.afterAll(async () => {
    await browser?.close().catch(() => {})
  })

  test('code dialog first, unlock prompt held until "I saved it"', async () => {
    const repo = mkdtempSync(join(tmpdir(), 'mg-bug047-'))
    execFileSync(st.cli, ['init'], { cwd: repo, env: { ...process.env, ...st.env }, encoding: 'utf8' })
    await page.evaluate(
      ([recent]) => {
        localStorage.removeItem('mg.repo')
        localStorage.setItem('mg.recent', recent!)
      },
      [JSON.stringify([{ path: repo, name: 'bug047', openedAt: Date.now(), branch: 'main' }])],
    )
    await page.evaluate(() => { window.location.hash = '#/welcome' })
    await page.reload()
    await page.getByTestId('recent-list').locator('button.open', { hasText: 'bug047' }).click()
    await expect(page.getByTestId('repo-switcher')).toContainText('bug047')
    await expect(page).toHaveURL(/files[/]all/)
    await goTo(page, 'security')

    const pass = 'e2e-Passphrase-47'
    await page.getByRole('button', { name: 'Turn on encryption…' }).click()
    const init = page.getByRole('dialog', { name: 'Turn on encryption' })
    await init.getByLabel('Passphrase (optional)').fill(pass)
    await init.getByRole('button', { name: 'Turn on encryption' }).click()

    const code = page.getByRole('dialog', { name: 'Save your recovery code' })
    const unlock = page.getByRole('dialog', { name: 'Unlock this repository' })
    // Sample 12 s: the code is there from the first sample and the unlock prompt never covers it.
    await expect(code).toBeVisible()
    for (let i = 0; i < 80; i++) {
      await expect(code).toBeVisible()
      await expect(unlock).toHaveCount(0)
      await page.waitForTimeout(150)
    }

    await code.getByLabel('I saved it').check()
    await code.getByRole('button', { name: 'Done' }).click()
    await expect(code).toHaveCount(0)

    await expect(unlock).toBeVisible()
    await unlock.getByLabel('Passphrase').fill(pass)
    await unlock.getByRole('button', { name: 'Unlock' }).click()
    await expect(unlock).toHaveCount(0)
    await expect(page.locator('dt:text-is("Recovery code") + dd')).toHaveText('Set up')
  })
})
