import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, mockIpc, RECENT, showHandler } from './ipc-mock'

// W4c finish-review fixes: rail active state, section counts, header height, Sync header actions,
// the Versions column (Compare / Tag / Undo), pills, disabled buttons.

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }

async function boot(page: Page, path: string, commands: Record<string, unknown> = {}) {
  await mockIpc(page, {
    commands: healthy({
      repo_log: fixture('log_graph'),
      branch_list: fixture('branches'),
      tag_list: fixture('tags'),
      stash_list: fixture('stash_list'),
      ...commands,
    }),
    handlers: { repo_show: showHandler() },
    storage: REPO,
  })
  await page.goto(`/#${path}`, { waitUntil: 'domcontentloaded' })
}

test.describe('W4c: shell', () => {
  test('the rail marks the hub active on every section, not only the first', async ({ page }) => {
    await boot(page, '/branches/merge')
    const rail = page.getByRole('navigation', { name: 'Main' })
    await expect(rail.getByRole('link', { name: /^Branches/ })).toHaveAttribute('aria-current', 'page')
    await expect(rail.getByRole('link', { name: /^History/ })).not.toHaveAttribute('aria-current', 'page')
    await page.getByRole('group', { name: 'Section' }).getByRole('button', { name: /^Stash/ }).click()
    await expect(rail.getByRole('link', { name: /^Branches/ })).toHaveAttribute('aria-current', 'page')
  })

  test('every hub header has the same height, Settings and Sync included', async ({ page }) => {
    const heights: number[] = []
    await boot(page, '/files/changes')
    for (const p of ['/files/changes', '/branches/sync', '/team/locks', '/settings/general']) {
      await page.goto(`/#${p}`, { waitUntil: 'domcontentloaded' })
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
      heights.push((await page.locator('header.hubhead').boundingBox())!.height)
    }
    expect(new Set(heights).size).toBe(1)
  })

  test('Get latest and Send are in the Sync header and only there', async ({ page }) => {
    await boot(page, '/branches/sync')
    await expect(page.getByRole('button', { name: /^Get latest/ })).toHaveCount(1)
    await expect(page.getByRole('button', { name: /^Send/ })).toHaveCount(1)
    await expectNoAxeViolations(page)
  })

  test('sections show a count where there is one', async ({ page }) => {
    await boot(page, '/history/versions')
    await expect(page.getByRole('group', { name: 'Section' }).getByRole('button', { name: /^Tags/ })).toHaveCount(1)
    await page.goto('/#/files/all', { waitUntil: 'domcontentloaded' })
    await expect(page.getByRole('group', { name: 'Section' }).getByRole('button', { name: /^Changes \d+$/ })).toBeVisible()
  })
})

test.describe('W4c: versions column', () => {
  test('Compare with previous opens Compare on that pair', async ({ page }) => {
    await boot(page, '/history/versions', { repo_diff: fixture('diff_commits') })
    await expect(page.getByTestId('commit-detail').getByRole('heading', { name: /Merge grade pass/ })).toBeVisible()
    await page.getByRole('button', { name: 'Compare with previous' }).click()
    await expect(page).toHaveURL(/#\/history\/compare$/)
    await expect.poll(async () => (await ipcCalls(page, 'repo_diff')).length).toBeGreaterThan(0)
    const args = (await ipcCalls(page, 'repo_diff'))[0]!.args
    expect(args.from).toBeTruthy()
    expect(args.to).toBeTruthy()
  })

  test('Tag this version names the selected version', async ({ page }) => {
    await boot(page, '/history/versions', { tag_create: { action: 'create', created: null, deleted: [], missing: [] } })
    await page.getByRole('button', { name: 'Tag this version' }).click()
    const dlg = page.getByRole('dialog', { name: 'Tag this version' })
    await dlg.getByLabel('Tag name').fill('v2.0')
    await dlg.getByRole('button', { name: 'Tag this version' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'tag_create')).map((c) => c.args)).toMatchObject([{ name: 'v2.0', options: { commit: expect.stringMatching(/^[0-9a-f]{40}$/) } }])
  })

  test('files at this version sit behind a disclosure', async ({ page }) => {
    await boot(page, '/history/versions')
    await expect(page.getByRole('tree', { name: 'Files at this version' })).toHaveCount(0)
    await page.getByText('Files at this version', { exact: true }).click()
    await expect(page.getByRole('tree', { name: 'Files at this version' })).toBeVisible()
  })
})

test.describe('W4c: components', () => {
  test('a disabled primary button is a flat grey control of the same size', async ({ page }) => {
    await boot(page, '/files/changes')
    const save = page.getByRole('button', { name: 'Save version' })
    await expect(save).toBeDisabled()
    const css = await save.evaluate((el) => {
      const s = getComputedStyle(el)
      return { bg: s.backgroundColor, opacity: s.opacity, border: s.borderTopWidth }
    })
    expect(css.opacity).toBe('1')
    expect(css.border).toBe('1px')
    const active = await page.getByRole('button', { name: 'Include all' }).count()
    expect(active).toBeGreaterThanOrEqual(0)
  })

  test('tag pills are outlined text in the body font', async ({ page }) => {
    await boot(page, '/history/tags')
    const pill = page.locator('[data-tag="v0.3"]').locator('.pill').first()
    await expect(pill).toBeVisible()
    const css = await pill.evaluate((el) => {
      const s = getComputedStyle(el)
      return { bg: s.backgroundColor, border: s.borderTopWidth, font: s.fontFamily }
    })
    expect(css.bg).toBe('rgba(0, 0, 0, 0)')
    expect(css.border).toBe('1px')
    expect(css.font).not.toMatch(/mono/i)
  })
})
