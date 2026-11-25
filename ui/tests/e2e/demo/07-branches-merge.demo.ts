import { copyFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { CORPUS } from './demo-env'
import { startChapter } from './demo'
import { cli, folder, seedRepo } from './seed'
import { choose, go, openRepo, refocus, toWelcome } from '../qa-live/gui'

// Chapter 07: try ideas on a branch, merge cleanly, then settle a clash between two pictures.
test('07 Branches and merging', async () => {
  const ch = await startChapter('07', 'Branches and merging')
  const { page } = ch
  const dir = folder('title-sequence')
  const put = (name: string, from: string) => copyFileSync(join(CORPUS, from), join(dir, name))
  const save = (msg: string) => { cli(dir, ['add', '--all']); cli(dir, ['commit', '-m', msg]) }
  put('banner.ai', 'creative-pairs/ai/banner_v1.ai')
  put('photo.jpeg', 'freepik__talk__71826.jpeg')
  writeFileSync(join(dir, 'notes.txt'), 'Title sequence\n')
  seedRepo(dir, 'Starting point')
  const dlg = page.getByRole('dialog')
  const startMerge = async (branch: string) => {
    await go(page, 'branches/merge'); await refocus(page); await ch.pace(1200); await page.reload(); await page.getByRole('button', { name: 'Start a merge' }).waitFor(); await ch.pace(800)
    await page.getByRole('button', { name: 'Start a merge' }).click()
    await ch.pace(700)
    await choose(page, 'Merge this branch', branch, 600)
    await dlg.getByRole('button', { name: 'Start merge' }).click()
    await ch.pace(1500)
  }
  try {
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'branches/branches')
    await ch.pace(1500)
    await ch.caption('A branch is a safe copy to try ideas without touching the main work', 4000)

    await page.getByRole('button', { name: 'New branch' }).click()
    await ch.pace(600)
    await ch.type(dlg.getByLabel('Branch name'), 'new-colours')
    await ch.pace(500)
    await dlg.getByRole('button', { name: 'Create and switch' }).click()
    await ch.pace(1000)
    await ch.caption('Create a branch and switch to it in one step', 3000)

    // Work done on the branch (the artist saves from their editor).
    put('banner.ai', 'creative-pairs/ai/banner_v2.ai')
    save('Warmer banner colours')
    await go(page, 'branches/branches')
    await refocus(page)
    await ch.pace(1500)
    await page.getByLabel('Branches', { exact: true }).locator('li', { hasText: 'main' }).getByRole('button', { name: 'Switch to' }).click()
    await ch.pace(1200)
    await ch.caption('Switch back to main and your files change with it', 3500)

    await startMerge('new-colours')
    await ch.caption('Happy with the idea? Merge it into the main work', 3500)
    await ch.pace(1200)

    // A real clash: both sides changed the same picture.
    cli(dir, ['branch', 'create', 'retouch']); cli(dir, ['branch', 'switch', 'retouch'])
    put('photo.jpeg', 'freepik__talk__72772.jpeg')
    save('Retouched photo')
    cli(dir, ['branch', 'switch', 'main'])
    put('photo.jpeg', 'Workstation_cube_lid_off.webp')
    save('Different photo on main')
    await startMerge('retouch')
    await ch.caption('Two people changed the same picture. MediaGit asks you to choose', 4000)

    const group = page.getByRole('group', { name: 'Choose a version of photo.jpeg' })
    await group.waitFor({ timeout: 20_000 })
    await ch.pace(800)
    await ch.caption('Keep mine, or keep theirs. Nothing is blended or damaged', 3500)
    await ch.skip(() => group.getByRole('button', { name: /^Keep / }).first().click())
    await ch.pace(1000)
    await ch.skip(() => page.getByRole('button', { name: 'Finish merge' }).first().click())
    await ch.pace(1800)
    await ch.caption('Merge finished with the version you chose', 3000)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
