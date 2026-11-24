import { test } from '@playwright/test'
import { Recorder, closeTerm } from './rec'
import { WORK } from './env'

// Preflight 0.3: the harness itself. App frame, CLI frame, MP4, contact sheet, live capture.
test('000 smoke: harness records the app and the CLI', async () => {
  const rec = await Recorder.start('000', 'smoke', { gui: true })
  const page = rec.page!
  rec.note(`url ${page.url()} title ${await page.title()}`)
  await rec.step('app is showing', async () => {
    await page.waitForSelector('#app *', { timeout: 20_000 })
  })
  const body = await page.evaluate(() => document.body.innerText.slice(0, 400))
  rec.note(`body text: ${body.replace(/\n+/g, ' / ')}`)
  rec.check((await page.locator('#app *').count()) > 0 && !body.includes('ERR_CONNECTION'), 'the desktop exe renders the real UI (not a blank or error page)')
  const v = await rec.cli(WORK, ['--version'])
  rec.check(v.stdout.includes('0.8.0-rc.1'), 'CLI reports 0.8.0-rc.1', v.stdout.trim())
  await rec.liveCapture(async () => {
    await page.mouse.move(300, 300)
    await new Promise((r) => setTimeout(r, 4000))
  }, await page.title())
  await rec.finish()
  await closeTerm()
})
