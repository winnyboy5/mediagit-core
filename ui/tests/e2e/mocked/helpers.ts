import AxeBuilder from '@axe-core/playwright'
import { expect, type Page } from '@playwright/test'

/** Zero axe violations (WCAG 2.2 AA) on the page as it stands. */
export async function expectNoAxeViolations(page: Page) {
  await page.evaluate(() => document.fonts.ready)
  const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa']).analyze()
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' | ')}`)).toEqual([])
}

/** The focused element has a visible solid outline of at least 2px. */
export async function expectFocusRing(page: Page) {
  const ring = await page.evaluate(() => {
    const s = getComputedStyle(document.activeElement!)
    return { style: s.outlineStyle, width: parseFloat(s.outlineWidth) }
  })
  expect(ring.style).toBe('solid')
  expect(ring.width).toBeGreaterThanOrEqual(2)
}
