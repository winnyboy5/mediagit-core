import { cpSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo, refocus, toWelcome, visibleErrors } from './gui'

// H-01..H-03 share one repository with 45+ versions, 3 authors, 5 branches and merges.
const repo = workDir('H-01')
interface Commit { oid: string; parents: string[]; author: { name: string; email: string }; time: number; summary: string }
const AUTHORS = ['Ana Ruiz <ana@studio.example>', 'Ben Okafor <ben@studio.example>', 'Chloé Dubois <chloe@studio.example>']
const MIXED: Record<string, string> = {
  '3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg': 'mixed/plan.svg',
  'freepik__talk__71826.jpeg': 'mixed/photo.jpeg',
  'freepik__talk__72772.jpeg': 'mixed/photo2.jpeg',
  'Workstation_cube_lid_off.webp': 'mixed/render.webp',
  '3D_Model_-_Sperry_F3619-00.gif': 'mixed/turntable.gif',
  '101394-video-720.mp4': 'mixed/clip.mp4',
  'video-variants/bbb-5s-h264.mov': 'mixed/bunny.mov',
  '_Into_the_Oceans_and_the_Air_.ogg': 'mixed/oceans.ogg',
  'pdf/1710.02185v1.pdf': 'mixed/paper.pdf',
  '12690118_5053480.ai': 'mixed/poster.ai',
  '12690118_5053481.eps': 'mixed/poster.eps',
  '1965_ac_shelby_427_cobra_sc.glb': 'mixed/cobra.glb',
  '1900s_telephone.stl': 'mixed/telephone.stl',
  'creative-pairs/blend/splash_zstd_v1.blend': 'mixed/splash.blend',
  'creative-pairs/ai/banner_v1.ai': 'mixed/banner_a.ai',
  'creative-pairs/ai/banner_v2.ai': 'mixed/banner_b.ai',
  'creative-pairs/ai/banner_v3.ai': 'mixed/banner_c.ai',
  'creative-pairs/ai/banner_v4.ai': 'mixed/banner_d.ai',
  'test-label-org.ai': 'mixed/label_org.ai',
  'test-label-alt.ai': 'mixed/label_alt.ai',
}

const list = (page: Page) => page.getByRole('listbox', { name: 'Versions, newest first' })
const appText = async (page: Page) => (await page.locator('#app').innerText()).replace(/\n+/g, ' / ')
const same = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i])
const sameSet = (a: string[], b: string[]) => a.length === b.length && [...a].sort().join() === [...b].sort().join()

async function log(rec: Recorder, ...args: string[]): Promise<Commit[]> {
  const r = await rec.cli(repo, ['log', '--machine', ...args], { quietFrame: true })
  try {
    return (JSON.parse(r.stdout) as { data: { commits: Commit[] } }).data.commits
  } catch {
    rec.note(`log ${args.join(' ')} gave no JSON: exit ${r.code} ${(r.stdout + r.stderr).slice(0, 200)}`)
    return []
  }
}

/** Load every page of the list, then return what the History store shows. */
async function shown(page: Page): Promise<{ oids: string[]; summaries: string[]; rows: unknown[]; head: string; done: boolean; searching: boolean }> {
  for (let i = 0; i < 30; i++) {
    const done = await page.evaluate(() => {
      const app = (document.querySelector('#app') as unknown as { __vue_app__: { config: { globalProperties: { $pinia: { _s: Map<string, Record<string, unknown>> } } } } }).__vue_app__
      const s = app.config.globalProperties.$pinia._s.get('history')!
      const el = document.querySelector('[role="listbox"][aria-label="Versions, newest first"]') as HTMLElement | null
      if (el) el.scrollTop = el.scrollHeight
      return (s.done as boolean) && !(s.loading as boolean) && !(s.loadingMore as boolean)
    })
    if (done) break
    await page.waitForTimeout(250)
  }
  return page.evaluate(() => {
    const app = (document.querySelector('#app') as unknown as { __vue_app__: { config: { globalProperties: { $pinia: { _s: Map<string, Record<string, unknown>> } } } } }).__vue_app__
    const s = app.config.globalProperties.$pinia._s.get('history')!
    const v = s.visible as { oid: string; summary: string }[]
    const el = document.querySelector('[role="listbox"][aria-label="Versions, newest first"]') as HTMLElement | null
    if (el) el.scrollTop = 0
    return { oids: v.map((c) => c.oid), summaries: v.map((c) => c.summary), rows: JSON.parse(JSON.stringify(s.rows ?? [])) as unknown[], head: String(s.head ?? ''), done: !!s.done, searching: !!s.searching }
  })
}

async function build(rec: Recorder) {
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  const cli = (...a: string[]) => rec.cli(repo, a, { quietFrame: true })
  await cli('init')
  let n = 0
  const save = async (file: string, body: string | { from: string }, message: string, day: string) => {
    mkdirSync(dirname(join(repo, file)), { recursive: true })
    if (typeof body === 'string') writeFileSync(join(repo, file), body)
    else cpSync(join(CORPUS, body.from), join(repo, file))
    await cli('add', '--all')
    const r = await cli('commit', '-m', message, '--author', AUTHORS[n++ % 3]!, '--date', `${day}T10:${String(n % 60).padStart(2, '0')}:00Z`)
    if (r.code !== 0) rec.note(`setup commit failed: ${message}: ${(r.stdout + r.stderr).slice(-200)}`)
  }
  const day = (i: number) => `2026-${String(1 + Math.floor(i / 8)).padStart(2, '0')}-${String(2 + (i % 8) * 3).padStart(2, '0')}`
  for (let i = 0; i < 12; i++) {
    if (i % 3 === 0) await save('art/banner.ai', { from: `creative-pairs/ai/banner_v${1 + ((i / 3) % 4)}.ai` }, `grade pass ${i}: banner colours`, day(i))
    else if (i % 3 === 1) await save(`video/shots/s${String(i).padStart(3, '0')}.txt`, `shot ${i}\n`, `layout ${i}: shot list`, day(i))
    else await save('notes/review.txt', `review ${i}\n`, `review notes ${i}`, day(i))
  }
  await cli('branch', 'create', 'feature/titles')
  await cli('branch', 'create', 'feature/sound')
  await cli('branch', 'switch', 'feature/titles')
  for (let i = 12; i < 15; i++) await save(`titles/t${i}.txt`, `title ${i}\n`, `titles ${i}`, day(i))
  await cli('branch', 'switch', 'main')
  for (let i = 15; i < 20; i++) await save(`video/shots/s${String(i).padStart(3, '0')}.txt`, `shot ${i}\n`, `layout ${i}: shot list`, day(i))
  let m = await cli('merge', 'feature/titles', '-m', 'Merge titles into main')
  rec.note(`setup merge 1: exit ${m.code} ${(m.stdout + m.stderr).replace(/\n+/g, ' / ').slice(-160)}`)
  await cli('branch', 'switch', 'feature/sound')
  for (let i = 20; i < 24; i++) await save(`sound/mix${i}.txt`, `mix ${i}\n`, `sound mix ${i}`, day(i))
  await cli('branch', 'switch', 'main')
  for (let i = 24; i < 28; i++) await save('notes/review.txt', `review ${i}\n`, `review notes ${i}`, day(i))
  m = await cli('merge', 'feature/sound', '-m', 'Merge sound into main')
  rec.note(`setup merge 2: exit ${m.code} ${(m.stdout + m.stderr).replace(/\n+/g, ' / ').slice(-160)}`)
  await cli('branch', 'create', 'hotfix/credits')
  await cli('branch', 'create', 'feature/grade-b')
  await cli('branch', 'switch', 'hotfix/credits')
  await save('titles/credits.txt', 'credits fix\n', 'hotfix: credits typo', day(28))
  await cli('branch', 'switch', 'feature/grade-b')
  for (let i = 29; i < 32; i++) await save('art/banner.ai', { from: `creative-pairs/ai/banner_v${1 + (i % 4)}.ai` }, `grade pass ${i}: alternate look`, day(i))
  await cli('branch', 'switch', 'main')
  for (let i = 32; i < 36; i++) await save(`video/shots/s${String(i).padStart(3, '0')}.txt`, `shot ${i}\n`, `layout ${i}: shot list`, day(i))
  m = await cli('merge', 'hotfix/credits', '-m', 'Merge hotfix credits')
  // criss-cross: main into grade-b, then one more commit on each side
  await cli('branch', 'switch', 'feature/grade-b')
  m = await cli('merge', 'main', '-m', 'Merge main into grade-b')
  rec.note(`setup merge main into grade-b: exit ${m.code} ${(m.stdout + m.stderr).replace(/\n+/g, ' / ').slice(-160)}`)
  await save('art/look.txt', 'look b\n', 'grade pass 36: look notes', day(36))
  await cli('branch', 'switch', 'main')
  for (const [from, to] of Object.entries(MIXED)) {
    mkdirSync(dirname(join(repo, to)), { recursive: true })
    cpSync(join(CORPUS, from), join(repo, to))
  }
  await cli('add', '--all')
  await cli('commit', '-m', 'Twenty mixed files', '--author', AUTHORS[0]!, '--date', '2026-06-20T09:00:00Z')
  await save('art/banner.ai', { from: 'creative-pairs/ai/banner_v2.ai' }, 'grade pass 40: final banner', '2026-06-21')
  await cli('branch', 'create', 'idea/untouched')
}

test('H-01 Versions list: paging, search and every filter', async () => {
  const rec = await Recorder.start('H-01', 'history-list', { gui: true })
  const page = rec.page!
  await build(rec)
  const mine = await log(rec)
  const all = await log(rec, '--all')
  rec.note(`repository: ${mine.length} versions on main, ${all.length} on all branches`)
  rec.check(mine.length >= 38, 'setup: main holds 38 or more versions', String(mine.length))
  const br = await rec.cli(repo, ['branch', 'list'])
  rec.note(`branches: ${br.stdout.trim().split('\n').map((l) => l.trim()).join(', ')}`)

  let t0 = Date.now()
  await rec.step('Open the repository, History', async () => {
    await toWelcome(page, true)
    await openRepo(page, repo)
    t0 = Date.now()
    await go(page, 'history')
    await list(page).getByRole('option').first().waitFor({ timeout: 30_000 })
  })
  rec.note(`PERF History first rows: ${Date.now() - t0} ms; DOM rows ${await list(page).getByRole('option').count()}`)
  const which = page.getByRole('group', { name: 'Which versions' }).or(page.getByRole('radiogroup', { name: 'Which versions' })).first()
  rec.note(`"Which versions" control: ${(await which.innerText().catch(() => 'not found')).replace(/\n+/g, ' / ')}`)
  let s = await shown(page)
  rec.note(`default list: ${s.oids.length} versions (main has ${mine.length}, all branches ${all.length})`)
  rec.check(sameSet(s.oids, all.map((c) => c.oid)) || same(s.oids, mine.map((c) => c.oid)), 'the default list equals log (this branch, or all branches)', `${s.oids.length}`)

  await rec.step('Which versions: This branch only', async () => {
    await which.getByText('This branch only', { exact: true }).click()
    await page.waitForTimeout(900)
  })
  s = await shown(page)
  rec.check(same(s.oids, mine.map((c) => c.oid)), '"This branch only" equals `log`, in the same order', `${s.oids.length} vs ${mine.length}; first differing index ${s.oids.findIndex((o, i) => o !== mine[i]?.oid)}`)
  await rec.step('Which versions: All branches', async () => {
    await which.getByText('All branches', { exact: true }).click()
    await page.waitForTimeout(900)
  })
  s = await shown(page)
  rec.check(sameSet(s.oids, all.map((c) => c.oid)), '"All branches" equals `log --all`', `${s.oids.length} vs ${all.length}`)
  rec.check(same(s.oids, all.map((c) => c.oid)), '"All branches" is in the same order as `log --all`', `first differing index ${s.oids.findIndex((o, i) => o !== all[i]?.oid)}`)
  rec.check(new Set(s.oids).size === s.oids.length, 'no version is listed twice')

  // --- quick search
  const search = page.getByPlaceholder('Search versions')
  for (const q of ['grade', 'GRADE', 'Chloé', 'banner', 'zzzqqq']) {
    await rec.step(`Search versions: "${q}"`, async () => {
      await search.fill(q)
      await page.waitForTimeout(1200)
    })
    s = await shown(page)
    const inMsg = all.filter((c) => c.summary.toLowerCase().includes(q.toLowerCase())).length
    const inAny = all.filter((c) => `${c.summary} ${c.author.name} ${c.author.email} ${c.oid}`.toLowerCase().includes(q.toLowerCase())).length
    rec.note(`search "${q}": app ${s.oids.length}; log message matches ${inMsg}, message or author ${inAny}; graph hidden note ${(await appText(page)).includes('graph is hidden')}`)
    rec.check(s.oids.length === inMsg || s.oids.length === inAny, `search "${q}" matches log`, `app ${s.oids.length}, message ${inMsg}, any ${inAny}`)
  }
  rec.note(`no-match screen: ${(await appText(page)).slice(-200)}`)
  await search.fill('')
  await page.waitForTimeout(900)

  // --- filters
  const filters = page.getByRole('group', { name: 'Filter versions' }).or(page.getByRole('form', { name: 'Filter versions' })).first()
  await rec.step('Open Filters', async () => {
    if (!(await filters.isVisible().catch(() => false))) await page.getByRole('button', { name: /Filters/ }).click()
    await page.waitForTimeout(400)
  })
  rec.note(`filter panel: ${(await filters.innerText().catch(() => 'not found')).replace(/\n+/g, ' / ').slice(0, 300)}`)
  const cases: { what: string; fill: Record<string, string>; args: string[] }[] = [
    { what: 'Saved by "Ana"', fill: { 'Saved by': 'Ana' }, args: ['--all', '--author', 'Ana'] },
    { what: 'Saved by an email', fill: { 'Saved by': 'ben@studio.example' }, args: ['--all', '--author', 'ben@studio.example'] },
    { what: 'Message contains "layout"', fill: { 'Message contains': 'layout' }, args: ['--all', '--grep', 'layout'] },
    { what: 'From 2026-03-01 until 2026-04-15', fill: { 'From date': '2026-03-01', 'Until date': '2026-04-15' }, args: ['--all', '--since', '2026-03-01', '--until', '2026-04-15'] },
    { what: 'Only video/shots', fill: { 'Only these files or folders': 'video/shots' }, args: ['--all', 'HEAD', 'video/shots'] },
    { what: 'Only art/banner.ai, notes', fill: { 'Only these files or folders': 'art/banner.ai, notes' }, args: ['--all', 'HEAD', 'art/banner.ai', 'notes'] },
    { what: 'Start from feature/grade-b', fill: { 'Start from': 'feature/grade-b' }, args: ['feature/grade-b'] },
    { what: 'Saved by Ana and message "grade"', fill: { 'Saved by': 'Ana', 'Message contains': 'grade' }, args: ['--all', '--author', 'Ana', '--grep', 'grade'] },
  ]
  for (const c of cases) {
    await rec.step(`Filter: ${c.what}`, async () => {
      await filters.getByRole('button', { name: 'Clear' }).click().catch(() => {})
      await page.waitForTimeout(300)
      for (const [label, value] of Object.entries(c.fill)) {
        if (label === 'Start from') await filters.getByLabel(label).selectOption({ label: value }).catch(async () => { await filters.getByLabel(label).click(); await page.getByRole('option', { name: value }).click() })
        else await filters.getByLabel(label).fill(value)
      }
      await filters.getByRole('button', { name: 'Apply filters' }).click()
      await page.waitForTimeout(1500)
    })
    s = await shown(page)
    const want = await log(rec, ...c.args)
    const alt = await log(rec, ...c.args.filter((a) => a !== '--all'))
    const ok = sameSet(s.oids, want.map((x) => x.oid)) || sameSet(s.oids, alt.map((x) => x.oid))
    rec.check(ok, `filter "${c.what}" equals log ${c.args.join(' ')}`, `app ${s.oids.length}, log ${want.length} (without --all ${alt.length})`)
    rec.note(`filter "${c.what}": app ${s.oids.length}, log ${want.length}, log without --all ${alt.length}`)
  }
  rec.note(`"Start from" choices: ${(await filters.getByLabel('Start from').innerText().catch(() => '')).replace(/\s+/g, ' ').slice(0, 200)}; From date input type: ${await filters.getByLabel('From date').getAttribute('type').catch(() => '?')}`)
  await rec.step('Clear the filters', async () => {
    await filters.getByRole('button', { name: 'Clear' }).click({ timeout: 3000 }).catch(() => rec.note('Clear is not clickable here'))
    await page.waitForTimeout(1200)
  })
  s = await shown(page)
  rec.check(sameSet(s.oids, all.map((c) => c.oid)), 'Clear brings back every version', `${s.oids.length} vs ${all.length}`)

  // --- keyboard
  await rec.step('Keyboard: focus the list, Arrow Down 3 times, End, Home', async () => {
    await list(page).focus()
    for (let i = 0; i < 3; i++) { await page.keyboard.press('ArrowDown'); await page.waitForTimeout(200) }
    await rec.shot('after 3 Arrow Down')
    await page.keyboard.press('End')
    await page.waitForTimeout(500)
    await rec.shot('after End')
    await page.keyboard.press('Home')
    await page.waitForTimeout(500)
  })
  const sel = await page.evaluate(() => {
    const app = (document.querySelector('#app') as unknown as { __vue_app__: { config: { globalProperties: { $pinia: { _s: Map<string, Record<string, unknown>> } } } } }).__vue_app__
    return String(app.config.globalProperties.$pinia._s.get('history')!.selectedOid ?? '')
  })
  rec.check(sel === all[0]!.oid || sel === s.oids[0], 'Home selects the newest version', `${sel.slice(0, 7)} vs ${s.oids[0]?.slice(0, 7)}`)
  rec.note(`detail panel for the newest version: ${(await page.getByTestId('commit-detail').innerText().catch(() => 'none')).replace(/\n+/g, ' / ').slice(0, 400)}`)
  await rec.finish()
  await closeTerm()
})

test('H-02 Graph lanes across five branches', async () => {
  const rec = await Recorder.start('H-02', 'history-graph', { gui: true })
  const page = rec.page!
  if (!existsSync(join(repo, '.mediagit'))) throw new Error('H-02 needs the repository made by H-01')
  await rec.cli(repo, ['branch', 'switch', 'main'], { quietFrame: true })
  const all = await log(rec, '--all')
  const byOid = new Map(all.map((c) => [c.oid, c]))
  const g = await rec.cli(repo, ['log', '--graph', '--all', '--oneline'])
  rec.note(`log --graph --all --oneline (first 24 lines):\n${g.stdout.split('\n').slice(0, 24).join('\n')}`)

  await rec.step('Open History, All branches', async () => {
    await toWelcome(page)
    await openRepo(page, repo)
    await go(page, 'history')
    await list(page).getByRole('option').first().waitFor({ timeout: 30_000 })
    await page.getByText('All branches', { exact: true }).first().click().catch(() => {})
    await page.waitForTimeout(900)
  })
  let s = await shown(page)
  rec.note(`graph row sample: ${JSON.stringify(s.rows.slice(0, 3)).slice(0, 700)}`)
  rec.check(s.rows.length === s.oids.length, 'every version has a graph row', `${s.rows.length} rows for ${s.oids.length} versions`)
  rec.check(s.head === all.find((c) => c.summary === 'grade pass 40: final banner')?.oid, 'the store knows the current version (main tip)', s.head.slice(0, 7))

  // Order rule: a version is never listed below one of its parents.
  const pos = new Map(s.oids.map((o, i) => [o, i]))
  const wrong = all.filter((c) => c.parents.some((p) => pos.has(p) && pos.get(p)! < pos.get(c.oid)!))
  rec.check(wrong.length === 0, 'no version is listed below its own parent', wrong.map((c) => c.summary).slice(0, 4).join(' | '))
  const merges = all.filter((c) => c.parents.length > 1)
  rec.note(`merge versions: ${merges.map((c) => c.summary).join(' | ')}`)
  rec.check(merges.length >= 4, 'setup: four merges exist (including the criss-cross)', String(merges.length))

  const laneOf = (row: unknown): number | null => {
    const r = row as Record<string, unknown>
    for (const k of ['lane', 'col', 'column', 'x']) if (typeof r?.[k] === 'number') return r[k] as number
    return null
  }
  const headRow = s.rows[pos.get(s.head) ?? 0]
  rec.note(`current version row: ${JSON.stringify(headRow).slice(0, 300)}; lane ${laneOf(headRow)}`)
  rec.check(laneOf(headRow) === 0 || laneOf(headRow) === null, 'the current branch tip sits in lane 0', String(laneOf(headRow)))
  // First-parent chain of main should stay in lane 0.
  const chain: string[] = []
  for (let c = byOid.get(s.head); c; c = byOid.get(c.parents[0] ?? '')) chain.push(c.oid)
  const off = chain.filter((o) => { const l = laneOf(s.rows[pos.get(o) ?? -1]); return l !== null && l !== 0 })
  rec.check(off.length === 0, 'the whole first-parent line of main stays in lane 0', `${off.length} of ${chain.length} off lane 0: ${off.slice(0, 3).map((o) => byOid.get(o)?.summary).join(' | ')}`)
  const lanes = s.rows.map(laneOf).filter((l): l is number => l !== null)
  rec.note(`lanes used: max ${lanes.length ? Math.max(...lanes) : 'n/a'}; graph cell widths in DOM: ${await page.locator('.gcell').first().evaluate((e) => (e as HTMLElement).style.width).catch(() => '?')}`)
  rec.check(!lanes.length || Math.max(...lanes) <= 5, 'the graph needs no more lanes than there are branches', String(lanes.length ? Math.max(...lanes) : 0))
  await rec.shot('graph top')
  await rec.step('Scroll the graph to the middle and the bottom', async () => {
    await list(page).evaluate((e) => { e.scrollTop = e.scrollHeight * 0.4 })
    await page.waitForTimeout(500)
    await rec.shot('graph middle')
    await list(page).evaluate((e) => { e.scrollTop = e.scrollHeight })
    await page.waitForTimeout(500)
  })
  const svgs = await list(page).locator('svg').count()
  rec.check(svgs > 0, 'graph cells are drawn (svg present)', String(svgs))
  const labels = await appText(page)
  rec.note(`branch labels on screen (bottom): ${['feature/titles', 'feature/sound', 'hotfix/credits', 'feature/grade-b', 'idea/untouched'].filter((b) => labels.includes(b)).join(', ')}`)

  await rec.step('Outside the app: switch to feature/grade-b. Back in the app (focus)', async () => {
    await rec.cli(repo, ['branch', 'switch', 'feature/grade-b'], { quietFrame: true })
    await refocus(page)
    await page.waitForTimeout(3500)
    await list(page).evaluate((e) => { e.scrollTop = 0 })
    await page.waitForTimeout(500)
  })
  s = await shown(page)
  const tipB = (await log(rec, 'feature/grade-b'))[0]!
  rec.check(s.head === tipB.oid, 'after a switch outside the app, History marks the new current version within 3.5 s', `${s.head.slice(0, 7)} vs ${tipB.oid.slice(0, 7)}`)
  rec.check((await page.getByTestId('branch-indicator').innerText()).includes('feature/grade-b'), 'the rail shows the new branch')
  const pos2 = new Map(s.oids.map((o, i) => [o, i]))
  rec.check(laneOf(s.rows[pos2.get(s.head) ?? 0]) === 0 || laneOf(s.rows[pos2.get(s.head) ?? 0]) === null, 'the new current branch moves to lane 0', String(laneOf(s.rows[pos2.get(s.head) ?? 0])))
  await rec.shot('graph after switching to feature/grade-b')
  await rec.step('This branch only, on feature/grade-b', async () => {
    await page.getByText('This branch only', { exact: true }).first().click()
    await page.waitForTimeout(900)
  })
  s = await shown(page)
  const mineB = await log(rec, 'feature/grade-b')
  rec.check(same(s.oids, mineB.map((c) => c.oid)), '"This branch only" follows the switched branch', `${s.oids.length} vs ${mineB.length}`)
  await page.getByText('All branches', { exact: true }).first().click().catch(() => {})
  await rec.cli(repo, ['branch', 'switch', 'main'], { quietFrame: true })
  await rec.finish()
  await closeTerm()
})

test('H-03 Version detail: a version with 20 mixed files', async () => {
  const rec = await Recorder.start('H-03', 'version-detail', { gui: true })
  const page = rec.page!
  if (!existsSync(join(repo, '.mediagit'))) throw new Error('H-03 needs the repository made by H-01')
  const mine = await log(rec)
  const mixed = mine.find((c) => c.summary === 'Twenty mixed files')!
  const banner = mine.find((c) => c.summary === 'grade pass 40: final banner')!
  const detail = page.getByTestId('commit-detail')
  const pick = async (summary: string) => {
    await page.getByPlaceholder('Search versions').fill(summary)
    await page.waitForTimeout(1000)
    await list(page).getByRole('option').first().click()
    await page.waitForTimeout(1500)
  }
  const tiles = () => detail.getByLabel('Files this version changed')
  const imgs = () => tiles().locator('img').evaluateAll((els) => (els as HTMLImageElement[]).map((i) => ({ alt: i.alt, title: i.closest('[title]')?.getAttribute('title') ?? '', ok: i.complete && i.naturalWidth > 0, w: i.naturalWidth })))

  await rec.step('Open History and pick "Twenty mixed files"', async () => {
    await toWelcome(page)
    await openRepo(page, repo)
    await go(page, 'history')
    await list(page).getByRole('option').first().waitFor({ timeout: 30_000 })
    await pick('Twenty mixed files')
    await page.waitForTimeout(2000)
    const more = detail.getByRole('button', { name: /Show all/ })
    rec.note(`"Show all" button: ${(await more.innerText().catch(() => 'none'))}`)
    await more.click().catch(() => {})
    await page.waitForTimeout(5000)
  })
  let text = (await detail.innerText()).replace(/\n+/g, ' / ')
  rec.note(`detail: ${text.slice(0, 700)}`)
  rec.check(text.includes('Ana Ruiz'), 'the author is shown', text.slice(0, 200))
  rec.check(text.includes(mixed.oid.slice(0, 7)), 'the short id is shown')
  rec.note(`date shown for 2026-06-20T09:00Z: ${text.match(/(20 Jun|Jun 20|2026-06-20|\d+ (months|mo|weeks|w|days|d) ago)[^/]*/)?.[0] ?? 'not found'}`)
  const names = Object.values(MIXED).map((p) => p.split('/').pop()!)
  const titles = await tiles().locator('[title]').evaluateAll((els) => els.map((e) => e.getAttribute('title') ?? ''))
  rec.check(names.every((n) => titles.some((t) => t.endsWith(n)) || text.includes(n)), 'all 20 changed files are listed', `missing: ${names.filter((n) => !titles.some((t) => t.endsWith(n)) && !text.includes(n)).join(', ')}`)
  rec.check(/20 /.test(text), 'the count "20" is shown', text.match(/Changes in this version[^/]*\/[^/]*/)?.[0])
  let pics = await imgs()
  rec.note(`thumbnails: ${pics.length} images, ${pics.filter((p) => p.ok).length} loaded: ${JSON.stringify(pics.map((p) => `${(p.title || p.alt).split('/').pop()}:${p.ok ? p.w : 'BROKEN'}`))}`)
  rec.check(pics.every((p) => p.ok), 'no broken thumbnail image', JSON.stringify(pics.filter((p) => !p.ok)).slice(0, 300))
  const imageNames = ['plan.svg', 'photo.jpeg', 'photo2.jpeg', 'render.webp', 'turntable.gif']
  rec.check(imageNames.every((n) => pics.some((p) => (p.title || p.alt).endsWith(n) && p.ok)), 'every image file has a real thumbnail', `without one: ${imageNames.filter((n) => !pics.some((p) => (p.title || p.alt).endsWith(n) && p.ok)).join(', ')}`)
  await rec.shot('thumbnail grid')

  await rec.step('Click the tile of photo.jpeg', async () => {
    await tiles().locator('[title$="photo.jpeg"]').first().click()
    await page.waitForTimeout(2500)
  })
  const picked = (await page.getByTestId('picked-file').innerText().catch(() => '')).replace(/\n+/g, ' / ')
  const insp = (await page.getByTestId('inspector').innerText().catch(() => '')).replace(/\n+/g, ' / ')
  rec.note(`picked file: ${picked.slice(0, 200)} | details: ${insp.slice(0, 400)}`)
  const prev = await page.evaluate(() => { const i = document.querySelector('[data-testid="preview"] img') as HTMLImageElement | null; return i ? `${i.naturalWidth}x${i.naturalHeight}` : 'no image' })
  rec.check(prev === '1248x832', 'the file preview from history is the real picture (1248x832)', prev)

  await rec.step('Pick the newer version "grade pass 40: final banner"', async () => {
    await pick('grade pass 40: final banner')
    await page.waitForTimeout(2000)
  })
  text = (await detail.innerText()).replace(/\n+/g, ' / ')
  const titles2 = await tiles().locator('[title]').evaluateAll((els) => els.map((e) => e.getAttribute('title') ?? ''))
  rec.check(titles2.length === 1 && titles2[0]!.endsWith('art/banner.ai'), 'the grid now shows only that version\'s file (no stale tiles)', JSON.stringify(titles2))
  rec.check(text.includes(banner.oid.slice(0, 7)), 'the detail header follows the selection')
  pics = await imgs()
  rec.note(`thumbnails for an .ai file: ${JSON.stringify(pics)}`)
  const sh = await rec.cli(repo, ['show', banner.oid])
  rec.note(`CLI show: ${(sh.stdout + sh.stderr).replace(/\n+/g, ' / ').slice(0, 300)}`)

  await rec.step('Files at this version (the whole tree)', async () => {
    const tab = detail.getByText('Files at this version', { exact: false }).first()
    await tab.click().catch(() => {})
    await page.waitForTimeout(1500)
  })
  text = (await detail.innerText()).replace(/\n+/g, ' / ')
  const lsr = await rec.cli(repo, ['ls-files', '--machine'], { quietFrame: true })
  const tracked = (JSON.parse(lsr.stdout) as { data: { files: unknown[] } }).data.files.length
  rec.note(`whole-tree view: ${text.slice(0, 400)}; ls-files at the tip has ${tracked} files`)
  rec.check(text.replace(/,/g, '').includes(String(tracked)), 'the whole-tree view counts the same files as ls-files at the tip', `${tracked}`)

  await rec.step('Pick a merge version: "Merge sound into main"', async () => {
    await pick('Merge sound into main')
    await page.waitForTimeout(2000)
  })
  text = (await detail.innerText()).replace(/\n+/g, ' / ')
  rec.note(`merge version detail: ${text.slice(0, 500)}`)
  rec.check(/merge|two|parents|joined|feature\/sound/i.test(text), 'a merge version says it is a merge')
  rec.check((await visibleErrors(page)).filter((e) => /fail|error|could not/i.test(e)).length === 0, 'no error panels', JSON.stringify(await visibleErrors(page)).slice(0, 300))
  await page.getByPlaceholder('Search versions').fill('')
  await rec.finish()
  await closeTerm()
})
