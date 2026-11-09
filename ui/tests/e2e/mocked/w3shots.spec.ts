import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { fixture, healthy, ipcRelease, mockIpc, RECENT } from './ipc-mock'
import { openSection } from './nav'

// W3 review screenshots: `SHOTS=1 pnpm e2e w3shots`. Skipped in normal runs.
// Output: ui/.impeccable/review/w3-<view>-<state>-<w>-<theme>.png. Every state is its own test (fresh page).
test.skip(!process.env.SHOTS, 'screenshots only on demand')

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const SIZES = [
  { w: 1440, h: 900 },
  { w: 1024, h: 700 },
]
const wrong = `() => { throw { code: 'wrong_passphrase', message: 'no' } }`
const PASS = 'correct-horse-battery'
const good = `(a) => { if (a.passphrase !== ${JSON.stringify(PASS)}) throw { code: 'wrong_passphrase', message: 'no' }; return null }`
const needs = `() => { throw { code: 'needs_input', message: 'passphrase needed' } }`
// Needs the passphrase until unlock_repo has been called, like a real encrypted repository.
const gated = `(a, calls) => { if (!calls.some((c) => c.cmd === 'unlock_repo')) throw { code: 'needs_input', message: 'passphrase needed' }; return ${JSON.stringify(fixture('log_graph'))} }`

interface Boot {
  commands?: Record<string, unknown>
  handlers?: Record<string, string>
  scripts?: Record<string, unknown[]>
}

async function shot(page: Page, name: string, theme: string, w: number) {
  mkdirSync('.impeccable/review', { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(350)
  await page.screenshot({ path: `.impeccable/review/w3-${name}-${w}-${theme}.png` })
}

async function open(page: Page, route: string, heading: string, b: Boot = {}) {
  await mockIpc(page, {
    commands: healthy({
      repo_log: fixture('log_graph'),
      branch_list: fixture('branches'),
      repo_stats: fixture('stats'),
      repo_gc: fixture('gc_dry'),
      repo_fsck: fixture('fsck_ok'),
      repo_verify: fixture('fsck_ok'),
      auth_run: fixture('auth_in'),
      key_status: fixture('key_on'),
      config_list: fixture('config_list'),
      config_schema: fixture('config_schema'),
      sparse_list: fixture('sparse_on'),
      reflog_list: fixture('reflog'),
      ...b.commands,
    }),
    handlers: b.handlers,
    scripts: b.scripts as never,
    storage: REPO,
  })
  await openSection(page, route, heading)
}

for (const { w, h } of SIZES) {
  test.describe(`${w}px`, () => {
    test.use({ viewport: { width: w, height: h } })
    const T = (name: string, fn: (page: Page, theme: string) => Promise<void>) => test(name, ({ page }, info) => fn(page, info.project.name))

    // V11 Storage
    T('V11 populated', async (page, t) => {
      await open(page, '/storage', 'Storage')
      await expect(page.getByRole('list', { name: 'Storage summary' })).toBeVisible()
      await shot(page, 'V11-storage-populated', t, w)
    })
    T('V11 loading', async (page, t) => {
      await open(page, '/storage', 'Storage', { handlers: { repo_stats: '' }, scripts: { repo_stats: [{ hold: true }, { resolve: fixture('stats') }] } })
      await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
      await shot(page, 'V11-storage-loading', t, w)
    })
    T('V11 empty', async (page, t) => {
      await open(page, '/storage', 'Storage', { commands: { repo_stats: fixture('stats_empty') } })
      await expect(page.getByText('Nothing stored yet')).toBeVisible()
      await shot(page, 'V11-storage-empty', t, w)
    })
    T('V11 error', async (page, t) => {
      await open(page, '/storage', 'Storage', { commands: { repo_stats: { __error: { code: 'error', message: 'Could not read the storage folder: the object store is missing.' } } } })
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V11-storage-error', t, w)
    })
    T('V11 unlock', async (page, t) => {
      await open(page, '/storage', '', { handlers: { repo_stats: needs } })
      await expect(page.getByRole('dialog', { name: 'Unlock this repository' })).toBeVisible()
      await shot(page, 'V11-storage-unlock', t, w)
    })

    T('V11 unlock wrong passphrase', async (page, t) => {
      await open(page, '/storage', '', { handlers: { repo_stats: needs, unlock_repo: wrong } })
      const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
      await dlg.getByLabel('Passphrase').fill('nope')
      await dlg.getByRole('button', { name: 'Unlock' }).click()
      await expect(dlg).toContainText('That passphrase did not unlock the repository.')
      await shot(page, 'V11-storage-unlock-wrong', t, w)
    })

    // V12 Maintenance
    T('V12 idle', async (page, t) => {
      await open(page, '/maintenance', 'Maintenance')
      await shot(page, 'V12-maintenance-idle', t, w)
    })
    T('V12 results', async (page, t) => {
      await open(page, '/maintenance', 'Maintenance', { commands: { repo_fsck: fixture('fsck_bad') } })
      await page.getByRole('button', { name: 'Preview clean-up' }).click()
      await page.getByRole('button', { name: 'Run integrity check' }).click()
      await expect(page.getByTestId('fsck-result')).toBeVisible()
      await shot(page, 'V12-maintenance-results', t, w)
    })
    T('V12 busy', async (page, t) => {
      await open(page, '/maintenance', 'Maintenance', { scripts: { repo_gc: fixture('gc_progress') } })
      await page.getByRole('button', { name: 'Preview clean-up' }).click()
      await expect(page.getByTestId('gc-progress')).toBeVisible()
      await shot(page, 'V12-maintenance-busy', t, w)
      await ipcRelease(page)
    })
    T('V12 confirm', async (page, t) => {
      await open(page, '/maintenance', 'Maintenance')
      await page.getByRole('checkbox', { name: 'Preview only' }).click()
      await page.getByRole('button', { name: 'Clean up…' }).click()
      await expect(page.getByRole('alertdialog')).toBeVisible()
      await shot(page, 'V12-maintenance-confirm', t, w)
    })
    T('V12 error', async (page, t) => {
      await open(page, '/maintenance', 'Maintenance', { commands: { repo_verify: { __error: { code: 'error', message: 'Another operation holds the repository lock.' } } } })
      await page.getByRole('button', { name: 'Verify' }).click()
      await expect(page.getByRole('alert')).toBeVisible()
      await shot(page, 'V12-maintenance-error', t, w)
    })

    // V13 Security
    T('V13 signed in', async (page, t) => {
      await open(page, '/security', 'Security')
      await expect(page.getByTestId('account')).toContainText('mira')
      await shot(page, 'V13-security-signedin', t, w)
    })
    T('V13 signed out', async (page, t) => {
      await open(page, '/security', 'Security', { commands: { auth_run: fixture('auth_out'), key_status: fixture('key_off') } })
      await expect(page.getByTestId('account')).toContainText('not signed in')
      await shot(page, 'V13-security-signedout', t, w)
    })
    T('V13 loading', async (page, t) => {
      await open(page, '/security', 'Security', { scripts: { auth_run: [{ hold: true }, { resolve: fixture('auth_in') }], key_status: [{ hold: true }, { resolve: fixture('key_on') }] } })
      await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
      await shot(page, 'V13-security-loading', t, w)
    })
    T('V13 error', async (page, t) => {
      await open(page, '/security', 'Security', { commands: { auth_run: { __error: { code: 'error', message: 'The server did not answer.' } } } })
      await expect(page.getByTestId('account').getByRole('alert')).toBeVisible()
      await shot(page, 'V13-security-error', t, w)
    })
    T('V13 sign in dialog', async (page, t) => {
      await open(page, '/security', 'Security', { commands: { auth_run: fixture('auth_out') } })
      await page.getByRole('button', { name: 'Sign in' }).click()
      await expect(page.getByRole('dialog', { name: 'Sign in' })).toBeVisible()
      await shot(page, 'V13-security-signin', t, w)
    })
    T('V13 recovery code', async (page, t) => {
      await open(page, '/security', 'Security', { commands: { key_status: fixture('key_off'), key_init: fixture('key_init') } })
      await page.getByRole('button', { name: 'Turn on encryption…' }).click()
      await page.getByRole('dialog', { name: 'Turn on encryption' }).getByRole('button', { name: 'Turn on encryption' }).click()
      await expect(page.getByTestId('recovery-code')).toBeVisible()
      await shot(page, 'V13-security-recovery', t, w)
    })

    T('V13 recovery code over encrypted card', async (page, t) => {
      await open(page, '/security', 'Security', { commands: { key_status: fixture('key_off'), key_init: fixture('key_init') } })
      await page.getByRole('button', { name: 'Turn on encryption…' }).click()
      await page.getByRole('dialog', { name: 'Turn on encryption' }).getByRole('button', { name: 'Turn on encryption' }).click()
      await expect(page.getByTestId('key-source')).toBeVisible()
      await shot(page, 'V13-security-recovery-source', t, w)
    })
    T('V13 unlocked and Lock now', async (page, t) => {
      await open(page, '/history', '', { handlers: { repo_log: gated, unlock_repo: good } })
      const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
      await expect(dlg).toBeVisible()
      await dlg.getByLabel('Passphrase').fill(PASS)
      await dlg.getByRole('button', { name: 'Unlock' }).click()
      await expect(dlg).toHaveCount(0)
      await openSection(page, 'security')
      await expect(page.getByTestId('unlocked')).toBeVisible()
      await shot(page, 'V13-security-unlocked', t, w)
    })
    T('sidebar Lock chip locked', async (page, t) => {
      await open(page, '/changes', 'Changes')
      await expect(page.getByTestId('unlocked-chip')).toHaveCount(0)
      await shot(page, 'chip-locked', t, w)
    })
    T('sidebar Locked chip (prompt cancelled)', async (page, t) => {
      await open(page, '/history', '', { handlers: { repo_log: needs } })
      await page.getByRole('dialog', { name: 'Unlock this repository' }).getByRole('button', { name: 'Cancel' }).click()
      await expect(page.getByTestId('locked-chip')).toBeVisible()
      await shot(page, 'chip-locked-needs', t, w)
    })
    T('locked error state after unlock', async (page, t) => {
      await open(page, '/history', '', { handlers: { repo_log: needs, unlock_repo: good } })
      const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
      await dlg.getByLabel('Passphrase').fill(PASS)
      await dlg.getByRole('button', { name: 'Unlock' }).click()
      await expect(page.getByRole('alert')).toContainText('This repository is locked')
      await shot(page, 'locked-error', t, w)
    })
    T('sidebar Lock chip unlocked', async (page, t) => {
      await open(page, '/history', '', { handlers: { repo_log: gated, unlock_repo: good } })
      const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
      await dlg.getByLabel('Passphrase').fill(PASS)
      await dlg.getByRole('button', { name: 'Unlock' }).click()
      await expect(page.getByTestId('unlocked-chip')).toBeVisible()
      await shot(page, 'chip-unlocked', t, w)
    })

    // V14 Advanced
    T('V14 rebase clash', async (page, t) => {
      await open(page, '/advanced', 'Advanced', { commands: { rebase_start: fixture('rebase_conflict') } })
      await page.getByRole('button', { name: 'Start rebase…' }).click()
      await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
      await expect(page.getByTestId('rebase-summary')).toBeVisible()
      await shot(page, 'V14-advanced-rebase', t, w)
    })
    T('V14 rebase idle', async (page, t) => {
      await open(page, '/advanced', 'Advanced')
      await expect(page.getByLabel('Rebase onto')).toBeVisible()
      await shot(page, 'V14-advanced-idle', t, w)
    })
    T('V14 rebase busy', async (page, t) => {
      await open(page, '/advanced', 'Advanced', { scripts: { rebase_start: fixture('seq_progress') } })
      await page.getByRole('button', { name: 'Start rebase…' }).click()
      await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()
      await expect(page.getByTestId('rebase-progress')).toBeVisible()
      await shot(page, 'V14-advanced-busy', t, w)
    })
    T('V14 cherry-pick', async (page, t) => {
      await open(page, '/advanced', 'Advanced')
      await page.getByRole('tab', { name: 'Cherry-pick' }).click()
      await expect(page.getByLabel('Copy this version')).toBeVisible()
      await shot(page, 'V14-advanced-pick', t, w)
    })
    T('V14 bisect idle', async (page, t) => {
      await open(page, '/advanced', 'Advanced')
      await page.getByRole('tab', { name: 'Bisect' }).click()
      await expect(page.getByRole('button', { name: 'Start bisect' })).toBeVisible()
      await shot(page, 'V14-advanced-bisect-idle', t, w)
    })
    T('V14 bisect testing', async (page, t) => {
      await open(page, '/advanced', 'Advanced', { commands: { bisect_start: fixture('bisect_testing') } })
      await page.getByRole('tab', { name: 'Bisect' }).click()
      await page.getByRole('button', { name: 'Start bisect' }).click()
      await expect(page.getByTestId('bisect-state')).toBeVisible()
      await shot(page, 'V14-advanced-bisect-testing', t, w)
    })
    T('V14 bisect found', async (page, t) => {
      await open(page, '/advanced', 'Advanced', { commands: { bisect_start: fixture('bisect_complete') } })
      await page.getByRole('tab', { name: 'Bisect' }).click()
      await page.getByRole('button', { name: 'Start bisect' }).click()
      await expect(page.getByText('The first bad version is')).toBeVisible()
      await shot(page, 'V14-advanced-bisect-found', t, w)
    })
    T('V14 reflog', async (page, t) => {
      await open(page, '/advanced', 'Advanced')
      await page.getByRole('tab', { name: 'Reflog' }).click()
      await expect(page.getByRole('button', { name: 'Restore to @1' })).toBeVisible()
      await shot(page, 'V14-advanced-reflog', t, w)
    })
    T('V14 reflog restore', async (page, t) => {
      await open(page, '/advanced', 'Advanced')
      await page.getByRole('tab', { name: 'Reflog' }).click()
      await page.getByRole('button', { name: 'Restore to @1' }).click()
      await expect(page.getByRole('alertdialog')).toBeVisible()
      await shot(page, 'V14-advanced-restore', t, w)
    })
    T('V14 reflog empty', async (page, t) => {
      await open(page, '/advanced', 'Advanced', { commands: { reflog_list: fixture('reflog_empty') } })
      await page.getByRole('tab', { name: 'Reflog' }).click()
      await expect(page.getByText('No history for this ref')).toBeVisible()
      await shot(page, 'V14-advanced-reflog-empty', t, w)
    })

    // V15 Settings
    T('V15 populated', async (page, t) => {
      await open(page, '/settings', 'General')
      await expect(page.getByTestId('config').getByLabel('Files sent at once')).toHaveValue('8')
      await shot(page, 'V15-settings-populated', t, w)
    })
    T('V15 loading', async (page, t) => {
      await open(page, '/settings', 'General', { scripts: { config_list: [{ hold: true }, { resolve: fixture('config_list') }] } })
      await expect(page.getByRole('status', { name: 'Loading' }).first()).toBeVisible()
      await shot(page, 'V15-settings-loading', t, w)
    })
    T('V15 empty', async (page, t) => {
      await open(page, '/settings', 'General', { commands: { config_list: fixture('config_empty'), sparse_list: fixture('sparse_off') } })
      await expect(page.getByTestId('config').getByLabel('Files sent at once')).toHaveValue('')
      await shot(page, 'V15-settings-empty', t, w)
    })
    T('V15 error', async (page, t) => {
      await open(page, '/settings', 'General', { commands: { config_list: { __error: { code: 'error', message: 'Cannot read the configuration.' } } } })
      await expect(page.getByTestId('identity').getByRole('alert')).toBeVisible()
      await shot(page, 'V15-settings-error', t, w)
    })
    T('V15 sparse busy', async (page, t) => {
      await open(page, '/settings', 'General', { scripts: { sparse_set: fixture('sparse_progress') } })
      await page.getByRole('button', { name: 'Apply…' }).click()
      await page.getByRole('alertdialog').getByRole('button', { name: 'Apply' }).click()
      await expect(page.getByTestId('sparse-progress')).toBeVisible()
      await shot(page, 'V15-settings-sparse-busy', t, w)
    })
    T('V15 compact dark', async (page, t) => {
      await open(page, '/settings', 'General')
      await page.getByRole('button', { name: 'Compact' }).click()
      await page.getByRole('button', { name: 'Dark' }).click()
      await shot(page, 'V15-settings-compact-dark', t, w)
    })
  })
}
