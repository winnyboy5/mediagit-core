import { expect, test } from '@playwright/test'
import { fixture, healthy, ipcCalls, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const NOT_A_REPO = fixture<{ __error: unknown }>('status_error')

test.describe('BUG-005: the rail creates, clones and opens in place', () => {
  test('Create keeps the open repository, and Cancel returns to it', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: REPO, dialog: '/repos/new' })
    await page.goto('/#/files/all', { waitUntil: 'domcontentloaded' })
    await page.getByTestId('repo-switcher').click()
    await page.getByRole('menuitem', { name: 'Create a new repository…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Create repository' })
    await expect(dlg).toBeVisible()
    await expect(page).toHaveURL(/#\/files\/all$/)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg).toHaveCount(0)
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
    await expect(page).toHaveURL(/#\/files\/all$/)
    await expect(page.getByTestId('repo-switcher')).toContainText('hero-film')
  })

  test('Clone keeps the open repository, and Cancel returns to it', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), storage: REPO })
    await page.goto('/#/files/all', { waitUntil: 'domcontentloaded' })
    await page.getByTestId('repo-switcher').click()
    await page.getByRole('menuitem', { name: 'Clone a repository…' }).click()
    const dlg = page.getByRole('dialog', { name: /clone/i })
    await expect(dlg).toBeVisible()
    await expect(page).toHaveURL(/#\/files\/all$/)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg).toHaveCount(0)
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
  })

  test('Create from the rail makes the repository and opens it', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ repo_init: { ok: true } }), storage: REPO, dialog: '/repos/new' })
    await page.goto('/#/files/all', { waitUntil: 'domcontentloaded' })
    await page.getByTestId('repo-switcher').click()
    await page.getByRole('menuitem', { name: 'Create a new repository…' }).click()
    await page.getByRole('dialog', { name: 'Create repository' }).getByRole('button', { name: 'Create', exact: true }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_init')).length).toBe(1)
    await expect.poll(async () => (await ipcCalls(page, 'repo_status')).some((c) => (c.args as { repo: string }).repo === '/repos/new')).toBe(true)
  })

  test('Open on a folder that is not a repository says so and stays on the open one', async ({ page }) => {
    await mockIpc(page, {
      commands: healthy(),
      handlers: { repo_status: `(a) => (a.repo === '/not/a/repo' ? ${JSON.stringify(NOT_A_REPO)} : ${JSON.stringify(fixture('status_populated'))})` },
      storage: REPO,
      dialog: '/not/a/repo',
    })
    await page.goto('/#/files/all', { waitUntil: 'domcontentloaded' })
    await page.getByTestId('repo-switcher').click()
    await page.getByRole('menuitem', { name: 'Open a repository…' }).click()
    await expect(page.locator('.mg-d', { hasText: 'Not a MediaGit repository' })).toBeVisible()
    await expect(page).toHaveURL(/#\/files\/all$/)
    await expect(page.getByTestId('repo-switcher')).toContainText('hero-film')
  })
})
