import { expect, test } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }

// Old top-level path -> where it lives now.
const LEGACY: [string, string][] = [
  ['/files', '/files/all'],
  ['/team', '/team/locks'],
  ['/changes', '/files/changes'],
  ['/history', '/history/versions'],
  ['/compare', '/history/compare'],
  ['/tags', '/history/tags'],
  ['/branches', '/branches/branches'],
  ['/merge', '/branches/merge'],
  ['/sync', '/branches/sync'],
  ['/locks', '/team/locks'],
  ['/remote', '/team/remote'],
  ['/server', '/team/server'],
  ['/storage', '/settings/storage'],
  ['/maintenance', '/settings/maintenance'],
  ['/security', '/settings/security'],
  ['/advanced', '/settings/advanced'],
  ['/settings', '/settings/general'],
]

// hub, hub heading, section id, section label, a heading that proves the section rendered.
const SECTIONS: [string, string, string, string, string][] = [
  ['files', 'Files', 'all', 'All files', 'All files'],
  ['files', 'Files', 'changes', 'Changes', 'Changes'],
  ['history', 'History', 'versions', 'Versions', 'History'],
  ['history', 'History', 'compare', 'Compare', 'Compare'],
  ['history', 'History', 'tags', 'Tags', 'Tags'],
  ['branches', 'Branches', 'branches', 'Branches', 'Branches'],
  ['branches', 'Branches', 'merge', 'Merge', 'Merge & conflicts'],
  ['branches', 'Branches', 'sync', 'Sync', 'Sync'],
  ['branches', 'Branches', 'stash', 'Stash', 'Stash'],
  ['team', 'Team', 'locks', 'Locks', 'Locks'],
  ['team', 'Team', 'remote', 'Remote files', 'Remote Browser'],
  ['team', 'Team', 'server', 'Server', 'Server'],
  ['settings', 'Settings', 'general', 'General', 'General'],
  ['settings', 'Settings', 'storage', 'Storage', 'Storage'],
  ['settings', 'Settings', 'maintenance', 'Maintenance', 'Maintenance'],
  ['settings', 'Settings', 'security', 'Security', 'Security'],
  ['settings', 'Settings', 'advanced', 'Advanced tools', 'Advanced'],
]

test.describe('Hubs: routes and sections', () => {
  test.beforeEach(async ({ page }) => {
    await mockIpc(page, {
      commands: healthy({ tag_list: fixture('tags'), stash_list: fixture('stash_list'), config_list: fixture('config_list'), branch_list: fixture('branches') }),
      storage: REPO,
    })
  })

  for (const [from, to] of LEGACY) {
    test(`old route ${from} redirects to ${to}`, async ({ page }) => {
      await page.goto(`/#${from}`, { waitUntil: 'domcontentloaded' })
      await expect(page).toHaveURL(new RegExp(`#${to}$`))
      await expect(page.getByRole('heading', { name: /^(Files|History|Branches|Team|Settings)$/, level: 1 })).toBeVisible()
    })
  }

  for (const [hub, hubName, id, label, heading] of SECTIONS) {
    test(`${hub}/${id} renders under the ${hubName} hub`, async ({ page }) => {
      await page.goto(`/#/${hub}/${id}`, { waitUntil: 'domcontentloaded' })
      await expect(page.getByRole('heading', { name: hubName, level: 1 })).toBeVisible()
      await expect(page.getByRole('heading', { name: heading, level: 2, exact: true }).first()).toBeVisible()
      // The section control (or Settings' list) marks the current section.
      const nav = page.getByRole('navigation', { name: "Section" })
      if (hub === 'settings') await expect(nav.getByRole('button', { name: label })).toHaveAttribute('aria-current', 'page')
      else await expect(page.getByRole('group', { name: "Section" }).getByRole('button', { name: new RegExp(`^${label}( \\d+)?$`) })).toHaveAttribute('aria-pressed', 'true')
      await expectNoAxeViolations(page)
    })
  }

  test('rail has four hubs plus Settings; selecting one opens it; keyboard shortcuts move between them', async ({ page }) => {
    await page.goto('/#/files/changes', { waitUntil: 'domcontentloaded' })
    const rail = page.getByRole('navigation', { name: 'Main' })
    for (const n of ['Files', 'History', 'Branches', 'Team', 'Settings']) await expect(rail.getByRole('link', { name: new RegExp(`^${n}`) })).toBeVisible()
    await rail.getByRole('link', { name: 'Team' }).click()
    await expect(page).toHaveURL(/#\/team\/locks$/)
    await page.keyboard.press('Control+2')
    await expect(page).toHaveURL(/#\/history\/versions$/)
    await page.keyboard.press('Alt+3')
    await expect(page).toHaveURL(/#\/history\/tags$/)
    await page.keyboard.press('Control+,')
    await expect(page).toHaveURL(/#\/settings\/general$/)
    await page.keyboard.press('Control+1')
    await expect(page).toHaveURL(/#\/files\/all$/)
  })

  test('the repository switcher separates Create and Clone', async ({ page }) => {
    await page.goto('/#/files', { waitUntil: 'domcontentloaded' })
    await page.getByTestId('repo-switcher').click()
    await expect(page.getByRole('menuitem', { name: 'Open a repository…' })).toBeVisible()
    await expect(page.getByRole('menuitem', { name: 'Create a new repository…' })).toBeVisible()
    await page.getByRole('menuitem', { name: 'Clone a repository…' }).click()
    await expect(page.getByRole('dialog', { name: /clone/i })).toBeVisible()
  })

  test('no activity pill while idle', async ({ page }) => {
    await page.goto('/#/files', { waitUntil: 'domcontentloaded' })
    await expect(page.getByRole('heading', { name: 'Files', level: 1 })).toBeVisible()
    await expect(page.locator('[data-activity-pill]')).toHaveCount(0)
    await expect(page.getByText('No activity')).toHaveCount(0)
  })
})
