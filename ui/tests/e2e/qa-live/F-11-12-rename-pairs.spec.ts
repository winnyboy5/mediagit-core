import { cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from '@playwright/test'
import { CORPUS, WORK, freeGb } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo, toWelcome } from './gui'
import { corpusHash, hashCompare, intCheck, restoreCheck, sha256 } from './integrity'

interface LsFile { path: string; status: string; staged: boolean; size: number }
const dirSize = (d: string): number => readdirSync(d, { withFileTypes: true }).reduce((n, e) => n + (e.isDirectory() ? dirSize(join(d, e.name)) : statSync(join(d, e.name)).size), 0)
const mb = (n: number) => `${(n / 1048576).toFixed(1)} MB`
const out = (r: { stdout: string; stderr: string }) => `${r.stdout}\n${r.stderr}`.trim()

test('F-11 Rename a 1.7 GB file and move a folder', async () => {
  const rec = await Recorder.start('F-11', 'rename-move', { gui: true })
  const page = rec.page!
  const repo = join(WORK, 'big')
  if (!existsSync(join(repo, '.mediagit'))) throw new Error('F-11 needs work\\big from the pilot (F-06)')
  await rec.cli(repo, ['reset', '--hard', 'HEAD'], { quietFrame: true })
  const ls = async () => (JSON.parse((await rec.cli(repo, ['ls-files', '--machine'], { quietFrame: true })).stdout) as { data: { files: LsFile[] } }).data.files
  const before = await ls()
  rec.note(`work\\big holds ${before.length} files, ${mb(before.reduce((n, f) => n + f.size, 0))}; free disk ${freeGb()} GB`)
  const want: Record<string, string> = {}
  for (const f of before) want[f.path] = await sha256(join(repo, f.path))
  const big = before.reduce((a, b) => (b.size > a.size ? b : a))
  const folder = dirname(before.find((f) => f.path !== big.path && dirname(f.path) !== dirname(big.path))?.path ?? before[0]!.path)
  const store0 = dirSize(join(repo, '.mediagit'))
  const newBig = join(dirname(big.path), `renamed_${big.path.split('/').pop()}`).replace(/\\/g, '/')
  rec.note(`biggest file: ${big.path} (${mb(big.size)}); folder to move: ${folder}; store ${mb(store0)}`)

  await rec.step('Open the 9.4 GB repository in the app', async () => {
    await toWelcome(page, true)
    await openRepo(page, repo)
  })
  await rec.step(`Outside the app: rename ${big.path.split('/').pop()}, move the folder "${folder}" to archive/${folder}`, async () => {
    renameSync(join(repo, big.path), join(repo, newBig))
    mkdirSync(join(repo, 'archive'), { recursive: true })
    renameSync(join(repo, folder), join(repo, 'archive', folder))
    await page.waitForTimeout(3500)
    await go(page, 'files/changes')
    await page.waitForTimeout(1500)
  })
  const st = await rec.cli(repo, ['status', '--porcelain'])
  rec.note(`CLI status after rename and move: ${st.stdout.trim().split('\n').join(' | ').slice(0, 500)}`)
  const rows = (await page.getByRole('tree', { name: 'Changes' }).innerText().catch(() => 'no tree')).replace(/\n+/g, ' / ')
  rec.note(`app Changes after rename and move: ${rows.slice(0, 500)}`)
  const cliRename = /^R/m.test(st.stdout)
  const appRename = /renamed|moved/i.test(rows.replace(/\S*\.\w+/g, ''))
  rec.check(cliRename === appRename, 'CLI and app agree on how a rename is shown (rename, or delete plus new)', `CLI rename ${cliRename}, app rename ${appRename}`)
  rec.note(`a rename is shown as a rename: CLI ${cliRename}, app ${appRename}`)
  rec.note(`summary: ${await page.getByTestId('changes-summary').innerText().catch(() => '')}; dock: ${await page.getByTestId('include-count').innerText().catch(() => '')}`)

  let r = await rec.cli(repo, ['add', '--all'])
  rec.check(r.code === 0, `add --all after rename and move (${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB)`, out(r).slice(-300))
  rec.note(`PERF add --all of renamed 9.4 GB: ${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB`)
  const st2 = await rec.cli(repo, ['status', '--porcelain'])
  rec.note(`status after add: ${st2.stdout.trim().split('\n').join(' | ').slice(0, 400)}`)
  rec.check(!/^( D|\?\?)/m.test(st2.stdout), 'add --all stages both sides of every rename', st2.stdout.slice(0, 300))
  r = await rec.cli(repo, ['commit', '-m', 'Rename the hero plate, archive a folder'])
  rec.check(r.code === 0, 'commit works', out(r).slice(-200))
  const store1 = dirSize(join(repo, '.mediagit'))
  rec.note(`store after the rename commit: ${mb(store1)} (grew ${mb(store1 - store0)})`)
  rec.check(store1 - store0 < 20 * 1048576, 'a rename stores no new file data (store grows under 20 MB)', mb(store1 - store0))
  r = await rec.cli(repo, ['show', 'HEAD'])
  rec.note(`show HEAD: ${out(r).replace(/\n+/g, ' / ').slice(0, 500)}`)

  await rec.step('History: the newest version in the app', async () => {
    await go(page, 'history')
    await page.waitForTimeout(2500)
  })
  rec.note(`History screen: ${(await page.locator('#app').innerText()).replace(/\n+/g, ' / ').slice(0, 600)}`)

  const moved: Record<string, string> = {}
  for (const [p, h] of Object.entries(want)) moved[p === big.path ? newBig : p.startsWith(`${folder}/`) ? `archive/${p}` : p] = h
  await restoreCheck(rec, repo, moved)
  rec.check(!existsSync(join(repo, big.path)), 'the old name does not come back on restore')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('F-12 Edited pairs: v1 to v4 of the same file', async () => {
  const rec = await Recorder.start('F-12', 'edited-pairs')
  const repo = workDir('F-12')
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  await rec.cli(repo, ['init'])
  const chains: Record<string, string[]> = {
    'art/banner.ai': [1, 2, 3, 4].map((n) => `creative-pairs/ai/banner_v${n}.ai`),
    'art/splash.blend': [1, 2, 3].map((n) => `creative-pairs/blend/splash_zstd_v${n}.blend`),
    'ml/pythia70m.safetensors': [1000, 71000, 142000, 143000].map((n) => `ml-pairs/safetensors/pythia70m_step${n}.safetensors`),
    'ml/papers.parquet': ['20261014_3a63217', '20261015_02b656f', '20261016_3ca1b80', '20261017_379dff2'].map((n) => `ml-pairs/parquet/daily_papers_${n}.parquet`),
    'ml/generated.npz': ['v1', 'v2'].map((n) => `ml-pairs/generated/${n}.npz`),
  }
  const versions: { oid: string; want: Record<string, string> }[] = []
  const table: string[] = []
  let raw = 0
  const store = () => dirSize(join(repo, '.mediagit'))
  for (let v = 0; v < 4; v++) {
    const s0 = store()
    let stepRaw = 0
    const changed: string[] = []
    for (const [dst, srcs] of Object.entries(chains)) {
      if (!srcs[v]) continue
      mkdirSync(dirname(join(repo, dst)), { recursive: true })
      cpSync(join(CORPUS, srcs[v]!), join(repo, dst))
      stepRaw += statSync(join(repo, dst)).size
      changed.push(dst)
    }
    const st = await rec.cli(repo, ['status', '--porcelain'])
    if (v > 0) rec.check(changed.every((c) => new RegExp(`^ M ${c}`, 'm').test(st.stdout)), `v${v + 1}: every replaced file shows as edited`, st.stdout.trim().split('\n').join(' | '))
    const a = await rec.cli(repo, ['add', '--all'])
    const c = await rec.cli(repo, ['commit', '-m', `version ${v + 1}`])
    rec.check(a.code === 0 && c.code === 0, `v${v + 1}: add and commit work`, out(a).slice(-200) + out(c).slice(-200))
    const grew = store() - s0
    raw += stepRaw
    table.push(`| v${v + 1} | ${changed.length} | ${mb(stepRaw)} | ${mb(grew)} | ${((grew / stepRaw) * 100).toFixed(0)}% | ${a.ms} ms | ${a.peakMb.toFixed(0)} MB |`)
    const want: Record<string, string> = {}
    for (const [dst, srcs] of Object.entries(chains)) want[dst] = await corpusHash(srcs[Math.min(v, srcs.length - 1)]!)
    const head = JSON.parse((await rec.cli(repo, ['log', '--machine', '-n', '1'], { quietFrame: true })).stdout) as { data: { commits: { oid: string }[] } }
    versions.push({ oid: head.data.commits[0]!.oid, want })
  }
  rec.note(`PAIRS TABLE\n| Version | Files | Raw size | Store growth | Growth / raw | add time | add peak |\n|---|---|---|---|---|---|---|\n${table.join('\n')}`)
  const total = store()
  rec.note(`store total ${mb(total)} for ${mb(raw)} of raw data (${((total / raw) * 100).toFixed(0)}%); free disk ${freeGb()} GB`)
  rec.check(total < raw, 'the store is smaller than the sum of all versions', `${mb(total)} vs ${mb(raw)}`)
  const stats = await rec.cli(repo, ['stats', '--storage', '--compression'])
  rec.note(`stats: ${out(stats).replace(/\n+/g, ' / ').slice(0, 600)}`)

  // --- every version must come back byte for byte
  for (const [i, ver] of [...versions.entries()].reverse()) {
    const r = await rec.cli(repo, ['reset', '--hard', ver.oid])
    rec.check(r.code === 0, `checkout of v${i + 1} works (${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB)`, out(r).slice(-300))
    await hashCompare(rec, repo, ver.want, `v${i + 1} equals its originals`)
  }
  const tip = versions[versions.length - 1]!
  await rec.cli(repo, ['reset', '--hard', tip.oid])
  await restoreCheck(rec, repo, tip.want)

  // --- gc must not break the delta chains
  const g = await rec.cli(repo, ['gc'])
  rec.note(`gc: exit ${g.code}, ${g.ms} ms, store ${mb(store())}: ${out(g).replace(/\n+/g, ' / ').slice(-300)}`)
  for (const [i, ver] of versions.entries()) {
    const r = await rec.cli(repo, ['reset', '--hard', ver.oid], { quietFrame: true })
    rec.check(r.code === 0, `after gc: checkout of v${i + 1} works`, out(r).slice(-300))
    await hashCompare(rec, repo, ver.want, `after gc: v${i + 1} equals its originals`)
  }
  await intCheck(rec, repo, tip.want)
  await rec.finish()
  await closeTerm()
})
