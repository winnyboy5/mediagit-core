import { mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { attach, mg, openRecent, startServer, tmp, type Ctx, type Srv } from './bugs'

// BUG-045, real app: Create account and Change password with the address empty use the repository's remote.
let c: Ctx
let srv: Srv
test.beforeAll(async () => {
  c = await attach()
  srv = await startServer(c.st, 'acct-repo', true)
})
test.afterAll(async () => {
  // The CLI keeps sign-ins in the OS keychain: take this one out again.
  try { mg(c.st, c.st.root, ['--machine', 'auth', 'logout', `--server=${srv.url}`]) } catch { /* never signed in */ }
  srv?.stop()
  await c?.browser.close().catch(() => {})
})

test('BUG-045: both dialogs work with the address field empty', async () => {
  const { st, page } = c
  const repo = join(tmp('bug045'), 'bug045')
  mkdirSync(repo)
  mg(st, repo, ['init'])
  mg(st, repo, ['remote', 'add', 'origin', `${srv.url}/acct-repo`])
  await openRecent(page, repo, 'bug045')
  await page.evaluate(() => { window.location.hash = '#/settings/security' })

  const OLD = 'Old-Quartz-Lantern-11!'
  const NEW = 'New-Harbor-Violet-22!'
  await page.getByRole('button', { name: 'Create account…' }).click()
  let dlg = page.getByRole('dialog', { name: 'Create account' })
  await dlg.getByLabel('Username').fill('e2eacct')
  await dlg.getByLabel('Email').fill('e2eacct@example.test')
  await dlg.getByLabel('Password', { exact: true }).fill(OLD)
  await dlg.getByLabel('Repeat password').fill(OLD)
  await dlg.getByRole('button', { name: 'Create account' }).click()
  await expect(dlg).toHaveCount(0)
  await expect(page.getByRole('alert')).toHaveCount(0)
  // Creating an account signs you in to it: there is no separate Sign in step.
  await expect(page.getByTestId('account')).toContainText('Signed in as e2eacct')

  await page.getByRole('button', { name: 'Change password…' }).click()
  dlg = page.getByRole('dialog', { name: 'Change password' })
  await dlg.getByLabel('Current password').fill(OLD)
  await dlg.getByLabel('New password', { exact: true }).fill(NEW)
  await dlg.getByLabel('Repeat new password').fill(NEW)
  await dlg.getByRole('button', { name: 'Change password' }).click()
  await expect(dlg).toHaveCount(0)
  // The dialog closes at once; the server change lands after. Wait for the app's own confirmation.
  await expect(page.getByText('Password changed').first()).toBeVisible()
  await expect(page.getByRole('alert')).toHaveCount(0)

  const login = (pw: string) => mg(st, repo, ['--machine', '--secret-stdin', 'auth', 'login', `--server=${srv.url}`, '--username', 'e2eacct'], `${pw}\n`)
  expect(() => login(OLD)).toThrow()
  expect(() => login(NEW)).not.toThrow()
})
