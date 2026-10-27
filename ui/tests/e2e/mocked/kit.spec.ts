import { expect, test } from '@playwright/test'
import AxeBuilder from '@axe-core/playwright'

const axe = (page: import('@playwright/test').Page) =>
  new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])

test.beforeEach(async ({ page }) => {
  await page.goto('/#/kit')
  await expect(page.getByRole('heading', { name: 'Component kit' })).toBeVisible()
  await page.evaluate(() => document.fonts.ready)
})

test('kit page has no axe violations', async ({ page }) => {
  const r = await axe(page).analyze()
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' | ')}`)).toEqual([])
})

test('open dialogs have no axe violations', async ({ page }) => {
  await page.getByRole('button', { name: 'Open dialog' }).click()
  await expect(page.getByRole('dialog')).toBeVisible()
  expect((await axe(page).analyze()).violations).toEqual([])
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: 'Open alert' }).click()
  await expect(page.getByRole('alertdialog')).toBeVisible()
  expect((await axe(page).analyze()).violations).toEqual([])
})

test('density and theme toggles drive the document', async ({ page }) => {
  await page.getByRole('button', { name: 'Compact' }).click()
  await expect(page.locator('html')).toHaveAttribute('data-density', 'compact')
  await page.getByRole('button', { name: 'Dark' }).click()
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  await page.waitForTimeout(400) // let colour transitions settle before axe
  expect((await axe(page).analyze()).violations).toEqual([])
  await page.getByRole('button', { name: 'Light' }).click()
  await page.waitForTimeout(400)
  expect((await axe(page).analyze()).violations).toEqual([])
})

test('keyboard focus ring is visible', async ({ page }) => {
  await page.keyboard.press('Tab')
  const ring = await page.evaluate(() => {
    const s = getComputedStyle(document.activeElement!)
    return { style: s.outlineStyle, width: parseFloat(s.outlineWidth), color: s.outlineColor }
  })
  expect(ring.style).toBe('solid')
  expect(ring.width).toBeGreaterThanOrEqual(2)
  expect(ring.color).not.toBe('rgba(0, 0, 0, 0)')
})

test('section screenshots', async ({ page }, info) => {
  for (const s of await page.locator('section[data-section]').all()) {
    const name = await s.getAttribute('data-section')
    await s.screenshot({ path: `test-results/kit-${info.project.name}-${name}.png` })
  }
})
