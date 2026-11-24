import { rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo, parseBytes, sampleProgress, type ProgressSample } from './gui'
import { copyCorpus, intCheck, restoreCheck } from './integrity'
import { OUT, freeGb } from './env'

// F-06: staging metrics on large real video. Files done/total, bytes, rate and ETA must be
// real and must move. Samples the header pill every 500 ms while "Include all" runs.
const BIG: Record<string, string> = {
  'video-implementation/F004C003_190925_MN99.mxf': 'plates/F004C003_190925_MN99.mxf',
  'video-implementation/DW0001C001_251020_111956_c1I7H.mxf': 'plates/DW0001C001_251020_111956_c1I7H.mxf',
  'video-spike/timing_unique_5g.mov': 'edit/timing_unique_5g.mov',
}

test('F-06 staging metrics on large files', async () => {
  const rec = await Recorder.start('F-06', 'staging-metrics', { gui: true })
  const page = rec.page!
  const repo = workDir('big')
  rmSync(repo, { recursive: true, force: true })
  workDir('big')
  rec.note(`free disk before: ${freeGb()} GB`)
  await rec.cli(repo, ['init'])
  const t0c = Date.now()
  const want = await copyCorpus(repo, BIG)
  const total = Object.keys(want).reduce((n, f) => n + statSync(join(repo, f)).size, 0)
  rec.note(`copied and hashed ${Object.keys(want).length} files, ${(total / 1e9).toFixed(2)} GB, in ${Math.round((Date.now() - t0c) / 1000)} s`)

  await rec.step('Open the repository and go to Files > Changes', async () => {
    await openRepo(page, repo)
    await go(page, 'files/changes')
    await page.getByRole('tree', { name: 'Changes' }).getByRole('treeitem').first().waitFor({ timeout: 30_000 })
  })
  const count = () => page.getByTestId('include-count').innerText()
  rec.check(/0 of 3/.test(await count()), 'three changes listed, none included', await count())

  const samples: ProgressSample[] = []
  let addMs = 0
  await rec.liveCapture(async () => {
    await rec.step('Click "Include all" and watch the progress', async () => {
      const t0 = Date.now()
      await page.getByRole('button', { name: 'Include all', exact: true }).click()
      let lastShot = 0
      // Done when the count reads 3 of 3 and the pill no longer shows a running op.
      for (;;) {
        const s = await sampleProgress(page, t0)
        samples.push(s)
        if (s.t - lastShot > 15_000) { lastShot = s.t; await rec.shot(`progress at ${Math.round(s.t / 1000)} s: ${s.pill}`) }
        const c = await count().catch(() => '')
        if (/3 of 3/.test(c) && !s.bytes && !s.files) break
        if (s.t > 40 * 60_000) throw new Error('add did not finish in 40 minutes')
        await page.waitForTimeout(500)
      }
      addMs = Date.now() - t0
    })
  }, 'MediaGit')
  writeFileSync(join(OUT, 'F-06-staging-metrics.samples.json'), JSON.stringify(samples))
  rec.note(`add took ${(addMs / 1000).toFixed(1)} s = ${(total / 1e6 / (addMs / 1000)).toFixed(0)} MB/s, ${samples.length} samples`)

  // --- what the numbers did
  const live = samples.filter((s) => s.bytes || s.files)
  const bytes = live.map((s) => parseBytes(s.bytes)).filter((n) => !Number.isNaN(n))
  const distinct = <T,>(a: T[]) => new Set(a).size
  rec.note(`first 6 samples: ${JSON.stringify(live.slice(0, 6))}`)
  rec.note(`middle sample: ${JSON.stringify(live[Math.floor(live.length / 2)])}`)
  rec.note(`last 3 samples: ${JSON.stringify(live.slice(-3))}`)
  rec.check(live.length > 5, 'the pill showed progress numbers while staging', `${live.length} samples with numbers of ${samples.length}`)
  rec.check(distinct(bytes) > 5, 'bytes done moved (more than 5 different values)', `${distinct(bytes)} distinct values`)
  rec.check(bytes.every((b, i) => i === 0 || b >= bytes[i - 1]! * 0.999), 'bytes done never went down', bytes.map((b) => (b / 1e9).toFixed(2)).filter((_, i) => i % Math.ceil(bytes.length / 20) === 0).join(' '))
  const totals = distinct(live.map((s) => s.bytes.split(' of ')[1] ?? '').filter(Boolean))
  rec.check(totals >= 1, 'a bytes total is shown', live.find((s) => s.bytes)?.bytes)
  const shownTotal = parseBytes(live.find((s) => s.bytes.includes(' of '))?.bytes.split(' of ')[1] ?? '')
  rec.check(Math.abs(shownTotal - total) / total < 0.08, 'the bytes total equals the real size of the three files', `shown ${(shownTotal / 1e9).toFixed(2)} GB, real ${(total / 1e9).toFixed(2)} GB (GiB reading: ${(total / 1024 ** 3).toFixed(2)})`)
  rec.check(live.some((s) => /of 3 files/.test(s.files)), 'files shows "N of 3 files"', [...new Set(live.map((s) => s.files))].join(' | '))
  rec.check(distinct(live.map((s) => s.files)) > 1, 'files done moved', [...new Set(live.map((s) => s.files))].join(' | '))
  rec.check(distinct(live.map((s) => s.rate).filter(Boolean)) > 3, 'rate is shown and changes', [...new Set(live.map((s) => s.rate))].slice(0, 12).join(' | '))
  rec.check(distinct(live.map((s) => s.eta).filter(Boolean)) > 3, 'ETA is shown and changes', [...new Set(live.map((s) => s.eta))].slice(0, 12).join(' | '))
  const first = live[0]
  rec.note(`first numbers appeared ${first?.t} ms after the click; rate then "${first?.rate}", eta "${first?.eta}"`)
  rec.check(!first || !first.rate || first.t > 400, 'no rate is claimed before anything was measured', JSON.stringify(first))
  // longest stretch with no visible change at all
  let stall = 0
  let since = live[0]?.t ?? 0
  for (let i = 1; i < live.length; i++) {
    if (live[i]!.bytes !== live[i - 1]!.bytes || live[i]!.bar !== live[i - 1]!.bar) since = live[i]!.t
    stall = Math.max(stall, live[i]!.t - since)
  }
  rec.note(`longest stretch with frozen numbers: ${(stall / 1000).toFixed(1)} s`)
  rec.check(stall < 15_000, 'progress never froze for 15 s or more', `${(stall / 1000).toFixed(1)} s`)

  // --- disk and CLI agree
  const por = await rec.cli(repo, ['status', '--porcelain'])
  rec.check(por.stdout.trim().split('\n').filter((l) => /^A/.test(l)).length === 3, 'CLI agrees: three files staged', por.stdout.trim())

  await rec.step('Save a version', async () => {
    await page.getByLabel('Message').fill('F-06: three large plates')
    await page.getByRole('button', { name: 'Save version', exact: true }).click()
    await page.getByRole('heading', { name: 'No changes' }).waitFor({ timeout: 10 * 60_000 })
  })
  const log = await rec.cli(repo, ['log', '--oneline', '-n', '1'])
  rec.check(log.stdout.includes('F-06: three large plates'), 'CLI agrees: the version exists', log.stdout.trim())
  const stats = await rec.cli(repo, ['stats', '--storage'])
  rec.note(`stats: ${stats.stdout.replace(/\n+/g, ' / ').slice(0, 500)}`)

  await intCheck(rec, repo, want)
  await restoreCheck(rec, repo, want)
  rec.note(`free disk after: ${freeGb()} GB`)
  await rec.finish()
  await closeTerm()
})
