import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import './demo-env'
import { startChapter } from './demo'
import { cli, copyMedia, folder } from './seed'
import { go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 04: version list, search, branch graph, version details with preview.
test('04 History graph and version details', async () => {
  const ch = await startChapter('04', 'History graph and version details')
  const { page } = ch
  try {
  const dir = folder('harbour-film')
  const note = (s: string) => writeFileSync(join(dir, 'notes.txt'), s)
  const save = (m: string) => { cli(dir, ['add', '--all']); cli(dir, ['commit', '-m', m]) }
  cli(dir, ['init', '--initial-branch', 'main', dir])
  note('harbour film\n')
  save('Start the harbour film')
  copyMedia(dir, '3D_model_of_Shucaris_ankylosskelos_appendage.png')
  save('Add the opening render')
  cli(dir, ['branch', 'create', 'feature/titles'])
  cli(dir, ['branch', 'create', 'feature/sound'])
  cli(dir, ['branch', 'switch', 'feature/titles'])
  note('harbour film\ntitle card: Harbour at Dawn\n')
  save('Draft the title card')
  cli(dir, ['branch', 'switch', 'main'])
  note('harbour film\ngrade: warm\n')
  save('First colour grade')
  cli(dir, ['branch', 'switch', 'feature/sound'])
  copyMedia(dir, 'freepik__talk__71826.jpeg')
  save('Add reference photo for the mix')
  cli(dir, ['branch', 'switch', 'main'])
  note('harbour film\ngrade: warm, final\n')
  save('Final colour grade')
  const list = page.getByRole('listbox', { name: 'Versions, newest first' })
  const detail = page.getByTestId('commit-detail')
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'history')
    await list.getByRole('option').first().waitFor({ timeout: 30_000 })
    await ch.pace(800)
    await ch.caption('History lists every version you have saved, newest first', 3500)

    const which = page.getByRole('group', { name: 'Which versions' }).or(page.getByRole('radiogroup', { name: 'Which versions' })).first()
    await ch.caption('Switch between this line of work and all of them', 3000)
    await which.getByText('This branch only', { exact: true }).click()
    await ch.pace(1500)
    await which.getByText('All branches', { exact: true }).click()
    await ch.pace(1500)

    await ch.caption('Search by what you wrote to find an older version', 3000)
    const search = page.getByPlaceholder('Search versions')
    await ch.type(search, 'grade')
    await ch.pace(1800)
    await search.fill('')
    await ch.pace(800)

    await ch.caption('The graph shows how side branches split off and rejoin', 3500)
    await ch.pace(1000)

    await list.getByRole('option').last().click()
    await ch.pace(1200)
    await list.getByRole('option').nth(-2).click().catch(() => {})
    await ch.pace(1500)
    await ch.caption('Open any version to see who saved it, when, and what changed', 3500)
    await page.getByPlaceholder('Search versions').fill('opening render')
    await ch.pace(900)
    await list.getByRole('option').first().click()
    await ch.pace(1500)
    await detail.getByLabel('Files this version changed').locator('[title]').first().click().catch(() => {})
    await ch.pace(2000)
    await ch.caption('Click a file to preview exactly how it looked back then', 3500)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
