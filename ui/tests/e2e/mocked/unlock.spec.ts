import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, mockIpc, RECENT, showHandler } from './ipc-mock'

// Any command on an encrypted repository that is not unlocked fails `needs_input`. The unlock
// prompt is central (the api layer), so views need nothing of their own: one W1 view (History)
// and one W2 view (Tags & Stash) stand for all of them.

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const PASS = 'correct-horse-battery'

const gate = (ok: unknown) =>
  `(a, calls) => { if (!calls.some((c) => c.cmd === 'unlock_repo')) throw { code: 'needs_input', message: 'passphrase needed' }; return ${JSON.stringify(ok)} }`
const unlocking = `(a) => { if (a.passphrase !== ${JSON.stringify(PASS)}) throw { code: 'wrong_passphrase', message: 'no' }; return null }`

async function boot(page: Page, route: string, handlers: Record<string, string>) {
  await mockIpc(page, {
    commands: healthy({ branch_list: fixture('branches'), key_status: fixture('key_on'), auth_run: fixture('auth_in') }),
    handlers: { unlock_repo: unlocking, ...handlers },
    storage: REPO,
  })
  await page.goto(`/#${route}`, { waitUntil: 'domcontentloaded' })
}
const dialog = (page: Page) => page.getByRole('dialog', { name: 'Unlock this repository' })

test.describe('Unlock: a W1 command (History)', () => {
  test('log needs the passphrase: prompt, unlock, versions appear', async ({ page }) => {
    await boot(page, '/history', { repo_log: gate(fixture('log_graph')), repo_show: showHandler() })
    await expect(dialog(page)).toBeVisible()
    await expectNoAxeViolations(page)
    await dialog(page).getByLabel('Passphrase').fill(PASS)
    await dialog(page).getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByRole('option')).toHaveCount(11)
    expect((await ipcCalls(page, 'unlock_repo')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film', passphrase: PASS }])
    // No command other than `unlock_repo` ever carries the passphrase.
    const others = (await ipcCalls(page)).filter((c) => c.cmd !== 'unlock_repo')
    expect(JSON.stringify(others)).not.toContain(PASS)
    expect(await page.evaluate(() => document.body.innerText + JSON.stringify({ ...localStorage }))).not.toContain(PASS)
  })

  test('cancelling shows the history error state instead of looping', async ({ page }) => {
    await boot(page, '/history', { repo_log: gate(fixture('log_graph')) })
    await dialog(page).getByRole('button', { name: 'Cancel' }).click()
    await expect(page.getByRole('alert')).toContainText('Cancelled')
    expect(await ipcCalls(page, 'unlock_repo')).toHaveLength(0)
  })
})

test.describe('Unlock: a W2 command (Tags & Stash)', () => {
  test('two commands failing together share one prompt and both run again', async ({ page }) => {
    await boot(page, '/tags', { tag_list: gate(fixture('tags')), stash_list: gate(fixture('stash_list')) })
    await expect(dialog(page)).toHaveCount(1)
    await dialog(page).getByLabel('Passphrase').fill('nope')
    await dialog(page).getByRole('button', { name: 'Unlock' }).click()
    await expect(dialog(page)).toContainText('That passphrase did not unlock the repository.')
    await dialog(page).getByLabel('Passphrase').fill(PASS)
    await dialog(page).getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByRole('list', { name: 'Tags' })).toBeVisible()
    await expect(page.getByRole('list', { name: 'Stashed work' })).toBeVisible()
    expect((await ipcCalls(page, 'unlock_repo')).map((c) => c.args.passphrase)).toEqual(['nope', PASS])
    expect(await ipcCalls(page, 'tag_list')).toHaveLength(2)
    expect(await ipcCalls(page, 'stash_list')).toHaveLength(2)
  })
})

test('switching repository locks the one left', async ({ page }) => {
  await boot(page, '/history', { repo_log: `() => (${JSON.stringify(fixture('log_graph'))})`, repo_show: showHandler() })
  await expect(page.getByRole('option').first()).toBeVisible()
  await page.getByTestId('repo-switcher').click()
  await page.getByRole('menuitem', { name: 'trailer-cut' }).click()
  await expect.poll(async () => (await ipcCalls(page, 'lock_repo')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film' }])
})

test.describe('Unlock: session and lock', () => {
  const stats = `(a, calls) => { if (!calls.some((c) => c.cmd === 'unlock_repo')) throw { code: 'needs_input', message: 'passphrase needed' }; return ${JSON.stringify(fixture('stats'))} }`

  test('an unlocked repository shows a Lock action in the sidebar and on Security; locking forgets it', async ({ page }) => {
    await boot(page, '/storage', { repo_stats: stats })
    await dialog(page).getByLabel('Passphrase').fill(PASS)
    await dialog(page).getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByRole('list', { name: 'Storage summary' })).toBeVisible()
    const chip = page.getByTestId('unlocked-chip')
    await expect(chip).toContainText('Unlocked')
    await expectNoAxeViolations(page)
    await page.getByRole('link', { name: 'Security' }).click()
    await expect(page.getByTestId('unlocked')).toContainText('Unlocked for this session')
    await page.getByRole('button', { name: 'Lock now' }).click()
    await expect(chip).toHaveCount(0)
    await expect(page.getByTestId('unlocked')).toHaveCount(0)
    expect((await ipcCalls(page, 'lock_repo')).map((c) => c.args)).toEqual([{ repo: '/repos/hero-film' }])
  })

  test('a failure that is not a wrong passphrase is shown, not asked about again', async ({ page }) => {
    await boot(page, '/storage', { repo_stats: stats, unlock_repo: `() => { throw { code: 'error', message: 'This folder is not a repository.' } }` })
    await dialog(page).getByLabel('Passphrase').fill(PASS)
    await dialog(page).getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByRole('alert')).toContainText('This folder is not a repository')
    await expect(page.getByRole('dialog')).toHaveCount(0)
    expect(await ipcCalls(page, 'unlock_repo')).toHaveLength(1)
  })

  test('the dialog says the passphrase is kept in memory until lock or exit', async ({ page }) => {
    await boot(page, '/storage', { repo_stats: stats })
    await expect(dialog(page)).toContainText('keeps it in memory')
    await expect(dialog(page)).toContainText('until you lock the repository or close the app')
  })
})
