import { existsSync, rmSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { WORK, pidsOf } from './env'
import { Recorder, closeTerm } from './rec'
import { flat, go, openRepo, toWelcome, visibleErrors } from './gui'
import { intCheck, sha256, walk } from './integrity'
import { startServer } from './server'

// R-02: cancel a 9.4 GB push half-way from the app, then send again. Uses the saved big repository (work/big).
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
const alerts = async (page: Page) => JSON.stringify(await visibleErrors(page)).slice(0, 500)

test('R-02 Cancel mid-push, then retry', async () => {
  const rec = await Recorder.start('R-02', 'cancel-push', { gui: true })
  const page = rec.page!
  const big = join(WORK, 'big')
  if (!existsSync(join(big, '.mediagit'))) throw new Error('needs the 9.4 GB repository work/big')
  const s = await startServer(join(WORK, 'R-02-srv'), ['big'])
  const cli = (...a: string[]) => rec.cli(big, a, { quietFrame: true, env: NOKEY })
  for (const r of ['origin', 'qa12', 'r02']) await cli('remote', 'remove', r)
  await cli('remote', 'add', 'r02', s.url('big'))
  const srvBytes = () => walk(join(s.dir, 'storage'), () => false).concat(walk(join(s.dir, 'repos'), () => false)).reduce((n, f) => { try { return n + statSync(f).size } catch { return n } }, 0)
  await rec.step('Open the big repository, Branches > Sync', async () => { await toWelcome(page); await openRepo(page, big); await go(page, 'branches/sync'); await page.waitForTimeout(2500) })
  const send = page.getByRole('group', { name: 'Share work' }).getByRole('button', { name: /^Send/ })
  const prog = page.getByTestId('transfer-progress')
  const samples: string[] = []
  const t0 = Date.now()
  await rec.step('Send, wait until about 3 GB is on the server, then Cancel', async () => {
    await send.click()
    for (let i = 0; i < 400; i++) { await page.waitForTimeout(250); const txt = flat(await prog.innerText().catch(() => '')).slice(0, 110); if (txt && samples.length < 60) samples.push(`${Date.now() - t0}ms ${txt}`); if (srvBytes() > 3e9) break; if (i > 40 && !txt && !pidsOf('mediagit.exe').length) break }
    await rec.shot('about to cancel')
    rec.note(`at cancel: server holds ${(srvBytes() / 1e9).toFixed(2)} GB after ${Date.now() - t0} ms | progress samples: ${samples.filter((_, i) => i % 4 === 0).slice(0, 8).join(' || ')}`)
    const t1 = Date.now()
    await prog.getByRole('button', { name: 'Cancel' }).click({ timeout: 5000 }).catch(() => rec.note('no Cancel button in the transfer panel'))
    for (let i = 0; i < 150 && pidsOf('mediagit.exe').length; i++) await page.waitForTimeout(100)
    rec.note(`after Cancel the CLI process ended in ${Date.now() - t1} ms; still running: ${pidsOf('mediagit.exe').length}`)
    await page.waitForTimeout(2500)
  })
  const al = await alerts(page)
  const mid = srvBytes()
  rec.note(`after cancel: ${al} | server holds ${(mid / 1e9).toFixed(2)} GB`)
  rec.check(mid > 1e9 && mid < 9e9, 'the push was really cancelled part-way', `${(mid / 1e9).toFixed(2)} GB on the server`)
  rec.check(pidsOf('mediagit.exe').length === 0, 'no CLI process keeps running after Cancel')
  rec.check(/cancel/i.test(al) && !/os error|panicked|protocol|failed with status/i.test(al), 'the app says the send was cancelled, in plain words', al)
  rec.check(await send.isEnabled(), 'Send is usable again')
  const half = join(WORK, 'R-02-half'); rmSync(half, { recursive: true, force: true })
  const c1 = await rec.cli(WORK, ['clone', s.url('big'), half], { env: NOKEY })
  rec.note(`clone from the server after the cancelled push: exit ${c1.code} ${flat(c1.stdout + c1.stderr).slice(-250)}`)
  rec.check(c1.code !== 0 || !existsSync(half) || walk(half).length === 0, 'the server does not offer a half-pushed version (no branch moved)', flat(c1.stdout + c1.stderr).slice(-120))
  rec.check((await (await fetch(`${s.base}/health`)).ok), 'the server is healthy after the cancel')
  rmSync(half, { recursive: true, force: true })
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  const t2 = Date.now()
  await rec.step('Send again and let it finish', async () => { await send.click(); await page.waitForTimeout(2000); for (let i = 0; i < 1200 && pidsOf('mediagit.exe').length; i++) await page.waitForTimeout(250); await page.waitForTimeout(3000) })
  rec.note(`retry: ${Date.now() - t2} ms | ${await alerts(page)} | server now ${(srvBytes() / 1e9).toFixed(2)} GB (a cancelled attempt must not leave a second copy)`)
  const full = join(WORK, 'R-02-full'); rmSync(full, { recursive: true, force: true })
  const c2 = await rec.cli(WORK, ['clone', s.url('big'), full], { env: NOKEY })
  rec.check(c2.code === 0, 'a clone after the retry works', `${flat(c2.stdout + c2.stderr).slice(-200)} | ${c2.ms} ms, peak ${c2.peakMb} MB`)
  const src = walk(big); let same = src.length > 0 && walk(full).length === src.length
  for (const f of src) { const g = join(full, f.slice(big.length + 1)); same = same && existsSync(g) && statSync(g).size === statSync(f).size }
  const biggest = src.map((f) => ({ f, n: statSync(f).size })).sort((a, b) => b.n - a.n)[1]!
  same = same && (await sha256(biggest.f)) === (await sha256(join(full, biggest.f.slice(big.length + 1))))
  rec.check(same, 'the clone has every file with the right size, and a 2 GB file equal by hash', `${walk(full).length} of ${src.length} files`)
  await intCheck(rec, full)
  rmSync(full, { recursive: true, force: true })
  await cli('remote', 'remove', 'r02')
  s.stop()
  rmSync(join(WORK, 'R-02-srv'), { recursive: true, force: true })
  await rec.finish()
  await closeTerm()
})
