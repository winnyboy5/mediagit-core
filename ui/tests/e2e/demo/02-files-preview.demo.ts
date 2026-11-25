import { copyFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { CORPUS } from './demo-env'
import { startChapter } from './demo'
import { cli, copyMedia, folder } from './seed'
import { go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 02: browse files, search, and preview video / 3D / PDF / audio / image.
test('02 Browse files and preview media', async () => {
  const ch = await startChapter('02', 'Browse files and preview media')
  const { page } = ch
  try {
  const dir = folder('city-promo')
  copyMedia(dir, '101394-video-720.mp4', '1965_ac_shelby_427_cobra_sc.glb',
    '_Into_the_Oceans_and_the_Air_.ogg', '3D_model_of_Shucaris_ankylosskelos_appendage.png', 'freepik__talk__71826.jpeg')
  copyFileSync(join(CORPUS, 'pdf', '1710.02185v1.pdf'), join(dir, '1710.02185v1.pdf'))
  cli(dir, ['init', '--initial-branch', 'main', dir])
  cli(dir, ['add', '--all'])
  cli(dir, ['commit', '-m', 'Promo assets'])
  const tree = page.getByRole('tree', { name: 'All files' })
  const find = page.getByRole('searchbox', { name: 'Find a file' })
  const pick = async (name: string, settle: number) => {
    await find.fill(name)
    await ch.pace(500)
    await tree.getByRole('treeitem', { name: new RegExp(name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click()
    await ch.pace(settle)
  }
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'files/all')
    await tree.getByRole('treeitem').first().waitFor({ timeout: 30_000 })
    await page.getByRole('group', { name: 'Folder view' }).getByText('Flat', { exact: true }).click()
    await ch.pace(800)
    await ch.caption('Every file in your project, in one list', 3200)

    await ch.caption('Type a few letters to find any file instantly', 2500)
    await find.click()
    await ch.type(find, 'cobra')
    await ch.pace(1200)
    await find.fill('')
    await ch.pace(600)

    await ch.caption('Click a file to see it, no need to open another program', 3000)
    await pick('101394-video-720.mp4', 3500)
    await ch.caption('Videos play right here', 3000)

    await pick('1965_ac_shelby_427_cobra_sc.glb', 4000)
    await ch.caption('3D models list their details, like faces and animation', 3500)

    await pick('1710.02185v1.pdf', 3500)
    await ch.caption('Documents show their size and when they last changed', 3000)

    await pick('_Into_the_Oceans_and_the_Air_.ogg', 3000)
    await ch.caption('Audio files play with one click', 3000)

    await pick('3D_model_of_Shucaris_ankylosskelos_appendage.png', 3000)
    await ch.caption('And images appear at full size', 3000)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
