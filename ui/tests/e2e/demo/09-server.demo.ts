import { test } from '@playwright/test'
import './demo-env'
import { startChapter } from './demo'
import { copyMedia, folder, seedRepo } from './seed'
import { choose, go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 09: set up and start a local server from the app, add a person with an access level, make an API key.
test('09 Run your own server', async () => {
  const ch = await startChapter('09', 'Run your own server')
  const { page } = ch
  const dlg = page.getByRole('dialog')
  const tab = async (name: string) => { await page.getByLabel('Server pages').getByRole('button', { name, exact: true }).click(); await ch.pace(1200) }
  try {
    const dir = folder('hero-film')
    copyMedia(dir, '3D_model_of_Shucaris_ankylosskelos_appendage.png')
    seedRepo(dir, 'First look')
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'team/server')
    await ch.pace(2000)
    await ch.caption('Want to keep everything on your own machine? MediaGit can run its own server', 3500)

    await page.getByRole('button', { name: 'Set up local server…' }).click()
    await dlg.waitFor()
    await ch.pace(800)
    await ch.type(dlg.getByLabel('Admin username'), 'studio-admin')
    await ch.type(dlg.getByLabel('Admin email'), 'admin@studio.test')
    await dlg.getByLabel('Password', { exact: true }).fill('Studio!Demo-2026')
    await dlg.getByLabel('Repeat password').fill('Studio!Demo-2026')
    await ch.caption('Choose an administrator name and password. That is all the setup there is', 3500)
    await dlg.getByRole('button', { name: 'Set up and start' }).click()
    await page.waitForFunction(() => /running/i.test((document.querySelector('[data-testid="local-state"]') as HTMLElement | null)?.innerText ?? ''), null, { timeout: 60_000 })
    await ch.pace(1200)
    await ch.caption('Your server is running, and only this computer can reach it by default', 3500)

    await tab('People')
    await page.getByRole('button', { name: 'Add person…' }).click()
    await dlg.waitFor()
    await ch.type(dlg.getByLabel('Username'), 'maya')
    await ch.type(dlg.getByLabel('Email'), 'maya@studio.test')
    await choose(page, 'What they can do', /Can look/, 500)
    await dlg.getByLabel('Starting password').fill('Welcome!Pass-2026')
    await ch.caption('Add your people and decide what each one may do: just look, or change files', 4000)
    await dlg.getByRole('button', { name: 'Add person' }).click()
    await dlg.waitFor({ state: 'hidden', timeout: 15_000 }).catch(async () => { console.log('add-person dialog stuck:', (await dlg.innerText()).replace(/\s+/g, ' ').slice(0, 400)); throw new Error('Add person dialog did not close') })
    await ch.pace(2500)
    await ch.caption('Maya can now sign in and browse, but not change anything', 3000)

    await tab('API keys')
    await page.getByRole('button', { name: 'New key…' }).click()
    await dlg.waitFor()
    await ch.type(dlg.getByLabel('Name'), 'render machine')
    await ch.caption('Machines get their own keys, so nobody has to share a password', 3500)
    await dlg.getByRole('button', { name: 'Make key' }).click()
    const key = page.getByTestId('new-key-value')
    await key.waitFor()
    await ch.blur(key)
    await ch.caption('A new key is shown only once, so save it somewhere safe', 3500)
    await page.getByText('I saved it').click()
    await page.getByRole('button', { name: 'Done' }).click()
    await ch.pace(1500)
    await ch.caption('The key is listed by name from now on, never in full', 3000)
  } finally {
    try {
      await tab('Overview')
      await page.getByRole('button', { name: 'Stop server…' }).click()
      await page.getByRole('button', { name: 'Stop server', exact: true }).click()
      await ch.pace(3000)
    } catch { /* server not running: nothing to stop */ }
    console.log('clip:', await ch.stop())
  }
})
