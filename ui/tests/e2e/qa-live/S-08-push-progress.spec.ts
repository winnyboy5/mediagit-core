import { existsSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { Recorder, closeTerm } from './rec'
import { go, openRepo, sampleProgress, visibleErrors, type ProgressSample } from './gui'
import { corpusHash, intCheck } from './integrity'
import { startServer } from './server'
import { OUT, WORK, freeGb } from './env'

// S-08 (pilot part): add a remote in the app, Send 9.4 GB with progress, then a fresh CLI
// clone must equal the originals. Needs work\big from F-06 (one version, three large files).
const FILES: Record<string, string> = {
  'plates/F004C003_190925_MN99.mxf': 'video-implementation/F004C003_190925_MN99.mxf',
  'plates/DW0001C001_251020_111956_c1I7H.mxf': 'video-implementation/DW0001C001_251020_111956_c1I7H.mxf',
  'edit/timing_unique_5g.mov': 'video-spike/timing_unique_5g.mov',
}

test('S-08 push with progress, then clone and compare', async () => {
  const rec = await Recorder.start('S-08', 'push-progress', { gui: true })
  const page = rec.page!
  const repo = join(WORK, 'big')
  if (!existsSync(join(repo, '.mediagit'))) throw new Error('run F-06 first: work\\big is missing')
  const want: Record<string, string> = {}
  for (const [rel, src] of Object.entries(FILES)) want[rel] = await corpusHash(src)
  const total = Object.keys(want).reduce((n, f) => n + statSync(join(repo, f)).size, 0)

  const srvDir = join(WORK, 'srv-s08')
  rmSync(srvDir, { recursive: true, force: true })
  const clone = join(WORK, 'big-clone')
  rmSync(clone, { recursive: true, force: true })
  // A remote left by an earlier run of this case would point at a dead port.
  await rec.cli(repo, ['remote', 'remove', 'origin'], { quietFrame: true })
  const srv = await startServer(srvDir, ['big'])
  rec.note(`server ${srv.base} pid ${srv.pid}, free disk ${freeGb()} GB`)

  try {
    await rec.step('Open the repository, Branches > Sync (no remote yet)', async () => {
      await openRepo(page, repo)
      await go(page, 'branches/sync')
      await page.getByRole('button', { name: 'Add remote' }).first().waitFor()
    })
    rec.note(`Sync page with no remote: ${(await page.locator('#app').innerText()).replace(/\n+/g, ' / ').slice(0, 400)}`)
    rec.check(await page.getByRole('button', { name: /Send/ }).first().isDisabled(), 'header Send is disabled while there is no remote')

    const dlg = page.getByRole('dialog')
    await rec.step('Add remote: a bad address is refused in the form', async () => {
      await page.getByRole('button', { name: 'Add remote' }).first().click()
      await dlg.waitFor()
      await dlg.getByLabel('Name').fill('origin')
      await dlg.getByLabel('Address').fill('not a url')
    })
    rec.check(await dlg.getByRole('button', { name: 'Add remote' }).isDisabled(), 'Add remote is disabled for "not a url"')
    await rec.step('Add remote: the real address', async () => {
      await dlg.getByLabel('Address').fill(srv.url('big'))
      await dlg.getByRole('button', { name: 'Add remote' }).click()
      await dlg.waitFor({ state: 'hidden' })
      await page.getByTestId('sync-state').waitFor()
    })
    const rl = await rec.cli(repo, ['remote', 'list', '--verbose'])
    rec.check(rl.stdout.includes(srv.url('big')), 'CLI agrees: origin points at the server', rl.stdout.trim())

    // --- Send, sampling the header pill and the Sync page panel
    const samples: (ProgressSample & { panel: string })[] = []
    let pushMs = 0
    await rec.liveCapture(async () => {
      await rec.step('Click Send and watch the progress', async () => {
        const t0 = Date.now()
        await page.getByRole('group', { name: 'Share work' }).getByRole('button', { name: /Send/ }).click()
        let lastShot = 0
        let seen = false
        for (;;) {
          const s = await sampleProgress(page, t0)
          const panel = await page.evaluate(() => (document.querySelector('[data-testid="transfer-progress"]') as HTMLElement | null)?.innerText.replace(/\s+/g, ' ').trim() ?? '')
          samples.push({ ...s, panel })
          if (panel || s.pill.includes('%')) seen = true
          if (s.t - lastShot > 15_000) { lastShot = s.t; await rec.shot(`push at ${Math.round(s.t / 1000)} s: ${panel || s.pill}`) }
          const idle = !panel && (!s.pill || /done|failed|cancelled/.test(s.pill))
          if ((seen && idle && s.t > 3000) || (!seen && s.t > 30_000)) break
          if (s.t > 60 * 60_000) throw new Error('push did not finish in 60 minutes')
          await page.waitForTimeout(500)
        }
        pushMs = Date.now() - t0
      })
    })
    writeFileSync(join(OUT, 'S-08-push-progress.samples.json'), JSON.stringify(samples))
    const live = samples.filter((s) => s.panel)
    rec.note(`push took ${(pushMs / 1000).toFixed(1)} s = ${(total / 1e6 / (pushMs / 1000)).toFixed(0)} MB/s; ${live.length} samples with a panel`)
    rec.note(`first panel samples: ${JSON.stringify(live.slice(0, 4).map((s) => `${s.t}: ${s.panel}`))}`)
    rec.note(`middle: ${JSON.stringify(live[Math.floor(live.length / 2)]?.panel)} | pill: ${JSON.stringify(live[Math.floor(live.length / 2)]?.pill)}`)
    rec.note(`last: ${JSON.stringify(live.slice(-2).map((s) => s.panel))}; final pill: ${JSON.stringify(samples.at(-1)?.pill)}`)
    rec.note(`messages on screen after the push: ${JSON.stringify(await visibleErrors(page))}`)
    rec.check(live.length > 5, 'the Sync page showed a transfer panel while sending', `${live.length} samples`)
    rec.check(new Set(live.map((s) => s.panel)).size > 5, 'the transfer numbers moved', `${new Set(live.map((s) => s.panel)).size} distinct readings`)
    rec.check(live.some((s) => /Cancel/.test(s.panel)), 'a Cancel button is offered during the push')
    rec.check(live.some((s) => /\/s/.test(s.panel)), 'a rate is shown during the push')
    rec.check(/done/.test(samples.at(-1)?.pill ?? '') || !(await visibleErrors(page)).join(' ').match(/fail|error/i), 'the push ended without an error message', samples.at(-1)?.pill)

    await page.waitForTimeout(1500)
    await rec.shot('Sync page after the push')
    rec.note(`Sync state after push: ${(await page.getByTestId('sync-state').innerText()).replace(/\n+/g, ' / ')}`)

    // --- the server really has it: clone with the CLI and compare bytes
    const cl = await rec.cli(WORK, ['clone', srv.url('big'), clone])
    rec.check(cl.code === 0, `CLI clone from the server works (${(cl.ms / 1000).toFixed(1)} s = ${(total / 1e6 / (cl.ms / 1000)).toFixed(0)} MB/s, peak ${cl.peakMb.toFixed(0)} MB)`, cl.code === 0 ? undefined : (cl.stdout + cl.stderr).slice(-500))
    await intCheck(rec, clone, want)
    await intCheck(rec, repo, want)
    const st = await rec.cli(repo, ['status', '--branch'])
    rec.note(`source status after push: ${st.stdout.replace(/\n+/g, ' / ')}`)
    const srvLog = existsSync(srv.log) ? statSync(srv.log).size : 0
    rec.note(`server log ${srvLog} bytes; free disk after ${freeGb()} GB`)
  } finally {
    srv.stop()
  }
  await rec.finish()
  await closeTerm()
})
