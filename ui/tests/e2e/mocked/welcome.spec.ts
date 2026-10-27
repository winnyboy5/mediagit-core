import { expect, test } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const withRecent = { 'mg.recent': RECENT }

test.describe('Welcome', () => {
  test('renders actions, recent list and footer without app chrome; no axe violations', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: withRecent })
    await page.goto('/#/welcome')
    await expect(page.getByRole('heading', { name: 'Welcome' })).toBeVisible()
    for (const n of [/Open folder/, /Clone/, /New repository/]) await expect(page.getByRole('button', { name: n })).toBeVisible()
    await expect(page.getByTestId('recent-list').getByRole('listitem')).toHaveCount(2)
    await expect(page.getByText('mediagit 0.7.0')).toBeVisible()
    await expect(page.getByRole('navigation', { name: 'Main' })).toHaveCount(0)
    await expectNoAxeViolations(page)
  })

  test('empty recent list explains what to do', async ({ page }) => {
    await mockIpc(page, { commands: healthy() })
    await page.goto('/#/welcome')
    await expect(page.getByText('No repositories yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('keyboard: first Tab lands on Open folder with a focus ring; Enter on a recent repo opens it', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: withRecent })
    await page.goto('/#/welcome')
    await expect(page.getByRole('heading', { name: 'Welcome' })).toBeVisible()
    await page.keyboard.press('Tab')
    await expect(page.getByRole('button', { name: /Open folder/ })).toBeFocused()
    await expectFocusRing(page)
    await page.getByRole('button', { name: /^hero-film/ }).focus()
    await page.keyboard.press('Enter')
    await expect(page).toHaveURL(/#\/changes$/)
    expect((await ipcCalls(page, 'repo_status'))[0]!.args).toEqual({ repo: '/repos/hero-film' })
  })

  test('Open folder uses the picker; cancelling it does nothing', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), dialog: '/repos/picked' })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /Open folder/ }).click()
    await expect(page).toHaveURL(/#\/changes$/)
    expect((await ipcCalls(page, 'repo_status'))[0]!.args).toEqual({ repo: '/repos/picked' })
  })

  test('cancelled picker stays on Welcome', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), dialog: null })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /Open folder/ }).click()
    await expect(page.getByRole('heading', { name: 'Welcome' })).toBeVisible()
    expect(await ipcCalls(page, 'repo_status')).toHaveLength(0)
  })

  test('a folder that is not a repository shows code + message with Retry, and Ctrl+O opens the picker', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ repo_status: fixture('status_error') }), dialog: '/not/a/repo' })
    await page.goto('/#/welcome')
    await expect(page.getByRole('heading', { name: 'Welcome' })).toBeVisible()
    await page.keyboard.press('Control+o')
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('Not a MediaGit repository')
    await expect(alert).not.toContainText('error')
    await expect(page).toHaveURL(/#\/welcome$/)
    await expectNoAxeViolations(page)
    await expect(alert.getByRole('button', { name: 'Retry' })).toHaveCount(0)
  })

  test('remove from recent', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: withRecent })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: 'Remove hero-film from recent' }).click()
    await expect(page.getByTestId('recent-list').getByRole('listitem')).toHaveCount(1)
  })

  test('New repository: init error is shown, not faked', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ repo_init: fixture('unsupported') }), dialog: '/repos/new' })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /New repository/ }).click()
    await expect(page.getByRole('alert')).toContainText('json_unsupported')
    expect((await ipcCalls(page, 'repo_init'))[0]!.args).toEqual({ path: '/repos/new' })
    await expect(page).toHaveURL(/#\/welcome$/)
  })

  test('New repository success opens it', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ repo_init: { ok: true } }), dialog: '/repos/new' })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /New repository/ }).click()
    await expect(page).toHaveURL(/#\/changes$/)
  })
})

test.describe('Startup gate', () => {
  test('contract mismatch blocks the whole app', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ engine_info: fixture('engine_mismatch') }) })
    await page.goto('/#/welcome')
    await expect(page.getByRole('alert')).toContainText('contract_mismatch')
    await expect(page.getByRole('alert')).toContainText('different versions')
    await expect(page.getByRole('heading', { name: 'Welcome' })).toHaveCount(0)
    await expectNoAxeViolations(page)
  })

  test('missing engine blocks, Retry re-checks', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ engine_info: fixture('engine_missing') }) })
    await page.goto('/#/welcome')
    await expect(page.getByRole('alert')).toContainText('cli_not_found')
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'engine_info')).length).toBe(2)
  })
})

test.describe('Clone dialog', () => {
  const open = async (page: import('@playwright/test').Page) => {
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /Clone/ }).click()
    await expect(page.getByRole('dialog')).toBeVisible()
  }
  const fill = async (page: import('@playwright/test').Page) => {
    await page.getByLabel('Repository URL').fill('https://studio.example/hero-film')
    await page.getByLabel('Save into').fill('/repos/hero-film')
  }

  test('validates, is keyboard reachable, closes on Escape; no axe violations', async ({ page }) => {
    await mockIpc(page, { commands: healthy() })
    await open(page)
    const submit = page.getByRole('dialog').getByRole('button', { name: 'Clone', exact: true })
    await expect(submit).toBeDisabled()
    await page.getByLabel('Repository URL').fill('ftp://nope')
    await expect(page.getByText('Use an http(s) address')).toBeVisible()
    await expectNoAxeViolations(page)
    await fill(page)
    await expect(submit).toBeEnabled()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })

  test('Browse fills the destination from the picker', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), dialog: '/picked/dir' })
    await open(page)
    await page.getByRole('button', { name: 'Browse…' }).click()
    await expect(page.getByLabel('Save into')).toHaveValue('/picked/dir')
  })

  test('progress: no length -> indeterminate; rate/ETA stay -- until samples are 1 s apart; completes into Changes', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), scripts: { repo_clone: fixture('clone_progress') } })
    await open(page)
    await fill(page)
    await page.getByRole('dialog').getByRole('button', { name: 'Clone', exact: true }).click()
    const dlg = page.getByTestId('clone-progress')
    const bar = dlg.getByRole('progressbar').first()
    await expect(bar).toBeVisible()
    await expect(bar).not.toHaveAttribute('aria-valuenow', /.*/) // length unknown: indeterminate
    await ipcRelease(page)
    await expect(dlg.getByRole('progressbar', { name: 'Downloading media' })).toHaveAttribute('aria-valuenow', '100000')
    await expect(dlg.getByText('ETA --').last()).toBeVisible()
    await page.waitForTimeout(1100)
    await ipcRelease(page) // lump of progress 1.1 s later
    await expect(dlg.getByText(/\/s$/).first()).toBeVisible()
    await expect(dlg.getByText('ETA --')).toHaveCount(0) // the finished first bar is hidden; the live one has a rate by now
    await expect(dlg.getByRole('progressbar', { name: 'Connecting to server' })).toHaveCount(0)
    await ipcRelease(page)
    await ipcRelease(page)
    await expect(page).toHaveURL(/#\/changes$/)
    expect((await ipcCalls(page, 'repo_clone'))[0]!.args).toMatchObject({ url: 'https://studio.example/hero-film', dest: '/repos/hero-film' })
  })

  test('failure shows code + message and Retry', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), scripts: { repo_clone: fixture('clone_fail') } })
    await open(page)
    await fill(page)
    await page.getByRole('dialog').getByRole('button', { name: 'Clone', exact: true }).click()
    const alert = page.getByRole('dialog').getByRole('alert')
    await expect(alert).toContainText('E_NET_TIMEOUT')
    await expect(alert).toContainText('Server did not answer within 60 s')
    await expectNoAxeViolations(page)
  })

  test('unsupported backend is reported as an error, never as success', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ repo_clone: fixture('unsupported') }) })
    await open(page)
    await fill(page)
    await page.getByRole('dialog').getByRole('button', { name: 'Clone', exact: true }).click()
    await expect(page.getByRole('dialog').getByRole('alert')).toContainText('json_unsupported')
    await expect(page).toHaveURL(/#\/welcome$/)
  })

  test('Cancel clone stops it and returns to the form', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), scripts: { repo_clone: fixture('clone_cancel') } })
    await open(page)
    await fill(page)
    await page.getByRole('dialog').getByRole('button', { name: 'Clone', exact: true }).click()
    await page.getByRole('button', { name: 'Cancel clone' }).click()
    expect(await ipcCalls(page, 'cancel_op')).toHaveLength(1)
    await expect(page.getByLabel('Repository URL')).toBeVisible()
    await expect(page.getByRole('dialog').getByRole('alert')).toHaveCount(0) // cancelling is not an error
  })
})
