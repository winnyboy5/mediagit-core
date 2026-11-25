import { copyFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import './demo-env'
import { CORPUS, WORK } from './demo-env'
import { startChapter } from './demo'
import { cli, folder, seedRepo } from './seed'
import { startServer, type LocalServer } from '../qa-live/server'
import { go, openRepo, refocus, toWelcome } from '../qa-live/gui'

// Chapter 10: add a remote, send a ~275 MB video with the progress bar, a teammate sends a change, get latest,
// lock and unlock a file, browse the remote files. Uses a harness server (no sign-in) that this chapter stops again.
const BIG = 'video-implementation/DW0001C001_251020_111956_c1I7H.mxf'
const busy = (page: Page) => page.waitForFunction(() => !document.querySelector('[data-testid="transfer-progress"]') && !document.querySelector('[data-activity-pill] [role="progressbar"]'), null, { timeout: 180_000 })
const header = (page: Page, name: 'Get latest' | 'Send') => page.getByRole('group', { name: 'Share work' }).getByRole('button', { name: new RegExp(`^${name}`) }).first()

test('10 Share work with your team', async () => {
  const ch = await startChapter('10', 'Share work with your team')
  const { page } = ch
  const dlg = page.getByRole('dialog')
  let srv: LocalServer | undefined
  try {
    const dir = folder('dragon-keep')
    const ben = join(WORK, 'demo-ben')
    const srvDir = join(WORK, 'demo-srv')
    rmSync(ben, { recursive: true, force: true })
    rmSync(srvDir, { recursive: true, force: true })
    writeFileSync(join(dir, 'shot_notes.txt'), 'Dragon Keep - opening shot\nplate 010, grade pending\n')
    copyFileSync(join(CORPUS, BIG), join(dir, 'plate_010.mxf'))
    seedRepo(dir, 'First look')
    srv = await startServer(srvDir, ['dragon-keep'])
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'branches/sync')
    await ch.pace(1500)
    await ch.caption('To work together, connect your project to a shared place', 3500)
    await page.getByRole('button', { name: 'Add remote' }).first().click()
    await dlg.waitFor()
    await dlg.getByLabel('Name').fill('')
    await ch.type(dlg.getByLabel('Name'), 'origin')
    await dlg.getByLabel('Address').fill(srv!.url('dragon-keep'))
    await ch.pace(1200)
    await dlg.getByRole('button', { name: 'Add remote' }).click()
    await dlg.waitFor({ state: 'hidden' })
    await ch.pace(1000)

    await header(page, 'Send').click()
    await page.getByTestId('transfer-progress').waitFor({ timeout: 30_000 }).catch(() => {})
    const sending = ch.caption('Send uploads your new versions, with live progress for big files', 6000)
    await busy(page)
    await sending
    await ch.pace(1500)
    await ch.caption('Done. Your teammates can now get everything you sent', 3000)

    // A teammate gets the project and sends one new version.
    const meanwhile = ch.caption('Meanwhile, a teammate gets the project and adds their own notes', 12_000)
    await ch.pace(400)
    cli(WORK, ['clone', srv!.url('dragon-keep'), ben])
    writeFileSync(join(ben, 'grade_notes.txt'), 'Warmer highlights on plate 010\n')
    cli(ben, ['add', '--all'])
    cli(ben, ['commit', '-m', 'Grade notes'])
    cli(ben, ['push', 'origin', 'main'])
    await meanwhile
    await refocus(page)
    await ch.pace(1500)
    await page.getByRole('button', { name: 'Fetch' }).first().click()
    await ch.caption('A teammate has sent a new version. The header shows there is something to get', 3500)
    await header(page, 'Get latest').click()
    await ch.pace(1200)
    await busy(page)
    await ch.pace(800)
    await ch.caption('Get latest brings their work into your project', 3000)

    await go(page, 'team/locks')
    await ch.pace(1500)
    await page.getByRole('button', { name: 'Lock a file' }).first().click()
    await dlg.waitFor()
    await ch.type(dlg.getByLabel('File path'), 'plate_010.mxf')
    await dlg.getByRole('button', { name: 'Lock file' }).click()
    await ch.pace(2500)
    if (!(await page.locator('tr[data-lock]').count())) { // a server without sign-in needs a name for the lock
      if (!(await dlg.isVisible().catch(() => false))) {
        await page.getByRole('button', { name: 'Lock a file' }).first().click()
        await dlg.getByLabel('File path').fill('plate_010.mxf')
      }
      const more = page.getByTestId('lock-more')
      await more.locator('summary, button').first().click()
      await more.getByLabel('Lock as').fill('you')
      await dlg.getByRole('button', { name: 'Lock file' }).click()
      await ch.pace(2500)
    }
    await ch.caption('Lock a file while you work on it, so nobody overwrites your changes', 3500)
    await page.locator('tr[data-lock="plate_010.mxf"]').getByRole('button', { name: 'Unlock' }).click()
    await ch.pace(2000)
    await ch.caption('Unlock it when you are done', 2500)

    await go(page, 'team/remote')
    await ch.pace(2000)
    await ch.caption('Browse everything on the shared place without downloading it', 3000)
    await page.getByLabel('Remote folders and files').getByRole('button', { name: 'shot_notes.txt' }).click()
    await ch.pace(2500)
  } finally {
    srv?.stop()
    console.log('clip:', await ch.stop())
  }
})
