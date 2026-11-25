import { rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import './demo-env'
import { startChapter } from './demo'
import { cli, copyMedia, folder } from './seed'
import { WORK } from './demo-env'
import { startServer, type LocalServer } from '../qa-live/server'
import { go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 12: turn on encryption (before the first version), clean up storage, check integrity, switch the theme.
test('12 Security and upkeep', async () => {
  const ch = await startChapter('12', 'Security and upkeep')
  const { page } = ch
  const dlg = page.getByRole('dialog')
  const enc = page.getByTestId('encryption')
  const gone = (id: string) => page.waitForFunction((t) => !document.querySelector(`[data-testid="${t}-progress"]`), id, { timeout: 120_000 })
  let srv: LocalServer | undefined
  try {
    const dir = folder('dragon-keep')
    cli(dir, ['init', '--initial-branch', 'main', dir]) // encryption can only be turned on while the project has no saved version
    const srvDir = join(WORK, 'demo-srv')
    rmSync(srvDir, { recursive: true, force: true })
    srv = await startServer(srvDir, ['dragon-keep']) // a reachable remote, so the Security view shows no error
    cli(dir, ['remote', 'add', 'origin', srv.url('dragon-keep')])
    writeFileSync(join(dir, 'shot_notes.txt'), 'Dragon Keep - opening shot\nplate 010, grade pending\n')
    copyMedia(dir, '3D_model_of_Shucaris_ankylosskelos_appendage.png')
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'settings/security')
    await ch.pace(2000)
    await ch.caption('Protect your work: turn on encryption before saving the first version', 3500)
    await enc.getByRole('button', { name: 'Turn on encryption…' }).click()
    await dlg.waitFor()
    await ch.pace(1500)
    await dlg.getByRole('button', { name: 'Turn on encryption', exact: true }).click()
    const code = page.getByTestId('recovery-code')
    await code.waitFor({ timeout: 30_000 })
    await ch.blur(code)
    await ch.caption('You get a recovery code once. Keep it safe: it is the way back in', 3500)
    await dlg.getByText('I saved it').click()
    await dlg.getByRole('button').last().click()
    await ch.pace(1500)
    cli(dir, ['add', '--all'])
    cli(dir, ['commit', '-m', 'First look'])

    await go(page, 'settings/maintenance')
    await ch.pace(2000)
    await ch.caption('Housekeeping is one click. First see how much space could be freed', 3500)
    const gc = page.locator('[data-task="gc"]')
    if (!(await gc.getByRole('checkbox', { name: 'Preview only' }).isChecked().catch(() => false))) await gc.getByText('Preview only').click()
    await gc.getByRole('button', { name: 'Preview clean-up' }).click()
    await page.getByTestId('gc-result').waitFor({ timeout: 120_000 })
    await ch.pace(1500)
    await gc.getByText('Preview only').click()
    await gc.getByRole('button', { name: 'Clean up…' }).click()
    await ch.caption('Then clean up, after a clear confirmation', 3000)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Clean up' }).click()
    await gone('gc')
    await ch.pace(1500)

    const fsck = page.locator('[data-task="fsck"]')
    await fsck.getByRole('button', { name: 'Run integrity check' }).click()
    await page.getByTestId('fsck-result').waitFor({ timeout: 120_000 })
    await ch.pace(800)
    await ch.caption('An integrity check proves every saved file is still intact', 3500)
    await ch.pace(1500)

    await go(page, 'settings/general')
    await ch.pace(1200)
    await page.getByLabel('Theme').getByText('Dark', { exact: true }).click()
    await ch.caption('Prefer a dark room? Switch the look in one click', 3000)
    await page.getByLabel('Theme').getByText('Light', { exact: true }).click()
    await ch.pace(1500)
  } finally {
    srv?.stop()
    console.log('clip:', await ch.stop())
  }
})
