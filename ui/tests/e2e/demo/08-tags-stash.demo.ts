import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { startChapter } from './demo'
import { folder, seedRepo } from './seed'
import { go, openRepo, refocus, toWelcome } from '../qa-live/gui'

// Chapter 08: label an approved version, and set unfinished work aside for later.
test('08 Tags and stash', async () => {
  const ch = await startChapter('08', 'Tags and stash')
  const { page } = ch
  const dir = folder('ad-spot')
  writeFileSync(join(dir, 'notes.txt'), 'Ad spot\nfinal grade\n')
  seedRepo(dir, 'Client-approved cut')
  try {
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'history/tags')
    await ch.pace(800)
    await ch.caption('Label an important version so you can find it later', 2800)

    const form = page.getByTestId('new-tag')
    if (!(await form.getByLabel('Tag name').isVisible().catch(() => false))) await form.locator('summary, button').first().click()
    await ch.type(form.getByLabel('Tag name'), 'client-approved')
    await ch.type(form.getByLabel('Note (optional)'), 'Final delivery')
    await ch.pace(700)
    await ch.caption('Give it a name and a short note', 2500)
    await form.locator('button[type="submit"]').click()
    await ch.pace(1200)
    await ch.caption('Done. That version now carries its label', 2500)

    // Unfinished work in the folder.
    writeFileSync(join(dir, 'notes.txt'), 'Ad spot\nfinal grade\ntrying a darker look\n')
    await go(page, 'branches/stash')
    await refocus(page)
    await ch.pace(1000)
    await ch.caption('Half-done work in the way? Set it aside without saving a version', 2500)
    const section = page.locator('section', { has: page.getByRole('heading', { name: 'Stash' }) })
    await ch.type(section.getByLabel('Message (optional)'), 'Darker look, unfinished')
    await ch.pace(500)
    await section.locator('button[type="submit"]').click()
    await ch.pace(2500)
    await ch.caption('Your folder is back to the last saved version', 2500)

    await page.getByLabel('Stashed work').getByRole('button', { name: 'Pop stash 0' }).click()
    await ch.pace(2500)
    await ch.caption('Bring the unfinished work back whenever you are ready', 2800)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
