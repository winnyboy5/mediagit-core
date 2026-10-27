import { mkdirSync } from 'node:fs'
import { test, expect, type Page } from '@playwright/test'
import { fixture, healthy, ipcRelease, mockIpc, RECENT } from './ipc-mock'

// Review screenshots for the design critique: `SHOTS=1 pnpm e2e shots`.
// Skipped in normal runs. Output: ui/.impeccable/review/<name>-<w>-<theme>.png
test.skip(!process.env.SHOTS, 'screenshots only on demand')

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.changes.mode': JSON.stringify('tree') }
const SIZES = [
  { w: 1440, h: 900 },
  { w: 1024, h: 700 },
]

async function shot(page: Page, name: string, theme: string, w: number) {
  mkdirSync('.impeccable/review', { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  await page.screenshot({ path: `.impeccable/review/${name}-${w}-${theme}.png` })
}

for (const { w, h } of SIZES) {
  test.describe(`${w}px`, () => {
    test.use({ viewport: { width: w, height: h } })

    test('welcome', async ({ page }, info) => {
      await mockIpc(page, { commands: healthy(), storage: { 'mg.recent': RECENT } })
      await page.goto('/#/welcome')
      await expect(page.getByRole('heading', { name: 'Welcome' })).toBeVisible()
      await shot(page, 'V1-welcome', info.project.name, w)
    })

    test('changes states', async ({ page }, info) => {
      const states: [string, Record<string, unknown>][] = [
        ['V2-populated', {}],
        ['V2-empty', { repo_status: fixture('status_empty') }],
        ['V2-error', { repo_status: fixture('status_error') }],
      ]
      for (const [name, extra] of states) {
        await mockIpc(page, { commands: healthy(extra), storage: REPO })
        await page.goto('/#/changes')
        await expect(page.getByRole('heading', { name: 'Changes', level: 1 })).toBeVisible()
        if (name === 'V2-populated') await page.getByRole('treeitem', { name: /sky_hdri/ }).click()
        await page.waitForTimeout(300)
        await shot(page, name, info.project.name, w)
      }
    })

    test('busy and drawer', async ({ page }, info) => {
      await mockIpc(page, { commands: healthy(), storage: REPO, scripts: { repo_add: [{ hold: true }, { resolve: null }] } })
      await page.goto('/#/changes')
      await page.getByRole('button', { name: 'Stage all', exact: true }).click()
      await page.waitForTimeout(700)
      await shot(page, 'V2-busy', info.project.name, w)
      await page.locator('[data-activity-bar]').getByRole('button', { name: 'Activity' }).click()
      await page.waitForTimeout(300)
      await shot(page, 'ACT-drawer-running', info.project.name, w)
      await ipcRelease(page)
      await page.waitForTimeout(300)
      await shot(page, 'ACT-drawer-done', info.project.name, w)
    })
  })
}
