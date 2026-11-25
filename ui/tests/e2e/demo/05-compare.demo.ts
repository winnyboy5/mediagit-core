import { copyFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { CORPUS } from './demo-env'
import { startChapter } from './demo'
import { cli, folder, seedRepo } from './seed'
import { choose, go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 05: compare two versions of a picture side by side, in all four styles.
test('05 Compare two versions side by side', async () => {
  const ch = await startChapter('05', 'Compare two versions side by side')
  const { page } = ch
  const dir = folder('poster-art')
  copyFileSync(join(CORPUS, 'freepik__talk__71826.jpeg'), join(dir, 'hero.jpeg'))
  seedRepo(dir, 'First look')
  copyFileSync(join(CORPUS, 'freepik__talk__72772.jpeg'), join(dir, 'hero.jpeg'))
  cli(dir, ['add', '--all'])
  cli(dir, ['commit', '-m', 'Retouched look'])
  try {
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'history/compare')
    await page.waitForTimeout(1500)
    const setup = ch.caption('Pick any two versions of a file to compare', 6500)
    await choose(page, 'To', /Retouched look/)
    await choose(page, 'From', /First look/)
    await page.getByLabel('Changed files').locator('[title="hero.jpeg"]').click()
    await page.getByTestId('compare-pane').locator('img').first().waitFor({ timeout: 30_000 })
    await setup
    await ch.caption('Which version looked better? See them side by side', 3000)
    await ch.pace(1500)
    await ch.caption('Every changed file is listed, with its size and details', 3500)

    const pane = page.getByTestId('compare-pane')
    const styles: [string, string][] = [
      ['Side by side', 'Side by side: the old and the new, next to each other'],
      ['Onion', 'Onion skin: lay the new over the old and fade between them'],
      ['Swipe', 'Swipe: drag a line across the picture to reveal the other version'],
    ]
    for (const [tab, text] of styles) {
      await pane.getByRole('tab', { name: new RegExp(tab, 'i') }).first().click()
      await ch.pace(600)
      await ch.caption(text, 3200)
      const slider = pane.getByRole('slider').first()
      if (await slider.count()) {
        await slider.focus()
        for (let i = 0; i < 10; i++) { await page.keyboard.press('ArrowRight'); await ch.pace(150) }
        await ch.pace(600)
      }
    }
    await ch.caption('Use the slider to spot every small change', 3000)
  } finally {
    console.log('clip:', await ch.stop())
  }
})
