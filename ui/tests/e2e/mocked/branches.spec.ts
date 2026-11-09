import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const row = (page: Page, name: string) => page.locator(`[data-branch="${name}"]`)

async function boot(page: Page, commands: Record<string, unknown> = {}, scripts: Record<string, unknown[]> = {}) {
  await mockIpc(page, { commands: healthy({ branch_list: fixture('branches'), ...commands }), storage: REPO, scripts: scripts as never })
  await page.goto('/#/branches', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Branches', level: 1 })).toBeVisible()
}

test.describe('Branches: states', () => {
  test('populated: current and protected badges, ahead/behind, actions; no axe violations', async ({ page }) => {
    await boot(page)
    await expect(page.getByRole('list', { name: 'Branches' }).getByRole('listitem')).toHaveCount(3)
    const main = row(page, 'main')
    await expect(main).toContainText('Current')
    await expect(main).toContainText('Protected')
    await expect(main.getByLabel('3 ahead, 0 behind origin/main')).toBeVisible()
    await expect(row(page, 'review/cut-2').getByLabel('0 ahead, 3 behind origin/review/cut-2')).toBeVisible()
    await expect(row(page, 'feature/grade')).toContainText('Not shared yet')
    await expect(main.getByRole('button', { name: 'Switch to', exact: true })).toHaveCount(0)
    await expect(main.getByRole('button', { name: /Delete main/ })).toBeDisabled()
    await expect(row(page, 'feature/grade').getByRole('button', { name: 'Delete feature/grade' })).toBeEnabled()
    await expectNoAxeViolations(page)
  })

  test('loading, empty and error states', async ({ page }) => {
    await boot(page, {}, { branch_list: [{ hold: true }, { resolve: fixture('branches') }] })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(row(page, 'main')).toBeVisible()
  })

  test('empty', async ({ page }) => {
    await boot(page, { branch_list: { current: null, branches: [] } })
    await expect(page.getByText('No branches yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('error: code + message and Retry', async ({ page }) => {
    await boot(page, { branch_list: { __error: { code: 'error', message: 'Cannot read branches.' } } })
    await expect(page.getByRole('alert')).toContainText('Cannot read branches')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'branch_list')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'branch_list')).length).toBeGreaterThan(before)
  })
})

test.describe('Branches: actions', () => {
  test('switch shows a determinate items overlay, then the activity toast', async ({ page }) => {
    await boot(page, {}, { branch_switch: fixture('branch_switch_progress') })
    await row(page, 'feature/grade').getByRole('button', { name: 'Switch to', exact: true }).click()
    const overlay = page.getByTestId('switch-progress')
    await expect(overlay).toContainText('Switch to feature/grade')
    await expect(overlay.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0')
    await expect(overlay.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '40')
    await expect(overlay).toContainText('Updating files')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(overlay.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '18')
    await expect(overlay).toContainText('18 / 40 files')
    await ipcRelease(page)
    await expect(overlay).toHaveCount(0)
    await expect(page.getByText('Switched to feature/grade').first()).toBeVisible()
    expect((await ipcCalls(page, 'branch_switch'))[0]!.args).toMatchObject({ repo: '/repos/hero-film', name: 'feature/grade' })
  })

  test('a dirty working tree refusal is shown plainly and dismissable', async ({ page }) => {
    await boot(page, {}, { branch_switch: fixture('branch_switch_dirty') })
    await row(page, 'review/cut-2').getByRole('button', { name: 'Switch to', exact: true }).click()
    const alert = page.getByRole('alert')
    await expect(alert).toContainText('You have unsaved changes in 3 files.')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Dismiss' }).click()
    await expect(alert).toHaveCount(0)
    await expect(row(page, 'review/cut-2').getByRole('button', { name: 'Switch to', exact: true })).toBeEnabled()
  })

  test('create: validates the name, then creates and switches', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'New branch' }).click()
    const dlg = page.getByRole('dialog', { name: 'New branch' })
    const name = dlg.getByLabel('Branch name')
    const create = dlg.getByRole('button', { name: 'Create', exact: true })
    await expect(create).toBeDisabled()
    await name.fill('bad name')
    await expect(dlg).toContainText('Cannot contain spaces')
    await name.fill('main')
    await expect(dlg).toContainText('A branch with this name exists')
    await name.fill('--force')
    await expect(dlg).toContainText('Cannot start with a dash')
    await expectNoAxeViolations(page)
    await name.fill('grade-v2')
    await expect(create).toBeEnabled()
    await dlg.getByRole('button', { name: 'Create and switch' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'branch_create')).map((c) => c.args.name)).toEqual(['grade-v2'])
    await expect.poll(async () => (await ipcCalls(page, 'branch_switch')).map((c) => c.args.name)).toEqual(['grade-v2'])
    await expect(dlg).toHaveCount(0)
  })

  test('create without switching leaves the tree alone; Escape closes the dialog', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'New branch' }).click()
    const dlg = page.getByRole('dialog', { name: 'New branch' })
    await dlg.getByLabel('Branch name').fill('layout')
    await dlg.getByRole('button', { name: 'Create', exact: true }).click()
    await expect.poll(async () => (await ipcCalls(page, 'branch_create')).length).toBe(1)
    expect(await ipcCalls(page, 'branch_switch')).toHaveLength(0)
    await page.getByRole('button', { name: 'New branch' }).click()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })

  test('rename: same name refused, new name sent', async ({ page }) => {
    await boot(page)
    await row(page, 'feature/grade').getByRole('button', { name: 'Rename feature/grade' }).click()
    const dlg = page.getByRole('dialog', { name: 'Rename feature/grade' })
    await expect(dlg.getByLabel('New name')).toHaveValue('feature/grade')
    await expect(dlg).toContainText('Choose a different name')
    await expect(dlg.getByRole('button', { name: 'Rename', exact: true })).toBeDisabled()
    await dlg.getByLabel('New name').fill('feature/grade-final')
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    await expect.poll(async () => (await ipcCalls(page, 'branch_rename')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', from: 'feature/grade', to: 'feature/grade-final' },
    ])
  })

  test('delete asks first and explains; cancel changes nothing', async ({ page }) => {
    await boot(page)
    const del = row(page, 'feature/grade').getByRole('button', { name: 'Delete feature/grade' })
    await del.click()
    const dlg = page.getByRole('dialog', { name: 'Delete feature/grade?' })
    await expect(dlg).toContainText('already part of another branch')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'branch_delete')).toHaveLength(0)
    await del.click()
    await page.getByRole('dialog', { name: 'Delete feature/grade?' }).getByRole('button', { name: 'Delete branch' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'branch_delete')).map((c) => c.args)).toEqual([
      { repo: '/repos/hero-film', name: 'feature/grade', opts: { mergedOnly: true } },
    ])
  })

  test('buttons are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await row(page, 'feature/grade').getByRole('button', { name: 'Switch to', exact: true }).focus()
    await expectFocusRing(page)
    await page.keyboard.press('Tab')
    await expect(row(page, 'feature/grade').getByRole('button', { name: 'Details of feature/grade' })).toBeFocused()
    await expectFocusRing(page)
  })
})
