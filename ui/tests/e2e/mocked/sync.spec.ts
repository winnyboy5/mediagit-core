import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const bar = (page: Page) => page.locator('[data-activity-bar]')
const run = (page: Page) => page.getByTestId('transfer-progress')

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, { commands: healthy({ remote_list: fixture('remotes'), ...commands }), storage: REPO, scripts: scripts as never })
  await page.goto('/#/sync', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Sync', level: 1 })).toBeVisible()
}

test.describe('Sync: states', () => {
  test('populated: where you stand, three actions, remotes; no axe violations', async ({ page }) => {
    await boot(page)
    const state = page.getByTestId('sync-state')
    await expect(state).toContainText('main')
    await expect(state).toContainText('origin/main')
    await expect(state.getByLabel('3 versions to send')).toBeVisible()
    await expect(state.getByLabel('0 versions to get')).toBeVisible()
    await expect(state).toContainText('Not yet from this app')
    for (const n of ['Fetch', 'Get latest', 'Send']) await expect(page.getByRole('button', { name: n, exact: true })).toBeEnabled()
    await expect(page.locator('[data-remote="origin"]')).toContainText('https://mediagit.studio.example/hero-film')
    await expectNoAxeViolations(page)
  })

  test('never-fetched upstream shows -- instead of inventing a count', async ({ page }) => {
    const s = fixture<{ branch: { upstream: object } }>('status_populated')
    await boot(page, { repo_status: { ...s, branch: { ...s.branch, upstream: { name: 'origin/main', ahead: null, behind: null } } } })
    await expect(page.getByTestId('sync-state').getByLabel('Unknown until you fetch')).toHaveCount(2)
    await expect(page.getByTestId('sync-state')).toContainText('Fetch to find out')
  })

  test('loading, empty and error states', async ({ page }) => {
    await boot(page, {}, { remote_list: [{ hold: true }, { resolve: fixture('remotes') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(page.getByTestId('sync-state')).toBeVisible()
  })

  test('empty: no remote yet', async ({ page }) => {
    await boot(page, { remote_list: { remotes: [] } })
    await expect(page.getByText('No remote yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('error: code + message and Retry', async ({ page }) => {
    await boot(page, { remote_list: { __error: { code: 'error', message: 'Cannot read remotes.' } } })
    await expect(page.getByRole('alert')).toContainText('Cannot read remotes.')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'remote_list')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_list')).length).toBeGreaterThan(before)
  })
})

test.describe('Sync: transfers', () => {
  test('send: indeterminate first, then determinate bytes with rate/ETA unknown until measured; cancel stops it', async ({ page }) => {
    await boot(page, {}, { repo_push: fixture('push_progress') })
    await page.getByRole('button', { name: 'Send', exact: true }).click()
    await expect(run(page)).toContainText('Send origin')
    const preparing = run(page).getByRole('progressbar', { name: 'Preparing' })
    await expect(preparing).toBeVisible()
    await expect(preparing).not.toHaveAttribute('aria-valuenow', /.*/)
    // controls are locked while it runs
    await expect(page.getByRole('button', { name: 'Fetch', exact: true })).toBeDisabled()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    const up = run(page).getByRole('progressbar', { name: 'Uploading media' })
    await expect(up).toHaveAttribute('aria-valuenow', '200000')
    await expect(run(page).getByRole('progressbar', { name: 'Preparing' })).toHaveCount(0) // finished bars are hidden
    await expect(run(page)).toContainText('ETA --')
    await run(page).getByRole('button', { name: 'Cancel' }).click()
    await expect(run(page)).toHaveCount(0)
    await expect(page.getByRole('alert')).toHaveCount(0) // cancelling is not an error
    expect(await ipcCalls(page, 'cancel_op')).toHaveLength(1)
    await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled()
  })

  test('get latest (pull) shows progress but offers no Cancel anywhere (issue 93)', async ({ page }) => {
    await boot(page, {}, { repo_pull: [{ progress: { id: 1, msg: 'Downloading media', pos: 10, len: 100, unit: 'bytes', done: false } }, { hold: true }, { resolve: { remote: 'origin', result: 'up_to_date', objects_received: 0, bytes_downloaded: 0 } }] })
    await page.getByRole('button', { name: 'Get latest', exact: true }).click()
    await expect(run(page)).toContainText('Downloading media')
    await expect(run(page).getByRole('button', { name: 'Cancel' })).toHaveCount(0)
    await bar(page).getByRole('button', { name: 'Activity' }).click()
    await expect(page.getByTestId('activity-drawer').getByRole('button', { name: /Cancel/ })).toHaveCount(0)
    await expect(bar(page).locator('button', { hasText: 'Cancel' })).toHaveCount(0)
    await ipcRelease(page)
  })

  test('send to completion toasts a summary and stamps last synced', async ({ page }) => {
    await boot(page, {}, { repo_push: fixture('push_progress') })
    await page.getByRole('button', { name: 'Send', exact: true }).click()
    for (let i = 0; i < 3; i++) {
      await page.waitForTimeout(150)
      await ipcRelease(page)
    }
    await expect(page.getByText('Sent 14 objects, 1.9 MB').first()).toBeVisible()
    await expect(page.getByTestId('sync-state')).toContainText('just now')
    expect((await ipcCalls(page, 'repo_push'))[0]!.args).toMatchObject({ repo: '/repos/hero-film', remote: 'origin' })
  })

  test('failure: message in a banner and a Retry row in the activity drawer', async ({ page }) => {
    await boot(page, {}, { repo_push: fixture('push_fail') })
    await page.getByRole('button', { name: 'Send', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('The server refused the push: main is protected.')
    await bar(page).getByRole('button', { name: 'Activity' }).click()
    const drawer = page.getByTestId('activity-drawer')
    await expect(drawer.getByRole('heading', { name: 'Failed' })).toBeVisible()
    await drawer.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_push')).length).toBe(2)
  })

  test('fetch and get latest call the right commands', async ({ page }) => {
    await boot(page, { repo_fetch: fixture('fetch_ok'), repo_pull: { remote: 'origin', result: 'up_to_date', objects_received: 0, bytes_downloaded: 0 } })
    await page.getByRole('button', { name: 'Fetch', exact: true }).click()
    await expect(page.getByText('Fetched 1 update, 3.2 MB').first()).toBeVisible()
    await page.getByRole('button', { name: 'Get latest', exact: true }).click()
    await expect(page.getByText('Already up to date').first()).toBeVisible()
    expect(await ipcCalls(page, 'repo_fetch')).toHaveLength(1)
    expect(await ipcCalls(page, 'repo_pull')).toHaveLength(1)
  })
})

test.describe('Sync: remotes', () => {
  test('add: validates name and address, then adds', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Add remote' }).click()
    const dlg = page.getByRole('dialog', { name: 'Add remote' })
    await dlg.getByLabel('Name').fill('origin')
    await dlg.getByLabel('Address').fill('ftp://nope')
    await expect(dlg).toContainText('A remote with this name exists')
    await expect(dlg).toContainText('Use an http(s) address or a local folder path')
    await expect(dlg.getByRole('button', { name: 'Add remote' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Name').fill('backup')
    await dlg.getByLabel('Address').fill('https://backup.example/hero')
    await dlg.getByRole('button', { name: 'Add remote' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_add')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', name: 'backup', url: 'https://backup.example/hero' },
    ])
  })

  test('remove asks first and reassures that nothing is deleted on the server', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Remove origin' }).click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Nothing is deleted on the server')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'remote_remove')).toHaveLength(0)
    await page.getByRole('button', { name: 'Remove origin' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Remove remote' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_remove')).map((c) => c.args.name)).toEqual(['origin'])
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Fetch', exact: true }).focus()
    await expectFocusRing(page)
  })
})
