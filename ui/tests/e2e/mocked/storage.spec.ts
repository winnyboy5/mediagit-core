import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }

async function boot(page: Page, commands: Record<string, unknown> = {}, handlers: Record<string, string> = {}, scripts: Record<string, unknown[]> = {}, wait = true) {
  await mockIpc(page, { commands: healthy({ repo_stats: fixture('stats'), ...commands }), handlers, scripts: scripts as never, storage: REPO })
  await page.goto('/#/storage', { waitUntil: 'domcontentloaded' })
  // A modal prompt hides the page from the accessibility tree, so its tests skip this.
  if (wait) await expect(page.getByRole('heading', { name: 'Storage', level: 1 })).toBeVisible()
}

test.describe('Storage: states', () => {
  test('populated: sizes, ratio, counts, people; no axe violations', async ({ page }) => {
    await boot(page)
    const cards = page.getByRole('list', { name: 'Storage summary' })
    await expect(cards).toContainText('18.4 GB')
    await expect(cards).toContainText('41.2 GB')
    await expect(cards).toContainText('2.24×')
    await expect(page.getByRole('progressbar', { name: 'Chunks' })).toHaveAttribute('aria-valuenow', '100')
    await expect(page.getByText('48,213')).toBeVisible()
    await expect(page.getByRole('table')).toContainText('Mira Okafor')
    await expect(page.getByRole('table')).toContainText('97')
    expect((await ipcCalls(page, 'repo_stats'))[0]!.args).toEqual({ repo: '/repos/hero-film', passphrase: null })
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton until the numbers arrive', async ({ page }) => {
    await boot(page, {}, { repo_stats: '' }, { repo_stats: [{ hold: true }, { resolve: fixture('stats') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(page.getByRole('list', { name: 'Storage summary' })).toBeVisible()
  })

  test('empty: nothing stored yet', async ({ page }) => {
    await boot(page, { repo_stats: fixture('stats_empty') })
    await expect(page.getByText('Nothing stored yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('error: code and message, Retry asks again', async ({ page }) => {
    await boot(page, { repo_stats: { __error: { code: 'error', message: 'Could not read the repository.' } } })
    await expect(page.getByRole('alert')).toContainText('Could not read the repository')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'repo_stats')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_stats')).length).toBeGreaterThan(before)
  })

  test('busy: recalculating keeps the numbers visible and marked busy', async ({ page }) => {
    await boot(page)
    await expect(page.getByRole('list', { name: 'Storage summary' })).toBeVisible()
    await page.getByRole('button', { name: 'Recalculate storage' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_stats')).length).toBe(2)
    await expect(page.getByRole('list', { name: 'Storage summary' })).toBeVisible()
  })

  test('refresh button is keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Recalculate storage' }).focus()
    await expectFocusRing(page)
  })
})

test.describe('Unlock prompt (needs_input)', () => {
  // Locked until `unlock_repo` was called with the right passphrase; the passphrase itself
  // never reaches `repo_stats`.
  const locked = (right: string) =>
    `(a, calls) => { if (!calls.some((c) => c.cmd === 'unlock_repo' && c.args.passphrase === ${JSON.stringify(right)})) throw { code: 'needs_input', message: 'passphrase needed' }; return ${JSON.stringify(fixture('stats'))} }`
  const unlocking = (right: string) =>
    `(a) => { if (a.passphrase !== ${JSON.stringify(right)}) throw { code: 'wrong_passphrase', message: 'no' }; return null }`
  const handlers = (right: string) => ({ repo_stats: locked(right), unlock_repo: unlocking(right) })

  test('asks for the passphrase, unlocks the repository, runs again, and forgets it', async ({ page }) => {
    await boot(page, {}, handlers('open-sesame-42'), {}, false)
    const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
    await expect(dlg).toBeVisible()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Passphrase').fill('open-sesame-42')
    await expect(dlg.getByLabel('Passphrase')).toHaveAttribute('type', 'password')
    await dlg.getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByRole('list', { name: 'Storage summary' })).toBeVisible()
    // The command itself carries no passphrase; only `unlock_repo` does.
    expect((await ipcCalls(page, 'repo_stats')).map((c) => c.args.passphrase)).toEqual([null, null])
    expect((await ipcCalls(page, 'unlock_repo')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', passphrase: 'open-sesame-42' }])
    expect(await page.evaluate(() => JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }))).not.toContain('open-sesame-42')
    expect(await page.evaluate(() => document.body.innerText)).not.toContain('open-sesame-42')
  })

  test('a wrong passphrase asks again and says so', async ({ page }) => {
    await boot(page, {}, handlers('right-one'), {}, false)
    const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
    await dlg.getByLabel('Passphrase').fill('wrong-one')
    await dlg.getByRole('button', { name: 'Unlock' }).click()
    await expect(dlg).toContainText('That passphrase did not unlock the repository.')
    await expect(dlg.getByLabel('Passphrase')).toHaveValue('')
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Passphrase').fill('right-one')
    await dlg.getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByRole('list', { name: 'Storage summary' })).toBeVisible()
  })

  test('cancelling leaves the view with a plain message and no retry loop', async ({ page }) => {
    await boot(page, {}, handlers('x'), {}, false)
    await page.getByRole('dialog', { name: 'Unlock this repository' }).getByRole('button', { name: 'Cancel' }).click()
    await expect(page.getByRole('alert')).toContainText('Cancelled')
    expect(await ipcCalls(page, 'repo_stats')).toHaveLength(1)
    expect(await ipcCalls(page, 'unlock_repo')).toHaveLength(0)
  })

  test('Escape cancels too', async ({ page }) => {
    await boot(page, {}, handlers('x'), {}, false)
    await expect(page.getByRole('dialog', { name: 'Unlock this repository' })).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await expect(page.getByRole('alert')).toContainText('Cancelled')
  })
})
