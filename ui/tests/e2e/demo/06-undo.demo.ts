import { copyFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { CORPUS } from './demo-env'
import { startChapter } from './demo'
import { cli, folder, seedRepo } from './seed'
import { go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 06: undo a version, move back in time, then recover through the undo log.
test('06 Undo safely', async () => {
  const ch = await startChapter('06', 'Undo safely')
  const { page } = ch
  const dir = folder('promo-cut')
  writeFileSync(join(dir, 'notes.txt'), 'Promo cut\nversion one\n')
  seedRepo(dir, 'First draft')
  copyFileSync(join(CORPUS, 'freepik__talk__71826.jpeg'), join(dir, 'cover.jpeg'))
  cli(dir, ['add', '--all']); cli(dir, ['commit', '-m', 'Add cover picture'])
  writeFileSync(join(dir, 'notes.txt'), 'Promo cut\nversion three\n')
  cli(dir, ['add', '--all']); cli(dir, ['commit', '-m', 'Rewrite notes'])
  const list = page.getByRole('listbox', { name: 'Versions, newest first' })
  const pick = async (text: string) => {
    await page.getByPlaceholder('Search versions').fill(text)
    await ch.pace(900)
    await list.getByRole('option').first().click()
    await ch.pace(900)
  }
  const undoMenu = async (item: RegExp) => {
    await page.getByRole('button', { name: 'Undo this version' }).first().click()
    await ch.pace(500)
    await page.getByRole('menuitem', { name: item }).click()
    await ch.pace(900)
  }
  try {
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'history/versions')
    await list.getByRole('option').first().waitFor({ timeout: 30_000 })
    await ch.caption('Made a mistake? Every version can be undone', 2500)

    await pick('Rewrite notes')
    await undoMenu(/adds a new one/)
    await ch.caption('Undo one version. History is kept, nothing is lost', 3000)
    await page.getByRole('dialog').getByRole('button', { name: /^Undo this version/ }).click()
    await ch.pace(2500)
    await page.getByPlaceholder('Search versions').fill('')
    await ch.pace(800)
    await ch.caption('The undo is saved as a new version on top', 3000)

    await pick('Add cover picture')
    await undoMenu(/in my files/)
    await ch.caption('Or move the project back to an earlier version, keeping your files', 3000)
    await page.getByRole('button', { name: 'Move branch back' }).click()
    await ch.pace(2500)

    await go(page, 'settings/advanced')
    await ch.pace(800)
    await page.getByRole('tab', { name: /undo log|reflog/i }).first().click()
    await ch.pace(800)
    await ch.caption('Went too far? The undo log remembers every move', 3000)
    await page.getByRole('button', { name: 'Restore to @1', exact: true }).click()
    await ch.pace(800)
    await ch.caption('Restore to how things were a moment ago', 3000)
    const dlg = page.locator('[role="dialog"], [role="alertdialog"]').first()
    const tick = dlg.getByRole('checkbox')
    if (await tick.count()) await tick.first().check().catch(() => dlg.locator('label').first().click())
    await dlg.getByRole('button').last().click()
    await ch.pace(2500)
    await ch.caption('Back where you were. Nothing lost', 2500)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
