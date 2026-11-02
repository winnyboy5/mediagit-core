import { expect, test, type Page } from '@playwright/test'
import { fixture, healthy, ipcCalls, mockIpc, RECENT } from './ipc-mock'

// W3 design-review fixes: lock state, error branches, danger/tones, density, wording.

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const NEEDS = `() => { throw { code: 'needs_input', message: 'passphrase needed' } }`

async function boot(page: Page, route: string, o: { commands?: Record<string, unknown>; handlers?: Record<string, string> } = {}) {
  await mockIpc(page, {
    commands: healthy({
      branch_list: fixture('branches'),
      config_list: fixture('config_list'),
      sparse_list: fixture('sparse_off'),
      repo_gc: fixture('gc_dry'),
      repo_fsck: fixture('fsck_ok'),
      repo_stats: fixture('stats'),
      reflog_list: fixture('reflog'),
      repo_log: fixture('log_graph'),
      ...o.commands,
    }),
    handlers: o.handlers,
    storage: REPO,
  })
  await page.goto(`/#${route}`, { waitUntil: 'domcontentloaded' })
}
const unlockDialog = (page: Page) => page.getByRole('dialog', { name: 'Unlock this repository' })

test.describe('lock state', () => {
  test('a locked repository shows a Locked chip with Unlock…, which opens the dialog', async ({ page }) => {
    await boot(page, '/history', { handlers: { repo_log: NEEDS } })
    await unlockDialog(page).getByRole('button', { name: 'Cancel' }).click()
    const chip = page.getByTestId('locked-chip')
    await expect(chip).toContainText('Locked')
    await expect(page.getByTestId('unlocked-chip')).toHaveCount(0)
    await chip.getByRole('button', { name: 'Unlock this repository' }).click()
    await expect(unlockDialog(page)).toBeVisible()
  })

  test('needs_input that survives an unlock reads locked with Unlock…, no raw code, and the chip is not Unlocked', async ({ page }) => {
    await boot(page, '/history', { handlers: { repo_log: NEEDS, unlock_repo: '() => null' } })
    await unlockDialog(page).getByLabel('Passphrase').fill('pw')
    await unlockDialog(page).getByRole('button', { name: 'Unlock' }).click()
    const err = page.getByRole('alert')
    await expect(err).toContainText('This repository is locked')
    await expect(err).not.toContainText('needs_input')
    await expect(err.getByRole('button', { name: 'Unlock…' })).toBeVisible()
    await expect(page.getByTestId('unlocked-chip')).toHaveCount(0)
    await expect(page.getByTestId('locked-chip')).toBeVisible()
  })

  test('a wrong passphrase marks the field invalid', async ({ page }) => {
    await boot(page, '/history', { handlers: { repo_log: NEEDS, unlock_repo: `() => { throw { code: 'wrong_passphrase', message: 'no' } }` } })
    await unlockDialog(page).getByLabel('Passphrase').fill('bad')
    await unlockDialog(page).getByRole('button', { name: 'Unlock' }).click()
    await expect(unlockDialog(page).getByLabel('Passphrase')).toHaveAttribute('aria-invalid', 'true')
  })
})

test('sidebar groups Storage..Settings under a More label', async ({ page }) => {
  await boot(page, '/history')
  const more = page.getByRole('heading', { name: 'More', level: 2 })
  await expect(more).toBeVisible()
  const nav = page.getByRole('navigation', { name: 'Main' })
  const remote = await nav.getByRole('link', { name: 'Remote Browser' }).boundingBox()
  const label = await more.boundingBox()
  const storage = await nav.getByRole('link', { name: 'Storage' }).boundingBox()
  expect(remote!.y).toBeLessThan(label!.y)
  expect(label!.y).toBeLessThan(storage!.y)
})

test('settings: a failed read shows an error with Retry in the All settings card', async ({ page }) => {
  await boot(page, '/settings', { commands: { config_list: { __error: { code: 'error', message: 'Cannot read the configuration.' } } } })
  const card = page.getByTestId('config')
  await expect(card.getByRole('alert')).toContainText('Cannot read the configuration')
  const before = (await ipcCalls(page, 'config_list')).length
  await card.getByRole('button', { name: 'Retry' }).click()
  await expect.poll(async () => (await ipcCalls(page, 'config_list')).length).toBeGreaterThan(before)
})

test('maintenance: Clean up… is danger, Preview is primary; findings use chips; counts are formatted', async ({ page }) => {
  await boot(page, '/maintenance', { commands: { repo_fsck: fixture('fsck_bad') } })
  const gc = page.locator('[data-task="gc"]')
  await expect(gc.getByRole('button', { name: 'Preview clean-up' })).toHaveClass(/primary/)
  await gc.getByRole('checkbox', { name: 'Preview only' }).click()
  await expect(gc.getByRole('button', { name: 'Clean up…' })).toHaveClass(/danger/)
  await page.locator('[data-task="fsck"]').getByRole('button', { name: 'Run integrity check' }).click()
  const res = page.getByTestId('fsck-result')
  await expect(res).toContainText('48,213')
  const list = page.getByRole('list', { name: 'Check integrity findings' })
  await expect(list.locator('.chip.bad')).toHaveText(['Error'])
  await expect(list.locator('.chip.warn')).toHaveText(['Warning'])
  await expect(res.locator('dd.bad')).toHaveCount(2) // Damaged and Missing are non-zero
})

test('compact density shortens table and nav rows; comfortable stays 36px', async ({ page }) => {
  const rowH = (sel: string) => page.locator(sel).first().evaluate((e) => e.getBoundingClientRect().height)
  await boot(page, '/settings')
  await expect(page.getByTestId('config')).toContainText('performance.upload_concurrency')
  const comfortable = await rowH('[data-testid="config"] tbody tr')
  expect(comfortable).toBeGreaterThanOrEqual(36)
  expect(await rowH('nav a.item')).toBe(36)
  await page.evaluate(() => document.documentElement.setAttribute('data-density', 'compact'))
  expect(await rowH('[data-testid="config"] tbody tr')).toBeLessThanOrEqual(comfortable - 8)
  expect(await rowH('nav a.item')).toBe(28)
})

test.describe('advanced', () => {
  test('tabs lead with plain words; reflog intro avoids the word ref', async ({ page }) => {
    await boot(page, '/advanced')
    await expect(page.getByRole('tab')).toHaveText(['Replay onto a branch (rebase)', 'Copy a version (cherry-pick)', 'Find the break (bisect)', 'Undo log (reflog)'])
    await page.getByRole('tab', { name: 'Undo log' }).click()
    await expect(page.locator('[data-panel="reflog"] > p').first()).not.toContainText(/\bref\b/i)
  })

  test('copy a version starts with no version chosen', async ({ page }) => {
    await boot(page, '/advanced')
    await page.getByRole('tab', { name: 'Copy a version' }).click()
    await expect(page.getByLabel('Copy this version')).toContainText('Choose a version')
    await expect(page.getByRole('button', { name: 'Copy version' })).toBeDisabled()
  })
})

test('security signed out: only Sign in is primary; encryption is secondary', async ({ page }) => {
  await boot(page, '/security', { commands: { auth_run: fixture('auth_out'), key_status: fixture('key_off') } })
  await expect(page.getByRole('button', { name: 'Sign in' })).toHaveClass(/primary/)
  await expect(page.getByRole('button', { name: 'Turn on encryption…' })).not.toHaveClass(/primary/)
})
