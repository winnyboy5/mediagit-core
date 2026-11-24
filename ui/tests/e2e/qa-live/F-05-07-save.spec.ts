import { appendFileSync, cpSync, mkdirSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS, freeGb } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo, sampleProgress, toWelcome, visibleErrors, type ProgressSample } from './gui'
import { copyCorpus, intCheck, restoreCheck, sha256, walk } from './integrity'

interface Commit { oid: string; parents: string[]; author: { name: string; email: string }; time: number; summary: string; message: string }
const MIX: Record<string, string> = {
  '12690118_5053480.ai': 'design/poster.ai',
  '1965_ac_shelby_427_cobra_sc.glb': 'models/cobra.glb',
  'pdf/1710.02185v1.pdf': 'docs/paper.pdf',
  'freepik__talk__71826.jpeg': 'refs/talk.jpeg',
}
const dock = (page: Page) => page.getByTestId('save-dock')
const save = (page: Page) => page.getByRole('button', { name: 'Save version', exact: true })
const msg = (page: Page) => dock(page).getByLabel('Message')
const count = (page: Page) => page.getByTestId('include-count').innerText()
const dockText = async (page: Page) => (await dock(page).innerText()).replace(/\n+/g, ' / ')

async function log(rec: Recorder, repo: string): Promise<Commit[]> {
  const r = await rec.cli(repo, ['log', '--machine'], { quietFrame: true })
  return (JSON.parse(r.stdout) as { data: { commits: Commit[] } }).data.commits
}
async function changesPage(page: Page) {
  await go(page, 'files/changes')
  await page.waitForTimeout(900)
}
async function includeAll(page: Page) {
  const b = page.getByRole('button', { name: 'Include all', exact: true })
  if (await b.isVisible().catch(() => false)) await b.click()
  await page.waitForTimeout(900)
}
async function openMore(page: Page) {
  const more = page.getByTestId('commit-more')
  if ((await more.getAttribute('open')) === null) await more.locator('summary').click()
  await page.waitForTimeout(200)
}
/** Wait until the commit lands (the log grows) or 20 s pass. */
async function waitCommits(rec: Recorder, repo: string, n: number): Promise<Commit[]> {
  for (let i = 0; i < 40; i++) {
    const c = await log(rec, repo)
    if (c.length >= n) return c
    await new Promise((r) => setTimeout(r, 500))
  }
  return log(rec, repo)
}

test('F-05 Save a version: guards, shortcut and every option', async () => {
  const rec = await Recorder.start('F-05', 'save-version', { gui: true })
  const page = rec.page!
  const repo = workDir('F-05')
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  await rec.cli(repo, ['init'])
  const want = await copyCorpus(repo, MIX)

  await rec.step('Open the repository, Files > Changes (4 new files, none included)', async () => {
    await toWelcome(page, true)
    await openRepo(page, repo)
    await changesPage(page)
  })
  rec.check(await save(page).isDisabled(), 'Save version is disabled with nothing included')
  rec.note(`dock with nothing included: ${await dockText(page)}`)

  await rec.step('Type a message with nothing included', async () => { await msg(page).fill('first plates') })
  rec.check(await save(page).isDisabled(), 'a message alone does not enable Save')
  await rec.step('Include all, clear the message', async () => {
    await includeAll(page)
    await msg(page).fill('')
  })
  rec.check(await save(page).isDisabled(), 'Save is disabled with an empty message')
  rec.note(`hint with an empty message: ${await dockText(page)}`)
  await rec.step('Message of spaces only', async () => { await msg(page).fill('     ') })
  rec.check(await save(page).isDisabled(), 'a message of spaces only does not enable Save')

  // --- Ctrl+Enter
  await rec.step('Type "First plates for the dragon shot", press Ctrl+Enter', async () => {
    await msg(page).fill('First plates for the dragon shot')
    await rec.shot('ready to save')
    await msg(page).press('Control+Enter')
  })
  let commits = await waitCommits(rec, repo, 1)
  rec.check(commits.length === 1 && commits[0]!.summary === 'First plates for the dragon shot', 'Ctrl+Enter saves the version', JSON.stringify(commits[0] ?? null).slice(0, 200))
  rec.check(commits[0]?.author.name === 'QA Live', 'the configured identity is the author', JSON.stringify(commits[0]?.author))
  await page.waitForTimeout(1200)
  await rec.shot('after the first save')
  rec.check((await msg(page).inputValue().catch(() => '')) === '', 'the message box is cleared after saving')
  const st = await rec.cli(repo, ['status', '--porcelain'])
  rec.check(st.stdout.trim() === '', 'the tree is clean after saving', st.stdout.slice(0, 200))
  rec.note(`Changes page after a save: ${(await page.locator('#app').innerText()).replace(/\n+/g, ' / ').slice(-260)}`)

  // --- author, date, signoff
  await rec.step('New file. More options: Author, Date, Signed-off-by', async () => {
    writeFileSync(join(repo, 'notes.txt'), 'grade notes v1\n')
    await page.waitForTimeout(3200)
    await changesPage(page)
    await includeAll(page)
    await msg(page).fill('Grade notes')
    await openMore(page)
    await dock(page).getByLabel('Author').fill('Ana Ruiz <ana@studio.example>')
    await dock(page).getByLabel('Date').fill('2026-01-15T10:30:00Z')
    await dock(page).getByRole('checkbox', { name: 'Add a Signed-off-by line' }).click()
    await rec.shot('more options filled')
    await save(page).click()
  })
  commits = await waitCommits(rec, repo, 2)
  const c2 = commits[0]
  rec.check(commits.length === 2 && c2?.author.name === 'Ana Ruiz' && c2.author.email === 'ana@studio.example', '--author is honoured', JSON.stringify(c2?.author))
  rec.check(c2?.time === Date.parse('2026-01-15T10:30:00Z') / 1000, '--date is honoured', `${c2?.time} vs ${Date.parse('2026-01-15T10:30:00Z') / 1000}`)
  rec.check(/Signed-off-by: /.test(c2?.message ?? ''), '--signoff adds the line', (c2?.message ?? '').slice(0, 200))
  rec.note(`signoff line: ${(c2?.message ?? '').split('\n').find((l) => l.startsWith('Signed-off-by')) ?? 'none'} (author was overridden to Ana Ruiz)`)
  await page.waitForTimeout(1200)
  await changesPage(page)
  rec.note(`dock with no changes left: visible ${await dock(page).isVisible().catch(() => false)}`)

  // --- bad author, bad date
  writeFileSync(join(repo, 'notes.txt'), 'grade notes v2\n')
  await page.waitForTimeout(3200)
  await rec.step('Author without an email: "Ana"', async () => {
    await changesPage(page)
    await includeAll(page)
    await msg(page).fill('Bad author')
    await openMore(page)
    rec.check((await dock(page).getByLabel('Author').inputValue()) === '' && (await dock(page).getByLabel('Date').inputValue()) === '' && !(await dock(page).getByRole('checkbox', { name: 'Add a Signed-off-by line' }).isChecked()), 'BUG-013: Author, Date and Signed-off-by are reset after a save (one-time options do not carry to the next version)')
    rec.note(`options kept from the last save: author "${await dock(page).getByLabel('Author').inputValue()}", date "${await dock(page).getByLabel('Date').inputValue()}", signoff ${await dock(page).getByRole('checkbox', { name: 'Add a Signed-off-by line' }).isChecked()}`)
    await dock(page).getByLabel('Author').fill('Ana')
    await dock(page).getByLabel('Date').fill('')
    await page.waitForTimeout(300)
    if (!(await save(page).isDisabled())) await save(page).click()
    await page.waitForTimeout(2000)
  })
  commits = await log(rec, repo)
  let errs = await visibleErrors(page)
  rec.note(`bad author: commits ${commits.length}, dock: ${(await dockText(page)).slice(0, 300)}, alerts ${JSON.stringify(errs).slice(0, 300)}`)
  rec.check(commits.length === 2, 'a bad Author makes no version', `author saved as ${JSON.stringify(commits[0]?.author)}`)
  rec.check(/author|name|email/i.test((await dockText(page)) + errs.join(' ')), 'the app explains the bad Author')

  await rec.step('Date in words: "yesterday"', async () => {
    await changesPage(page)
    await includeAll(page)
    await msg(page).fill('Bad date')
    await openMore(page)
    await dock(page).getByLabel('Author').fill('')
    await dock(page).getByLabel('Date').fill('yesterday')
    await page.waitForTimeout(300)
    if (!(await save(page).isDisabled())) await save(page).click()
    await page.waitForTimeout(2000)
  })
  commits = await log(rec, repo)
  errs = await visibleErrors(page)
  rec.note(`bad date: commits ${commits.length}, time of newest ${commits[0]?.time}, dock: ${(await dockText(page)).slice(0, 300)}, alerts ${JSON.stringify(errs).slice(0, 300)}`)
  rec.check(commits.length === 2 || Math.abs((commits[0]?.time ?? 0) - Date.now() / 1000) < 172_800, 'a date in words is refused or understood (never a wrong date silently)', `${commits[0]?.time}`)
  let base = commits.length

  // --- dry run
  await rec.step('Preview only (dry run)', async () => {
    await changesPage(page)
    await includeAll(page)
    await msg(page).fill('Dry run only')
    await openMore(page)
    await dock(page).getByLabel('Date').fill('')
    await dock(page).getByLabel('Author').fill('')
    await dock(page).getByRole('checkbox', { name: 'Preview only' }).click()
    await page.waitForTimeout(300)
    rec.note(`button text in preview mode: "${await dock(page).locator('button[type="submit"]').innerText()}"`)
    await dock(page).locator('button[type="submit"]').click()
    await page.waitForTimeout(2500)
  })
  commits = await log(rec, repo)
  rec.check(commits.length === base, 'Preview only saves nothing', `${commits.length} vs ${base}`)
  rec.note(`preview note: ${(await page.getByTestId('preview-note').innerText().catch(() => 'none')).replace(/\n+/g, ' / ').slice(0, 300)}`)
  await rec.step('Untick Preview only, save for real', async () => {
    await openMore(page)
    const dry = dock(page).getByRole('checkbox', { name: 'Preview only' })
    if (await dry.isChecked()) await dry.click()
    await msg(page).fill('Grade notes v2')
    await save(page).click()
  })
  commits = await waitCommits(rec, repo, base + 1)
  rec.check(commits.length === base + 1 && commits[0]!.summary === 'Grade notes v2', 'the real save after a preview works', commits[0]?.summary)
  base = commits.length
  await page.waitForTimeout(1200)

  // --- message from a file
  const msgFile = join(repo, '..', 'F-05-message.txt')
  writeFileSync(msgFile, 'Lighting pass 3\n\nWarmer key light.\nUnicode: é ñ 日本語 🎬\n')
  writeFileSync(join(repo, 'notes.txt'), 'grade notes v3\n')
  await page.waitForTimeout(3200)
  await rec.step('Take the description from a file, message box left empty', async () => {
    await changesPage(page)
    await includeAll(page)
    await msg(page).fill('')
    await openMore(page)
    await dock(page).getByLabel('Take the description from a file').fill(msgFile)
    await page.waitForTimeout(300)
    await rec.shot('message from file')
    if (!(await save(page).isDisabled())) await save(page).click()
  })
  commits = await waitCommits(rec, repo, base + 1)
  rec.check(commits.length === base + 1 && commits[0]!.summary === 'Lighting pass 3', '--file gives the message', commits[0]?.summary)
  rec.check((commits[0]?.message ?? '').includes('日本語 🎬'), 'unicode and emoji in the message survive', (commits[0]?.message ?? '').slice(0, 120))
  base = commits.length
  await page.waitForTimeout(1200)

  // --- commit --all
  writeFileSync(join(repo, 'notes.txt'), 'grade notes v4\n')
  await page.waitForTimeout(3200)
  await rec.step('Nothing included. Tick "Stage every changed tracked file first"', async () => {
    await changesPage(page)
    await openMore(page)
    await dock(page).getByLabel('Take the description from a file').fill('')
    await msg(page).fill('Notes v4 with --all')
    await dock(page).getByRole('checkbox', { name: 'Stage every changed tracked file first' }).click()
    await page.waitForTimeout(300)
    rec.note(`include count ${await count(page)}, Save disabled ${await save(page).isDisabled()}`)
    if (!(await save(page).isDisabled())) await save(page).click()
  })
  commits = await waitCommits(rec, repo, base + 1)
  rec.check(commits.length === base + 1 && commits[0]!.summary === 'Notes v4 with --all', '--all saves a tracked edit that was not included', commits[0]?.summary)
  base = commits.length
  await page.waitForTimeout(1200)

  // --- allow empty
  await rec.step('No changes at all. Allow a version with no changes', async () => {
    await changesPage(page)
    const visible = await dock(page).isVisible().catch(() => false)
    rec.note(`save dock visible with no changes: ${visible}`)
    if (!visible) return
    await openMore(page)
    const all = dock(page).getByRole('checkbox', { name: 'Stage every changed tracked file first' })
    if (await all.isChecked()) await all.click()
    await dock(page).getByRole('checkbox', { name: 'Allow a version with no changes' }).click()
    await msg(page).fill('Milestone marker')
    await page.waitForTimeout(300)
    if (!(await save(page).isDisabled())) await save(page).click()
  })
  commits = await waitCommits(rec, repo, base + 1)
  rec.check(commits.length === base + 1 && commits[0]!.summary === 'Milestone marker', '--allow-empty is reachable in the app when nothing changed', `commits ${commits.length}, newest "${commits[0]?.summary}"`)
  base = commits.length

  // --- long, multi-line message
  const long = `Comp v12\n\n${'Rebalanced the fog layers and fixed the matte edge on the left tower. '.repeat(70)}\n- item one\n- item two`
  writeFileSync(join(repo, 'notes.txt'), 'grade notes v5\n')
  await page.waitForTimeout(3200)
  await rec.step('A 5,000 character message with line breaks', async () => {
    await changesPage(page)
    await includeAll(page)
    await openMore(page)
    const empty = dock(page).getByRole('checkbox', { name: 'Allow a version with no changes' })
    if (await empty.isChecked()) await empty.click()
    await msg(page).fill(long)
    await page.waitForTimeout(300)
    await save(page).click()
  })
  commits = await waitCommits(rec, repo, base + 1)
  rec.check(commits[0]?.message.trim() === long.trim(), 'a long multi-line message is stored exactly', `${commits[0]?.message.length} vs ${long.length} chars`)
  base = commits.length
  await page.waitForTimeout(1200)

  // --- BUG-011 from the app: a case-only rename
  await rec.step('Outside the app: rename design\\poster.ai to design\\Poster.ai', async () => {
    renameSync(join(repo, 'design', 'poster.ai'), join(repo, 'design', 'Poster.ai'))
    await page.waitForTimeout(3200)
    await changesPage(page)
  })
  rec.note(`Changes after a case-only rename: ${(await page.getByRole('tree', { name: 'Changes' }).innerText().catch(() => 'no tree')).replace(/\n+/g, ' / ').slice(0, 300)}`)
  await rec.step('Include all, save "Fix file name case"', async () => {
    await includeAll(page)
    rec.note(`include count: ${await count(page)}`)
    await msg(page).fill('Fix file name case')
    await save(page).click()
  })
  commits = await waitCommits(rec, repo, base + 1)
  await page.waitForTimeout(1500)
  const lsr = await rec.cli(repo, ['ls-files', '--machine'])
  const tracked = (JSON.parse(lsr.stdout) as { data: { files: { path: string; status: string }[] } }).data.files
  const posters = tracked.filter((f) => /design\/poster\.ai/i.test(f.path))
  rec.check(posters.length === 1 && posters[0]!.path === 'design/Poster.ai' && posters[0]!.status === 'unchanged', 'after the app saves a case-only rename, the version holds the new name only', JSON.stringify(posters))
  rec.note(`Changes after that save: ${(await page.locator('#app').innerText()).replace(/\n+/g, ' / ').slice(-300)}`)

  delete want['design/poster.ai']
  want['design/Poster.ai'] = await sha256(join(repo, 'design', 'Poster.ai'))
  await restoreCheck(rec, repo, want)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('F-07 Many small files: include and save 1,186 files in one go', async () => {
  const rec = await Recorder.start('F-07', 'many-small-files', { gui: true })
  const page = rec.page!
  const repo = workDir('F-07')
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  await rec.cli(repo, ['init'])
  cpSync(join(CORPUS, 'Medieval Village MegaKit[Standard]'), join(repo, 'MegaKit'), { recursive: true })
  cpSync(join(CORPUS, 'sketch'), join(repo, 'sketch'), { recursive: true })
  const all = walk(repo)
  const bytes = all.length
  rec.note(`${all.length} files copied, free disk ${freeGb()} GB`)
  const want: Record<string, string> = {}
  for (const f of all) want[f.slice(repo.length + 1).replace(/\\/g, '/')] = await sha256(f)

  let t0 = Date.now()
  await rec.step('Open the repository with 1,186 new files', async () => {
    await toWelcome(page, true)
    t0 = Date.now()
    await openRepo(page, repo)
    await changesPage(page)
    await page.getByRole('tree', { name: 'Changes' }).getByRole('treeitem').first().waitFor({ timeout: 60_000 })
  })
  rec.note(`PERF open to Changes list with ${bytes} new files: ${Date.now() - t0} ms`)
  const c0 = await count(page)
  rec.check(c0.replace(/,/g, '').includes(String(all.length)), 'the dock counts every new file', c0)

  const samples: ProgressSample[] = []
  let includeMs = 0
  let worstGap = 0
  await rec.step('Click Include all and watch the progress', async () => {
    const t = Date.now()
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    let last = Date.now()
    for (;;) {
      const s = await sampleProgress(page, t)
      worstGap = Math.max(worstGap, Date.now() - last)
      last = Date.now()
      samples.push(s)
      const done = (await count(page).catch(() => '')).replace(/,/g, '')
      if (new RegExp(`^${all.length} of ${all.length}\\b`).test(done)) break
      if (Date.now() - t > 600_000) break
      if (samples.length === 6) await rec.shot('include all in progress')
      await page.waitForTimeout(400)
    }
    includeMs = Date.now() - t
  })
  rec.note(`PERF Include all ${all.length} files: ${includeMs} ms, ${samples.length} samples, slowest UI answer ${worstGap} ms`)
  const seen = [...new Set(samples.map((s) => s.files).filter(Boolean))]
  rec.note(`files counter values seen: ${JSON.stringify(seen.slice(0, 12))}${seen.length > 12 ? ` ... (${seen.length})` : ''}`)
  rec.note(`pill texts seen: ${JSON.stringify([...new Set(samples.map((s) => s.pill).filter(Boolean))].slice(0, 6))}`)
  rec.check(seen.length >= 3, 'the files-done counter moves during a long Include all', `${seen.length} distinct values in ${includeMs} ms`)
  rec.check(worstGap < 2000, 'the app keeps answering during Include all (no read took over 2 s)', `${worstGap} ms`)
  const st = await rec.cli(repo, ['status', '--porcelain'])
  const staged = st.stdout.split('\n').filter((l) => /^A /.test(l)).length
  rec.check(staged === all.length, 'CLI agrees: every file is staged', `${staged} of ${all.length}`)

  let saveMs = 0
  await rec.step('Save version "Import village kit and sketches"', async () => {
    await msg(page).fill('Import village kit and sketches')
    const t = Date.now()
    await save(page).click()
    await waitCommits(rec, repo, 1)
    saveMs = Date.now() - t
    await page.waitForTimeout(1200)
  })
  rec.note(`PERF Save version of ${all.length} files: ${saveMs} ms`)
  const commits = await log(rec, repo)
  rec.check(commits.length === 1, 'one version was saved')
  const clean = await rec.cli(repo, ['status', '--porcelain'])
  rec.check(clean.stdout.trim() === '', 'the tree is clean after the save', clean.stdout.slice(0, 200))

  // --- touch many files outside the app
  await rec.step('Outside the app: edit 200 files at once', async () => {
    for (const f of all.filter((p) => /\.(svg|txt|json|md|obj|mtl|gltf)$/i.test(p)).slice(0, 200)) appendFileSync(f, '\n')
    await page.waitForTimeout(4000)
    await changesPage(page)
  })
  const c1 = await count(page).catch(() => 'no dock')
  const edited = (await rec.cli(repo, ['status', '--porcelain'], { quietFrame: true })).stdout.split('\n').filter(Boolean).length
  rec.check(c1.replace(/,/g, '').includes(`of ${edited}`), 'the watcher picks up a burst of edits within 4 s', `dock "${c1}", CLI ${edited}`)
  await rec.cli(repo, ['reset', '--hard', 'HEAD'], { quietFrame: true })
  await restoreCheck(rec, repo, want)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})
