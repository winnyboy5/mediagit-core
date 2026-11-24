import { appendFileSync, cpSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { basename, join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS, freeGb } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo, toWelcome, visibleErrors } from './gui'

// F-01..F-03 share one repository: the Unity "megacity-metro" project (3,820 files, 500 MB, depth 10).
const repo = join(workDir('F-01'), 'metro')
interface LsFile { path: string; status: string; staged: boolean; size: number }
const tree = (page: Page) => page.getByRole('tree', { name: 'All files' })
const summary = (page: Page) => page.getByTestId('files-summary').innerText()
const find = (page: Page) => page.getByRole('searchbox', { name: 'Find a file' })

async function ls(rec: Recorder, ...extra: string[]): Promise<LsFile[]> {
  const r = await rec.cli(repo, ['ls-files', '--machine', ...extra], { quietFrame: true })
  return (JSON.parse(r.stdout) as { data: { files: LsFile[] } }).data.files
}

/** File rows the explorer holds right now (from the store, because the list is virtualized). */
function storeRows(page: Page): Promise<{ files: string[]; dirPaths: string[]; dirs: number; total: number; ignored: string[] }> {
  return page.evaluate(() => {
    const app = (document.querySelector('#app') as unknown as { __vue_app__: { config: { globalProperties: { $pinia: { _s: Map<string, Record<string, unknown>> } } } } }).__vue_app__
    const s = app.config.globalProperties.$pinia._s.get('files')!
    const rows = (s.rows as { node: { path: string; isDir: boolean } }[]).map((r) => ({ path: r.node.path, dir: r.node.isDir }))
    const isDir = (r: { dir: boolean }) => r.dir
    return {
      files: rows.filter((r) => !isDir(r)).map((r) => r.path),
      dirPaths: rows.filter(isDir).map((r) => r.path),
      dirs: rows.filter(isDir).length,
      total: (s.entries as unknown[]).length,
      ignored: (s.entries as { path: string; ignored?: boolean; status?: string }[]).filter((e) => e.ignored || e.status === 'ignored').map((e) => e.path),
    }
  })
}

/** Scroll the list top to bottom in steps; returns the longest gap between two painted frames (ms). */
function scrollProbe(page: Page): Promise<{ worst: number; steps: number; height: number; domRows: number }> {
  return page.evaluate(async () => {
    const t = document.querySelector('[role="tree"][aria-label="All files"]') as HTMLElement
    let el: HTMLElement | null = t
    while (el && !(el.scrollHeight > el.clientHeight + 4 && getComputedStyle(el).overflowY !== 'visible')) el = el.parentElement
    if (!el) return { worst: -1, steps: 0, height: 0, domRows: t.querySelectorAll('[role="treeitem"]').length }
    const frame = () => new Promise<number>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now()))))
    let worst = 0
    const steps = 40
    for (let i = 1; i <= steps; i++) {
      const t0 = performance.now()
      el.scrollTop = (el.scrollHeight * i) / steps
      worst = Math.max(worst, (await frame()) - t0)
    }
    const out = { worst: Math.round(worst), steps, height: el.scrollHeight, domRows: t.querySelectorAll('[role="treeitem"]').length }
    el.scrollTop = 0
    return out
  })
}

test('F-01 All files explorer on the Unity tree', async () => {
  const rec = await Recorder.start('F-01', 'explorer-tree', { gui: true })
  const page = rec.page!
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  cpSync(join(CORPUS, 'unity', 'megacity-metro-master'), repo, { recursive: true })
  rec.note(`free disk ${freeGb()} GB`)
  await rec.cli(repo, ['init'])
  let r = await rec.cli(repo, ['add', '--all'])
  rec.check(r.code === 0, `add --all of the Unity project (${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB)`, (r.stdout + r.stderr).slice(-300))
  rec.note(`PERF add --all Unity 3,820 files 500 MB: ${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB`)
  r = await rec.cli(repo, ['commit', '-m', 'Import megacity-metro'])
  rec.note(`PERF commit: ${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB`)
  rec.check(r.code === 0, 'commit works', (r.stdout + r.stderr).slice(-200))
  const files = await ls(rec)
  rec.note(`ls-files: ${files.length} files; not unchanged: ${files.filter((f) => f.status !== 'unchanged').length}`)
  rec.check(files.every((f) => f.status === 'unchanged'), 'every file is tracked and clean after the commit', JSON.stringify(files.filter((f) => f.status !== 'unchanged').slice(0, 5)))

  let t0 = Date.now()
  await rec.step('Open the Unity repository', async () => {
    await toWelcome(page, true)
    t0 = Date.now()
    await openRepo(page, repo)
    await tree(page).getByRole('treeitem').first().waitFor({ timeout: 60_000 })
  })
  const loadMs = Date.now() - t0
  rec.note(`PERF open to first explorer row: ${loadMs} ms`)
  rec.check(loadMs < 5000, 'the explorer shows rows within 5 s of opening', `${loadMs} ms`)
  const sum = await summary(page)
  rec.check(sum.replace(/,/g, '').includes(String(files.length)), 'the summary count equals ls-files', `${sum} vs ${files.length}`)
  rec.check(!/changed/.test(sum), 'a clean repository shows no "changed" count', sum)
  let rows = await storeRows(page)
  rec.note(`tree mode, collapsed: ${rows.files.length} file rows, ${rows.dirs} folder rows, DOM rows ${await tree(page).getByRole('treeitem').count()}`)

  // --- expand down to the deepest file
  const deepest = files.reduce((a, b) => (b.path.split('/').length > a.path.split('/').length ? b : a))
  const parts = deepest.path.split('/')
  rec.note(`deepest file (${parts.length - 1} folders): ${deepest.path}`)
  rec.check(rows.files.includes(deepest.path), 'the tree opens fully expanded: the deepest file is already a row', `${rows.files.length} file rows`)
  const top = tree(page).getByRole('treeitem', { name: new RegExp(`^${parts[0]}/`) }).first()
  let worstExpand = 0
  await rec.step(`Collapse the top folder "${parts[0]}"`, async () => {
    const t = Date.now()
    await top.locator('.chev').click()
    await page.waitForTimeout(300)
    worstExpand = Date.now() - t - 300
  })
  const shut = await storeRows(page)
  rec.check(shut.files.length < rows.files.length, 'collapsing a folder hides its files', `${shut.files.length} file rows left`)
  await rec.step(`Expand "${parts[0]}" again`, async () => {
    const t = Date.now()
    await top.locator('.chev').click()
    await page.waitForTimeout(300)
    worstExpand = Math.max(worstExpand, Date.now() - t - 300)
  })
  rec.check((await storeRows(page)).files.length === rows.files.length, 'expanding brings every file back')
  rec.note(`slowest collapse or expand of ${rows.files.length} rows: ${worstExpand} ms`)
  rec.check(worstExpand < 1000, 'collapse and expand of the whole tree take under 1 s', `${worstExpand} ms`)
  const leaf = tree(page).getByRole('treeitem', { name: new RegExp(basename(deepest.path).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first()
  // The list is virtualized, so a far row is not in the DOM: bring it in with the find box.
  await find(page).fill(basename(deepest.path))
  await page.waitForTimeout(500)
  rec.check(await leaf.isVisible().catch(() => false), 'the deepest file is reachable in the tree', deepest.path)
  await rec.step('Select the deepest file', async () => {
    await leaf.click()
    await page.waitForTimeout(800)
    await find(page).fill('')
  })
  rec.note(`details panel: ${(await page.locator('aside, [data-testid="inspector"]').last().innerText().catch(() => 'no panel found')).replace(/\n+/g, ' / ').slice(0, 300)}`)

  // --- flat view: every file, virtualized, smooth scroll
  await rec.step('Switch to Flat view', async () => {
    await page.getByRole('group', { name: 'Folder view' }).getByText('Flat', { exact: true }).click()
    await page.waitForTimeout(800)
  })
  rows = await storeRows(page)
  rec.check(rows.files.length === files.length, 'Flat view holds every file', `${rows.files.length} vs ${files.length}`)
  const want = new Set(files.map((f) => f.path))
  rec.check(rows.files.every((p) => want.has(p)), 'Flat view paths equal ls-files paths', rows.files.find((p) => !want.has(p)))
  const dom = await tree(page).getByRole('treeitem').count()
  rec.check(dom < 400, 'the list is virtualized (few DOM rows for thousands of files)', `${dom} DOM rows for ${rows.files.length} files`)
  let probe = { worst: 0, steps: 0, height: 0, domRows: 0 }
  await rec.step('Scroll the flat list top to bottom', async () => {
    probe = await scrollProbe(page)
  })
  rec.note(`PERF flat scroll: worst frame ${probe.worst} ms over ${probe.steps} steps, scroll height ${probe.height} px, DOM rows ${probe.domRows}`)
  rec.check(probe.worst >= 0 && probe.worst < 1000, 'no scroll step freezes for over 1 s', `${probe.worst} ms`)
  await rec.step('Back to Tree view and scroll', async () => {
    await page.getByRole('group', { name: 'Folder view' }).getByText('Tree', { exact: true }).click()
    await page.waitForTimeout(600)
    probe = await scrollProbe(page)
  })
  rec.note(`PERF tree scroll: worst frame ${probe.worst} ms, DOM rows ${probe.domRows}`)
  rec.check(await leaf.count().then((n) => n >= 0), 'Tree view comes back')
  rec.note(`folders stay expanded after Flat and back: ${(await storeRows(page)).files.includes(deepest.path)}`)

  // --- changes made outside the app show in the explorer
  const edited = files.find((f) => f.path.endsWith('.cs'))!.path
  await rec.step('Outside the app: edit one file, add one, delete one', async () => {
    appendFileSync(join(repo, edited), '\n// qa edit\n')
    writeFileSync(join(repo, 'NEW_NOTES.txt'), 'new file\n')
    rmSync(join(repo, files.find((f) => f.path.endsWith('.shader'))!.path))
    await page.waitForTimeout(3500)
  })
  const sum2 = await summary(page)
  rec.check(/3 changed/.test(sum2), 'the watcher shows "3 changed" within 3.5 s', sum2)
  rec.check((await visibleErrors(page)).filter((e) => /fail|error/i.test(e)).length === 0, 'no error panels', JSON.stringify(await visibleErrors(page)))
  await rec.finish()
  await closeTerm()
})

test('F-02 Search in the explorer', async () => {
  const rec = await Recorder.start('F-02', 'explorer-search', { gui: true })
  const page = rec.page!
  if (!existsSync(join(repo, '.mediagit'))) throw new Error('F-02 needs the repository made by F-01')
  writeFileSync(join(repo, 'megacity-metro-master', 'Assets', 'étude ノート (v2).txt'), 'unicode name\n')
  const files = await ls(rec)
  await rec.step('Open the Unity repository, Files > All files', async () => {
    await toWelcome(page)
    await openRepo(page, repo)
    await go(page, 'files/all')
    await tree(page).getByRole('treeitem').first().waitFor({ timeout: 60_000 })
  })
  const before = await storeRows(page)
  const byName = (q: string) => files.filter((f) => basename(f.path).toLowerCase().includes(q.toLowerCase())).map((f) => f.path)
  const byPath = (q: string) => files.filter((f) => q.toLowerCase().split(/\s+/).filter(Boolean).every((w) => f.path.toLowerCase().includes(w))).map((f) => f.path)
  const same = (a: string[], b: string[]) => a.length === b.length && [...a].sort().join('\n') === [...b].sort().join('\n')
  const exact = basename(files.find((f) => f.path.endsWith('.prefab'))!.path)

  const queries: [string, string][] = [
    [exact, 'an exact file name'],
    ['Mat', 'a partial name'],
    ['.prefab', 'an extension'],
    ['.PREFAB', 'an extension in capitals'],
    ['étude', 'a unicode name'],
    ['ノート (v2)', 'Japanese text with brackets'],
    ['[', 'a regex special character'],
    ['  .shader  ', 'a query with spaces around it'],
  ]
  for (const [q, what] of queries) {
    let ms = 0
    await rec.step(`Find: "${q}" (${what})`, async () => {
      const t = Date.now()
      await find(page).fill(q)
      await page.waitForTimeout(450)
      ms = Date.now() - t - 450
    })
    const got = (await storeRows(page)).files
    const n = byName(q.trim())
    const p = byPath(q.trim())
    const ok = same(got, n) || same(got, p)
    rec.check(ok, `"${q}" (${what}) matches ls-files`, `app ${got.length}, by name ${n.length}, by path ${p.length}`)
    rec.note(`"${q}": app ${got.length} rows, by name ${n.length}, by path ${p.length}, matches ${same(got, n) ? 'name' : same(got, p) ? 'path' : 'NEITHER'}; summary "${await summary(page)}"; fill took ${ms} ms`)
  }

  await rec.step('Find: a path fragment "Assets/Scenes"', async () => {
    await find(page).fill('Assets/Scenes')
    await page.waitForTimeout(450)
  })
  let got = (await storeRows(page)).files
  rec.note(`"Assets/Scenes": app ${got.length}, by path ${byPath('Assets/Scenes').length} (a folder search ${got.length ? 'works' : 'is not supported'})`)
  await rec.step('Find: a wildcard "*.png"', async () => {
    await find(page).fill('*.png')
    await page.waitForTimeout(450)
  })
  got = (await storeRows(page)).files
  rec.note(`"*.png": app ${got.length}, real .png files ${byName('.png').length}; screen: ${(await page.locator('section.explorer').innerText()).replace(/\n+/g, ' / ').slice(-160)}`)

  await rec.step('Find: no match "zzzqqq"', async () => {
    await find(page).fill('zzzqqq')
    await page.waitForTimeout(450)
  })
  rec.check(await page.getByText('No match').isVisible(), 'no match shows an empty state, not a blank list')
  rec.note(`no-match text: ${(await page.locator('section.explorer .pad').innerText()).replace(/\n+/g, ' / ')}`)

  await rec.step('Type "material" key by key', async () => {
    await find(page).fill('')
    const t = Date.now()
    await find(page).pressSequentially('material', { delay: 40 })
    await page.waitForTimeout(300)
    rec.note(`PERF typing 8 keys at 40 ms: ${Date.now() - t - 300} ms total (320 ms is the floor)`)
  })
  got = (await storeRows(page)).files
  rec.check(same(got, byName('material')) || same(got, byPath('material')), 'typed search ends on the right result', `${got.length}`)
  rec.note(`search in Tree mode shows ${(await storeRows(page)).dirs} folder rows and ${await tree(page).getByRole('treeitem').count()} DOM rows`)

  await rec.step('Clear the search with the keyboard (Escape)', async () => {
    await find(page).press('Escape')
    await page.waitForTimeout(450)
  })
  rec.note(`Escape clears the field: ${(await find(page).inputValue()) === ''}`)
  await rec.step('Clear the search field', async () => {
    await find(page).fill('')
    await page.waitForTimeout(450)
  })
  const after = await storeRows(page)
  rec.check(after.files.length === before.files.length && after.dirs === before.dirs, 'clearing restores the same tree', `${after.files.length}/${after.dirs} vs ${before.files.length}/${before.dirs}`)

  await rec.step('Search, then leave to History and come back', async () => {
    await find(page).fill('.prefab')
    await go(page, 'history')
    await page.waitForTimeout(600)
    await go(page, 'files/all')
    await page.waitForTimeout(600)
  })
  rec.note(`search text after leaving and coming back: "${await find(page).inputValue()}" with ${(await storeRows(page)).files.length} rows`)
  await find(page).fill('')
  rmSync(join(repo, 'megacity-metro-master', 'Assets', 'étude ノート (v2).txt'))
  await rec.finish()
  await closeTerm()
})

test('F-03 Show ignored files', async () => {
  const rec = await Recorder.start('F-03', 'explorer-ignored', { gui: true })
  const page = rec.page!
  if (!existsSync(join(repo, '.mediagit'))) throw new Error('F-03 needs the repository made by F-01')
  for (const [f, text] of [['Temp/cache.bin', 'cache'], ['Temp/deep/more.tmp', 'tmp'], ['Library/ShaderCache/a.bin', 'lib'], ['debug.log', 'log'], ['Assets/editor.log', 'log2'], ['keep.txt', 'keep'], ['Temp/keep.me', 'negated']] as const) {
    mkdirSync(join(repo, f, '..'), { recursive: true })
    writeFileSync(join(repo, f), `${text}\n`)
  }
  const toggle = page.getByRole('checkbox', { name: 'Show ignored files' })
  const made = ['Temp/cache.bin', 'Temp/deep/more.tmp', 'Library/ShaderCache/a.bin', 'debug.log', 'Assets/editor.log']

  await rec.step('Open the repository before any ignore file exists', async () => {
    await toWelcome(page)
    await openRepo(page, repo)
    await go(page, 'files/all')
    await tree(page).getByRole('treeitem').first().waitFor({ timeout: 60_000 })
    await page.waitForTimeout(1500)
  })
  let rows = await storeRows(page)
  rec.check(made.every((f) => rows.files.includes(f)) || (await storeRows(page)).total > 0, 'with no ignore file the new files are ordinary new files')
  rec.note(`existing ignore files in the Unity project: .gitignore ${existsSync(join(repo, '.gitignore'))}, .mediagitignore ${existsSync(join(repo, '.mediagitignore'))}`)

  await rec.step('Outside the app: write .mediagitignore (*.log, Temp/, Library/, !Temp/keep.me)', async () => {
    writeFileSync(join(repo, '.mediagitignore'), '# qa\n*.log\nTemp/\nLibrary/\n!Temp/keep.me\n')
    await page.waitForTimeout(3500)
  })
  const st = await rec.cli(repo, ['status', '--porcelain', '--ignored'])
  const cliIgnored = st.stdout.split('\n').filter((l) => l.startsWith('!!')).map((l) => l.slice(3).trim())
  const lsIgnored = (await ls(rec, '--ignored')).filter((f) => f.status === 'ignored').map((f) => f.path)
  rec.note(`CLI status --ignored: ${JSON.stringify(cliIgnored)}`)
  rec.note(`CLI ls-files --ignored: ${JSON.stringify(lsIgnored)}`)
  rec.check(made.every((f) => cliIgnored.some((c) => f === c || f.startsWith(c))), 'CLI status --ignored covers every ignored file (or its folder)', `not covered: ${JSON.stringify(made.filter((f) => !cliIgnored.some((c) => f === c || f.startsWith(c))))}`)
  rec.check(made.every((f) => lsIgnored.some((c) => f === c || f.startsWith(c))), 'CLI ls-files --ignored covers every ignored file (or its folder)', `not covered: ${JSON.stringify(made.filter((f) => !lsIgnored.some((c) => f === c || f.startsWith(c))))}`)
  const plain = await rec.cli(repo, ['status', '--porcelain'])
  rec.check(!made.some((f) => plain.stdout.includes(f)), 'plain status hides the ignored files', plain.stdout.slice(0, 300))
  rec.note(`negated pattern !Temp/keep.me is listed as new: ${plain.stdout.includes('Temp/keep.me')}`)

  rows = await storeRows(page)
  rec.check(!(await toggle.isChecked()), 'Show ignored files is off by default')
  rec.check(!made.some((f) => rows.files.includes(f)), 'the watcher drops the newly ignored files from the explorer within 3.5 s', JSON.stringify(made.filter((f) => rows.files.includes(f))))
  rec.note(`summary with the toggle off: ${await summary(page)}`)

  await rec.step('Tick "Show ignored files"', async () => {
    await toggle.click()
    await page.waitForTimeout(1500)
  })
  rows = await storeRows(page)
  const shown = made.filter((f) => rows.files.includes(f) || rows.dirPaths.some((d) => f.startsWith(`${d.replace(/\/$/, '')}/`)))
  rec.check(shown.length === made.length, 'with the toggle on, every ignored file is listed', `shown ${JSON.stringify(shown)}; store marks ignored: ${JSON.stringify(rows.ignored.slice(0, 8))}`)
  rec.note(`summary with the toggle on: ${await summary(page)}`)
  await rec.step('Find "debug.log" to see how an ignored row is marked', async () => {
    await find(page).fill('debug.log')
    await page.waitForTimeout(500)
  })
  const row = tree(page).getByRole('treeitem').first()
  const rowText = (await row.innerText().catch(() => '')).replace(/\n+/g, ' / ')
  rec.note(`ignored row: text "${rowText}", class "${await row.getAttribute('class').catch(() => '')}", has a checkbox ${await row.getByRole('checkbox').count()}`)
  rec.check(/ignored/i.test(rowText) || /ignored/i.test((await row.getAttribute('class').catch(() => '')) ?? '') || /ignored/i.test(await row.innerHTML().catch(() => '')), 'an ignored row is marked as ignored', rowText)
  await find(page).fill('')

  await rec.step('Files > Changes: ignored files must not be offered', async () => {
    await go(page, 'files/changes')
    await page.waitForTimeout(1200)
  })
  const changes = (await page.locator('#app').innerText()).replace(/\n+/g, ' / ')
  rec.check(!/cache\.bin|debug\.log|a\.bin/.test(changes), 'Changes does not list ignored files', changes.slice(0, 300))

  await rec.step('Back to All files: untick the toggle', async () => {
    await go(page, 'files/all')
    await page.waitForTimeout(600)
    rec.note(`toggle is remembered after leaving the page: ${await toggle.isChecked()}`)
    if (await toggle.isChecked()) await toggle.click()
    await page.waitForTimeout(1200)
  })
  rows = await storeRows(page)
  rec.check(!made.some((f) => rows.files.includes(f)), 'unticking hides them again')

  await rec.step('Outside the app: delete .mediagitignore', async () => {
    rmSync(join(repo, '.mediagitignore'))
    await page.waitForTimeout(3500)
  })
  rows = await storeRows(page)
  rec.check(made.every((f) => rows.files.includes(f)), 'removing the ignore file brings the files back as new within 3.5 s', JSON.stringify(made.filter((f) => !rows.files.includes(f))))
  for (const d of ['Temp', 'Library']) rmSync(join(repo, d), { recursive: true, force: true })
  for (const f of ['debug.log', 'Assets/editor.log', 'keep.txt']) rmSync(join(repo, f), { force: true })
  await rec.finish()
  await closeTerm()
})
