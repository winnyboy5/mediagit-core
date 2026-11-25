import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import './demo-env'
import { startChapter } from './demo'
import { cli, copyMedia, folder } from './seed'
import { go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 03: review changes, include/exclude, write a note, save a version.
test('03 Review changes and save a version', async () => {
  const ch = await startChapter('03', 'Review changes and save a version')
  const { page } = ch
  try {
  const dir = folder('spring-spot')
  writeFileSync(join(dir, 'shot_list.txt'), 'Spring spot\nshot 1: harbour at dawn\n')
  cli(dir, ['init', '--initial-branch', 'main', dir])
  cli(dir, ['add', '--all'])
  cli(dir, ['commit', '-m', 'Shot list'])
  // New work since the last version: a render, a photo, an edited note.
  copyMedia(dir, '3D_model_of_Shucaris_ankylosskelos_appendage.png', 'freepik__talk__71826.jpeg')
  writeFileSync(join(dir, 'shot_list.txt'), 'Spring spot\nshot 1: harbour at dawn\nshot 2: market, wide\n')
  const tree = page.getByRole('tree', { name: 'Changes' })
  const dock = page.getByTestId('save-dock')
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'files/changes')
    await tree.getByRole('treeitem').first().waitFor({ timeout: 30_000 })
    await ch.pace(800)
    await ch.caption('Changes shows everything you added or edited since the last version', 3500)

    await ch.caption('Each file is marked New or Edited, so nothing is a surprise', 3500)
    await ch.caption('Tick the files that belong in this version', 3000)
    await tree.getByRole('checkbox', { name: 'Include shot_list.txt' }).click()
    await ch.pace(2000)
    await tree.getByRole('checkbox', { name: 'Include 3D_model_of_Shucaris_ankylosskelos_appendage.png' }).click()
    await ch.pace(2000)
    await ch.caption('Leave out anything that is not ready, it stays safely on your disk', 3500)
    await ch.pace(500)
    await ch.caption('Or include everything with one click', 2800)
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    await ch.pace(1200)

    await ch.caption('Describe what you did in a few words', 3500)
    await ch.type(dock.getByLabel('Message'), 'Add first render and the second shot')
    await ch.pace(2000)
    await ch.caption('Save a version, and this moment is kept for good', 3000)
    await page.getByRole('button', { name: 'Save version', exact: true }).click()
    await tree.getByRole('treeitem').first().waitFor({ state: 'detached', timeout: 60_000 }).catch(() => {})
    await ch.pace(2500)
    await ch.caption('Saved. The list is clear and you can keep working', 4500)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
