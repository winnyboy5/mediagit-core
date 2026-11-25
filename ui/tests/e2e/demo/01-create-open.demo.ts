import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import './demo-env'
import { startChapter } from './demo'
import { copyMedia, folder } from './seed'
import { answerPicker, toWelcome } from '../qa-live/gui'

// Chapter 01: create a project from a folder that already holds files, then reopen it from Recent.
test('01 Create and open a project', async () => {
  const ch = await startChapter('01', 'Create and open a project')
  const { page } = ch
  // Seed after startChapter: it wipes the per-chapter work folder.
  const dir = folder('dragon-keep')
  writeFileSync(join(dir, 'shot_notes.txt'), 'Dragon Keep - opening shot\nplate 010, grade pending\n')
  copyMedia(dir, '3D_model_of_Shucaris_ankylosskelos_appendage.png')
  try {
    await toWelcome(page, true)
    await ch.caption('MediaGit keeps every version of your big creative files', 3500)

    await answerPicker(page, dir)
    await ch.caption('Start a project from any folder, even one that already has files', 3000)
    await page.getByRole('button', { name: 'New repository (init)' }).click()
    const dlg = page.getByRole('dialog', { name: 'Create repository' })
    await dlg.waitFor()
    await ch.pace(1200)
    await dlg.getByText('More options').click()
    await ch.pace(600)
    await ch.type(dlg.getByLabel('First branch name'), 'main')
    await ch.caption('Pick a name for the main line of work, or just keep the default', 3000)
    await dlg.getByRole('button', { name: 'Create' }).click()
    await page.getByRole('navigation', { name: 'Main' }).waitFor({ timeout: 20_000 })
    await ch.pace(1000)
    await ch.caption('Done. Your existing files show up right away, nothing is moved or changed', 4000)

    await toWelcome(page)
    await ch.caption('Every project you open is remembered here', 3000)
    await page.getByTestId('recent-list').locator('button.open', { hasText: 'dragon-keep' }).first().click()
    await page.getByRole('navigation', { name: 'Main' }).waitFor({ timeout: 30_000 })
    await ch.pace(800)
    await ch.caption('One click and you are back in your project', 3500)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
