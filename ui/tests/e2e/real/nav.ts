import { expect, type Page } from '@playwright/test'
import { sectionPath } from '../mocked/nav'

/**
 * Go to a section by its old or new route name (`changes`, `tags`, `team/server`...).
 * Waits for the section's own h2 only when the caller names one; the hub sections
 * (Tags, Stash, Settings) carry no h2 of their own.
 */
export async function goTo(page: Page, view: string, heading?: string) {
  await page.evaluate((v) => { window.location.hash = `#/${v}` }, sectionPath(view))
  if (heading) await expect(page.getByRole('heading', { name: heading, level: 2, exact: true }).first()).toBeVisible()
}
