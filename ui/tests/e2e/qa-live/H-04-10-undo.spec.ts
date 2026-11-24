import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo, refocus, toWelcome, visibleErrors } from './gui'
import { intCheck, sha256, walk } from './integrity'

// H-04..H-10 share one small repository of real edited pairs: 5 versions on main.
const repo = workDir('H-04')
interface Commit { oid: string; parents: string[]; summary: string }
const SUM = ['v1 first banner', 'v2 banner colours and a render', 'v3 new photo and notes', 'v4 drop render add clip', 'v5 final notes']

const flat = (s: string) => s.replace(/\n+/g, ' / ')
const list = (page: Page) => page.getByRole('listbox', { name: 'Versions, newest first' })
const store = <T>(page: Page, id: string, expr: string) =>
  page.evaluate(([i, e]) => {
    const app = (document.querySelector('#app') as unknown as { __vue_app__: { config: { globalProperties: { $pinia: { _s: Map<string, Record<string, unknown>> } } } } }).__vue_app__
    const s = app.config.globalProperties.$pinia._s.get(i!)
    return s ? (JSON.parse(JSON.stringify(new Function('s', `return ${e}`)(s) ?? null)) as unknown) : '(no store)'
  }, [id, expr] as const) as Promise<T>

async function log(rec: Recorder, ...args: string[]): Promise<Commit[]> {
  const r = await rec.cli(repo, ['log', '--machine', ...args], { quietFrame: true })
  try { return (JSON.parse(r.stdout) as { data: { commits: Commit[] } }).data.commits } catch { rec.note(`log gave no JSON: ${(r.stdout + r.stderr).slice(0, 200)}`); return [] }
}
const head = async (rec: Recorder) => (await log(rec))[0]?.oid ?? ''
const status = async (rec: Recorder) => flat((await rec.cli(repo, ['status', '--machine'], { quietFrame: true })).stdout).slice(0, 700)
const paths = (json: string) => [...new Set([...json.matchAll(/"(?:path|new_path|old_path)":\s*"([^"]+)"/g)].map((m) => m[1]!.replace(/\\\\/g, '/')))].sort()
async function tree(): Promise<Record<string, string>> {
  const out: Record<string, string> = {}
  for (const f of walk(repo)) out[f.slice(repo.length + 1).replace(/\\/g, '/')] = await sha256(f)
  return out
}
const sameTree = (a: Record<string, string>, b: Record<string, string>) => JSON.stringify(Object.entries(a).sort()) === JSON.stringify(Object.entries(b).sort())

async function build(rec: Recorder) {
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  const cli = (...a: string[]) => rec.cli(repo, a, { quietFrame: true })
  const put = (file: string, body: string | { from: string }) => {
    mkdirSync(dirname(join(repo, file)), { recursive: true })
    if (typeof body === 'string') writeFileSync(join(repo, file), body)
    else cpSync(join(CORPUS, body.from), join(repo, file))
  }
  const save = async (i: number) => {
    await cli('add', '--all')
    const r = await cli('commit', '-m', SUM[i]!, '--date', `2026-0${i + 1}-10T10:00:00Z`)
    if (r.code !== 0) rec.note(`setup commit failed: ${SUM[i]}: ${(r.stdout + r.stderr).slice(-200)}`)
  }
  await cli('init')
  put('art/banner.ai', { from: 'creative-pairs/ai/banner_v1.ai' }); put('art/photo.jpeg', { from: 'freepik__talk__71826.jpeg' }); put('notes.txt', 'line1\n'); await save(0)
  put('art/banner.ai', { from: 'creative-pairs/ai/banner_v2.ai' }); put('art/render.webp', { from: 'Workstation_cube_lid_off.webp' }); await save(1)
  put('art/photo.jpeg', { from: 'freepik__talk__72772.jpeg' }); put('notes.txt', 'line3\n'); await save(2)
  rmSync(join(repo, 'art/render.webp')); put('video/clip.mp4', { from: '101394-video-720.mp4' }); await save(3)
  put('notes.txt', 'final\n'); await save(4)
}

async function open(page: Page, route: string) {
  await toWelcome(page)
  await openRepo(page, repo)
  await go(page, route)
  await page.waitForTimeout(1500)
}
async function pick(page: Page, summary: string) {
  await page.getByPlaceholder('Search versions').fill(summary)
  await page.waitForTimeout(1000)
  await list(page).getByRole('option').first().click()
  await page.waitForTimeout(1200)
}
async function undoMenu(page: Page, item: RegExp) {
  await page.getByRole('button', { name: 'Undo this version' }).first().click()
  await page.waitForTimeout(400)
  await page.getByRole('menuitem', { name: item }).click()
  await page.waitForTimeout(700)
}
const dialogText = async (page: Page) => flat(await page.locator('[role="dialog"], [role="alertdialog"]').first().innerText().catch(() => '(no dialog)'))
async function choose(page: Page, label: string, option: RegExp) {
  await page.locator('.field').filter({ has: page.locator('label', { hasText: new RegExp(`^${label}$`) }) }).locator('.trigger').click()
  await page.waitForTimeout(400)
  await page.getByRole('option', { name: option }).first().click()
  await page.waitForTimeout(2500)
}
const STALE = ['MERGE_HEAD', 'MERGE_MSG']
/** BUG-020 leaves merge files behind; they break the app (BUG-021). Record, then remove so later steps can run. */
function clearStale(rec: Recorder, when: string): boolean {
  const left = STALE.filter((n) => existsSync(join(repo, '.mediagit', n)))
  if (left.length) rec.note(`stale after ${when}: ${left.join(', ')} (removed by the harness)`)
  for (const n of left) rmSync(join(repo, '.mediagit', n))
  return left.length === 0
}
const need = () => {
  if (!existsSync(join(repo, '.mediagit'))) throw new Error('needs the repository made by H-04')
  for (const n of STALE) rmSync(join(repo, '.mediagit', n), { force: true })
}
const entries = (out: string) => out.split('\n').filter((l) => l.includes('@{')).length

test('H-04 Compare two versions', async () => {
  const rec = await Recorder.start('H-04', 'compare-versions', { gui: true })
  const page = rec.page!
  await build(rec)
  const all = await log(rec)
  rec.check(all.length === 5, 'setup: 5 versions', String(all.length))
  const [v5, , , , v1] = all.map((c) => c.oid)
  await rec.step('Open History > Compare', () => open(page, 'history/compare'))
  rec.note(`compare on arrival: ${flat(await page.locator('#app main, #app').last().innerText()).slice(0, 400)}`)
  await rec.step('From = v1, To = v5', async () => {
    const from = page.locator('.field').filter({ has: page.locator('label', { hasText: /^From$/ }) }).locator('.trigger')
    rec.note(`"From" on arrival: "${await from.innerText()}" disabled=${await from.isDisabled()} (it only unlocks once "To" is a saved version; no hint says so)`)
    await choose(page, 'To', /v5 final notes/)
    await choose(page, 'From', /v1 first banner/)
  })
  const shown = await store<{ path: string; status: string }[]>(page, 'compare', 's.files')
  const d = await rec.cli(repo, ['diff', v1!, v5!, '--machine'])
  const want = paths(d.stdout)
  rec.note(`diff --machine: ${d.stdout.slice(0, 500)}`)
  rec.note(`app files: ${JSON.stringify(shown).slice(0, 500)}`)
  const got = Array.isArray(shown) ? shown.map((f) => f.path).sort() : []
  rec.check(got.join() === want.join() && got.length > 0, 'file list equals `diff v1 v5`', `app ${got.join(',')} | cli ${want.join(',')}`)
  rec.check(got.length === 4, 'four files differ (banner, photo, notes, clip); render came and went', got.join(','))
  const rows = flat(await page.getByLabel('Changed files').innerText().catch(() => ''))
  rec.note(`rows: ${rows.slice(0, 400)}`)

  await rec.step('Pick art/photo.jpeg', async () => { await page.getByLabel('Changed files').locator('[title="art/photo.jpeg"]').click(); await page.waitForTimeout(4000) })
  const pane = page.getByTestId('compare-pane')
  const imgs = () => pane.locator('img').evaluateAll((els) => (els as HTMLImageElement[]).map((i) => `${i.alt}:${i.naturalWidth}x${i.naturalHeight}`))
  let pics = await imgs()
  rec.note(`side by side images: ${JSON.stringify(pics)}`)
  rec.check(pics.length >= 2 && pics.every((p) => !p.endsWith(':0x0')), 'before and after pictures are both real', JSON.stringify(pics))
  const props = flat(await pane.getByRole('table').innerText().catch(() => '(no table)'))
  rec.note(`properties: ${props.slice(0, 600)}`)
  const sz = (f: string) => readFileSync(join(CORPUS, f)).length
  rec.note(`true sizes: before ${sz('freepik__talk__71826.jpeg')} B, after ${sz('freepik__talk__72772.jpeg')} B`)
  rec.check(/size/i.test(props), 'properties table lists the size', props.slice(0, 200))
  for (const tab of ['Onion', 'Swipe', 'Blend', 'Slide']) {
    const t = pane.getByRole('tab', { name: new RegExp(tab, 'i') })
    if (await t.count()) { await rec.step(`Comparison style: ${tab}`, async () => { await t.first().click(); await page.waitForTimeout(800) }) }
  }
  rec.note(`tabs: ${JSON.stringify(await pane.getByRole('tab').allInnerTexts())}`)
  const slider = pane.getByRole('slider').first()
  if (await slider.count()) await rec.step('Move the slider with the keyboard', async () => { await slider.focus(); for (let i = 0; i < 8; i++) await page.keyboard.press('ArrowRight'); await page.waitForTimeout(500) })

  await rec.step('Pick art/banner.ai (no picture preview)', async () => { await page.getByLabel('Changed files').locator('[title="art/banner.ai"]').click(); await page.waitForTimeout(3000) })
  rec.note(`banner.ai pane: ${flat(await pane.innerText()).slice(0, 500)}`)
  await rec.step('Pick notes.txt (text)', async () => { await page.getByLabel('Changed files').locator('[title="notes.txt"]').click(); await page.waitForTimeout(2000) })
  const notes = flat(await pane.innerText())
  rec.note(`notes.txt pane: ${notes.slice(0, 500)}`)
  rec.check(/line1|final/.test(notes), 'a text file shows its changed lines', notes.slice(0, 200))
  await rec.step('Pick video/clip.mp4 (added)', async () => { await page.getByLabel('Changed files').locator('[title="video/clip.mp4"]').click(); await page.waitForTimeout(3000) })
  rec.note(`clip.mp4 pane: ${flat(await pane.innerText()).slice(0, 500)}`)

  await rec.step('Filter files: "art"', async () => { await page.getByLabel('Filter files').fill('art'); await page.waitForTimeout(600) })
  const filtered = await page.getByLabel('Changed files').locator('[title]').evaluateAll((e) => e.map((x) => x.getAttribute('title')))
  rec.check(filtered.length === 2 && filtered.every((p) => p!.startsWith('art/')), 'the filter keeps only art/ files', JSON.stringify(filtered))
  await page.getByLabel('Filter files').fill('')
  await rec.step('Same version on both sides', async () => { await choose(page, 'To', /v1 first banner/) })
  const sameTxt = flat(await page.locator('#app').innerText())
  rec.check(/Pick two different versions|No differences/.test(sameTxt), 'same version twice gives a plain message', sameTxt.slice(0, 200))
  rec.check((await visibleErrors(page)).filter((e) => /fail|error|could not/i.test(e)).length === 0, 'no error panels', JSON.stringify(await visibleErrors(page)).slice(0, 300))
  await rec.finish()
  await closeTerm()
})

test('H-05 Compare staged versus working', async () => {
  const rec = await Recorder.start('H-05', 'compare-staged', { gui: true })
  const page = rec.page!
  need()
  // photo: staged change. banner: staged v3, then edited again to v4 (staged and working differ). notes: working only.
  cpSync(join(CORPUS, 'freepik__talk__71826.jpeg'), join(repo, 'art/photo.jpeg'))
  cpSync(join(CORPUS, 'creative-pairs/ai/banner_v3.ai'), join(repo, 'art/banner.ai'))
  await rec.cli(repo, ['add', 'art/photo.jpeg', 'art/banner.ai'])
  cpSync(join(CORPUS, 'creative-pairs/ai/banner_v4.ai'), join(repo, 'art/banner.ai'))
  writeFileSync(join(repo, 'notes.txt'), 'final\nworking only\n')
  const cached = await rec.cli(repo, ['diff', '--cached', '--machine'])
  const work = await rec.cli(repo, ['diff', '--machine'])
  const headWork = await rec.cli(repo, ['diff', 'HEAD', '--machine'])
  rec.note(`diff --cached: ${paths(cached.stdout).join(',')} | diff: ${paths(work.stdout).join(',')} | diff HEAD: ${paths(headWork.stdout).join(',')} (exit ${headWork.code})`)
  await rec.step('Open Compare (defaults)', () => open(page, 'history/compare'))
  rec.note(`defaults: from=${await store(page, 'compare', 's.from')} to=${await store(page, 'compare', 's.to')}`)
  await rec.step('From = v5, To = Working folder', async () => { await choose(page, 'To', /Ready to save/); await choose(page, 'To', /Working folder/) })
  let got = ((await store<{ path: string }[]>(page, 'compare', 's.files')) ?? []).map((f) => f.path).sort()
  rec.check(got.join() === 'art/banner.ai,art/photo.jpeg,notes.txt', 'working side lists all three changed files', got.join(','))
  await rec.step('To = Ready to save (staged)', async () => { await choose(page, 'To', /Ready to save/) })
  got = ((await store<{ path: string }[]>(page, 'compare', 's.files')) ?? []).map((f) => f.path).sort()
  rec.check(got.join() === paths(cached.stdout).join() && got.join() === 'art/banner.ai,art/photo.jpeg', 'staged side equals `diff --cached` (no notes.txt)', `app ${got.join(',')} | cli ${paths(cached.stdout).join(',')}`)
  await rec.step('Pick art/banner.ai on the staged side', async () => { await page.getByLabel('Changed files').locator('[title="art/banner.ai"]').click(); await page.waitForTimeout(3500) })
  const pane = flat(await page.getByTestId('compare-pane').innerText())
  const note = flat(await page.getByTestId('staged-note').innerText().catch(() => ''))
  rec.note(`staged banner pane: ${pane.slice(0, 600)} | staged-note: ${note}`)
  const v3 = readFileSync(join(CORPUS, 'creative-pairs/ai/banner_v3.ai')).length
  const v4 = readFileSync(join(CORPUS, 'creative-pairs/ai/banner_v4.ai')).length
  rec.note(`true sizes: staged (v3) ${v3} B, working (v4) ${v4} B`)
  const sides = await store<unknown>(page, 'compare', 's.sides')
  rec.note(`sides: ${JSON.stringify(sides).slice(0, 500)}`)
  rec.check(note.length > 0 || /staged|ready to save/i.test(pane), 'the staged side is labelled', (note || pane).slice(0, 200))
  await rec.step('Pick art/photo.jpeg on the staged side', async () => { await page.getByLabel('Changed files').locator('[title="art/photo.jpeg"]').click(); await page.waitForTimeout(4000) })
  const pics = await page.getByTestId('compare-pane').locator('img').evaluateAll((els) => (els as HTMLImageElement[]).map((i) => `${i.alt}:${i.naturalWidth}x${i.naturalHeight}`))
  rec.note(`staged photo images: ${JSON.stringify(pics)} | ${flat(await page.getByTestId('compare-pane').innerText()).slice(0, 400)}`)
  await rec.step('Edit notes.txt outside the app, then refocus', async () => { writeFileSync(join(repo, 'notes.txt'), 'final\n'); await choose(page, 'To', /Working folder/); await refocus(page); await page.waitForTimeout(3000) })
  got = ((await store<{ path: string }[]>(page, 'compare', 's.files')) ?? []).map((f) => f.path).sort()
  rec.check(!got.includes('notes.txt'), 'a file put back outside the app leaves the list after refocus', got.join(','))
  await rec.cli(repo, ['reset', '--hard', 'HEAD'])
  rec.note(`after cleanup: ${await status(rec)}`)
  await rec.finish()
  await closeTerm()
})

test('H-06 Tags', async () => {
  const rec = await Recorder.start('H-06', 'tags', { gui: true })
  const page = rec.page!
  need()
  const all = await log(rec)
  const v2 = all.find((c) => c.summary === SUM[1])!.oid
  const tagsCli = async () => { const r = await rec.cli(repo, ['tag', 'list', '--machine'], { quietFrame: true }); return [...r.stdout.matchAll(/"name":\s*"([^"]+)"/g)].map((m) => m[1]!).sort() }
  const tagsApp = () => page.getByLabel('Tags', { exact: true }).locator('li').allInnerTexts().then((t) => t.map(flat)).catch(() => [] as string[])
  const form = page.getByTestId('new-tag')
  const make = async (name: string, o: { note?: string; at?: string; replace?: boolean } = {}) => {
    if (!(await form.getByLabel('Tag name').isVisible().catch(() => false))) await form.locator('summary, button').first().click()
    await form.getByLabel('Tag name').fill(name)
    await form.getByLabel('Note (optional)').fill(o.note ?? '')
    const at = form.getByLabel('Version to tag')
    if (!(await at.isVisible().catch(() => false))) await form.getByText(/More options|Advanced|options/i).first().click().catch(() => {})
    await at.fill(o.at ?? '').catch(() => rec.note('"Version to tag" field not reachable'))
    if (o.replace) await form.getByLabel(/Replace the tag/).check().catch(() => form.getByText(/Replace the tag/).click())
    await page.waitForTimeout(400)
    await rec.shot(`form filled: ${name}`)
    const btn = form.locator('button[type="submit"]')
    const state = `${flat(await btn.innerText())} disabled=${await btn.isDisabled()}`
    if (!(await btn.isDisabled())) await btn.click()
    await page.waitForTimeout(1500)
    return state
  }
  await rec.step('Open History > Tags', () => open(page, 'history/tags'))
  rec.note(`empty state: ${flat(await page.locator('#app').innerText()).slice(-300)}`)
  await rec.step('Create plain tag v1.0', async () => { rec.note(`submit: ${await make('v1.0')}`) })
  await rec.step('Create annotated tag v1.1 with a note', async () => { rec.note(`submit: ${await make('v1.1', { note: 'Final delivery' })}`) })
  await rec.step('Create tag old-v2 on an older version', async () => { rec.note(`submit: ${await make('old-v2', { at: v2 })}`) })
  let cli = await tagsCli()
  rec.check(cli.join() === 'old-v2,v1.0,v1.1', 'three tags exist per `tag list`', cli.join(','))
  rec.note(`rows: ${JSON.stringify(await tagsApp())}`)
  rec.check((await tagsApp()).length === 3, 'the app lists three tags')
  const shw = await rec.cli(repo, ['tag', 'show', 'old-v2'])
  rec.check((shw.stdout + shw.stderr).includes(v2.slice(0, 7)), 'old-v2 points at v2', flat(shw.stdout).slice(0, 200))

  await rec.step('Duplicate name v1.0', async () => { rec.note(`submit: ${await make('v1.0')}`) })
  const dup = `${flat(await form.innerText()).slice(0, 400)} | alerts ${JSON.stringify(await visibleErrors(page))} | dialog ${await dialogText(page)}`
  rec.note(`duplicate: ${dup}`)
  rec.check(/taken|exists|already|in use/i.test(dup), 'a duplicate name is refused in plain words', dup.slice(0, 250))
  rec.check(!/os error|panicked|exit code|stderr|\bErr\(/i.test(dup), 'no raw error text for a duplicate')
  await page.keyboard.press('Escape')
  await rec.step('Bad names: "bad name", "..", "v1.0/"', async () => {
    for (const n of ['bad name', '..', 'a~b', 'CON']) { await form.getByLabel('Tag name').fill(n); await page.waitForTimeout(350); rec.note(`name "${n}": ${flat(await form.innerText()).match(/Tag name[^/]*\/[^/]*\/?[^/]*/)?.[0]} submit disabled=${await form.locator('button[type="submit"]').isDisabled()}`) }
  })
  await rec.step('Replace v1.0 on v2 (force)', async () => {
    rec.note(`submit: ${await make('v1.0', { at: v2, replace: true })}`)
    rec.note(`replace dialog: ${await dialogText(page)}`)
    await page.getByRole('button', { name: 'Replace tag' }).click().catch(() => rec.note('no Replace tag confirm shown'))
    await page.waitForTimeout(1500)
  })
  const s2 = await rec.cli(repo, ['tag', 'show', 'v1.0'])
  rec.check((s2.stdout + s2.stderr).includes(v2.slice(0, 7)), 'v1.0 now points at v2', flat(s2.stdout).slice(0, 200))
  await form.getByLabel(/Replace the tag/).uncheck().catch(() => {})

  await rec.step('Filter: v1.*', async () => { await page.getByLabel('Show only tags matching').fill('v1.*'); await page.waitForTimeout(1500) })
  const f = await tagsApp()
  const fc = await rec.cli(repo, ['tag', 'list', 'v1.*'])
  rec.check(f.length === 2 && f.every((t) => t.includes('v1.')), 'pattern v1.* shows two tags', `${JSON.stringify(f)} | cli: ${flat(fc.stdout).slice(0, 120)}`)
  await rec.step('Filter with no match', async () => { await page.getByLabel('Show only tags matching').fill('zzz*'); await page.waitForTimeout(1200) })
  rec.check(/No tag matches/.test(await page.locator('#app').innerText()), 'no match gives the "No tag matches" note')
  await page.getByLabel('Show only tags matching').fill('')
  await page.waitForTimeout(1200)

  await rec.step('Details of v1.1', async () => { await page.getByRole('button', { name: 'Actions for tag v1.1' }).click(); await page.getByRole('menuitem').first().click(); await page.waitForTimeout(1500) })
  const det = flat(await page.getByTestId('tag-detail').innerText().catch(() => '(none)'))
  rec.note(`detail: ${det}`)
  rec.check(/Annotated tag/.test(det) && /Final delivery/.test(det), 'detail shows annotated kind and the note', det.slice(0, 200))
  await rec.step('Check signature', async () => { await page.getByRole('button', { name: 'Check signature' }).click(); await page.waitForTimeout(1500) })
  const ver = flat(await page.getByTestId('tag-detail').innerText().catch(() => ''))
  const vc = await rec.cli(repo, ['tag', 'verify', 'v1.1'])
  rec.note(`verify in app: ${ver.slice(-200)} | cli exit ${vc.code}: ${flat(vc.stdout + vc.stderr).slice(0, 200)}`)
  rec.check(/not signed/i.test(ver), 'an unsigned tag says so plainly', ver.slice(-160))
  await page.getByRole('button', { name: 'Close' }).last().click()

  await rec.step('Delete old-v2: cancel first', async () => { await page.getByRole('button', { name: 'Actions for tag old-v2' }).click(); await page.getByRole('menuitem').last().click(); await page.waitForTimeout(600); rec.note(`delete dialog: ${await dialogText(page)}`); await page.getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(600) })
  rec.check((await tagsCli()).includes('old-v2'), 'cancel keeps the tag')
  await rec.step('Delete old-v2: confirm', async () => { await page.getByRole('button', { name: 'Actions for tag old-v2' }).click(); await page.getByRole('menuitem').last().click(); await page.waitForTimeout(600); await page.getByRole('button', { name: 'Delete tag' }).click(); await page.waitForTimeout(1500) })
  cli = await tagsCli()
  rec.check(cli.join() === 'v1.0,v1.1', 'old-v2 is gone', cli.join(','))

  await rec.step('History: "Tag this version" on v3', async () => {
    await go(page, 'history/versions'); await list(page).getByRole('option').first().waitFor(); await pick(page, SUM[2]!)
    await page.getByRole('button', { name: 'Tag this version' }).first().click(); await page.waitForTimeout(500)
    await page.getByRole('dialog').getByLabel('Tag name').fill('look-approved'); await page.waitForTimeout(300)
    await page.getByRole('dialog').getByRole('button', { name: 'Tag this version' }).click(); await page.waitForTimeout(2000)
  })
  const v3 = all.find((c) => c.summary === SUM[2])!.oid
  const s3 = await rec.cli(repo, ['tag', 'show', 'look-approved'])
  rec.check((s3.stdout + s3.stderr).includes(v3.slice(0, 7)), 'the tag made from History points at v3', flat(s3.stdout + s3.stderr).slice(0, 200))
  const pills = flat(await page.getByLabel('Names pointing at this version').innerText().catch(() => ''))
  rec.check(pills.includes('look-approved'), 'the version shows its new tag pill at once', pills)
  await page.getByPlaceholder('Search versions').fill('')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('H-07 Revert a version', async () => {
  const rec = await Recorder.start('H-07', 'revert', { gui: true })
  const page = rec.page!
  need()
  await rec.cli(repo, ['reset', '--hard', 'HEAD'], { quietFrame: true })
  const base = await head(rec)
  const before = await tree()
  const toHistory = async () => { await go(page, 'history/versions'); await list(page).getByRole('option').first().waitFor({ timeout: 30_000 }) }
  await rec.step('Open History, pick v4, Undo menu', async () => { await open(page, 'history/versions'); await pick(page, SUM[3]!); await undoMenu(page, /adds a new one/) })
  rec.note(`revert dialog: ${await dialogText(page)}`)
  await rec.step('Cancel the dialog', async () => { await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(700) })
  rec.check((await head(rec)) === base, 'cancel makes no version')
  await rec.step('Undo v4 for real (clean revert)', async () => { await undoMenu(page, /adds a new one/); await page.getByRole('dialog').getByRole('button', { name: /^Undo this version/ }).click(); await page.waitForTimeout(5000) })
  let now = await log(rec)
  rec.note(`after revert: top = "${now[0]?.summary}" | errors ${JSON.stringify(await visibleErrors(page))}`)
  rec.check(now.length === 6 && now[0]!.parents[0] === base, 'one new version on top of the old tip', `${now.length} versions`)
  rec.check(existsSync(join(repo, 'art/render.webp')) && !existsSync(join(repo, 'video/clip.mp4')), 'the files are back as before v4 (render back, clip gone)')
  rec.check((await sha256(join(repo, 'art/render.webp'))) === (await sha256(join(CORPUS, 'Workstation_cube_lid_off.webp'))), 'render.webp has its original bytes')
  await page.getByPlaceholder('Search versions').fill(''); await page.waitForTimeout(1200)
  const top = flat(await list(page).getByRole('option').first().innerText())
  rec.check(top.includes(now[0]!.summary.slice(0, 12)), 'the list shows the new version at the top without a manual refresh', top)
  await rec.cli(repo, ['reset', '--hard', base], { quietFrame: true })

  await rec.step('Undo v3 while v5 changed the same file (conflict)', async () => { await refocus(page); await toHistory(); await pick(page, SUM[2]!); await undoMenu(page, /adds a new one/); await page.getByRole('dialog').getByRole('button', { name: /^Undo this version/ }).click(); await page.waitForTimeout(5000) })
  let banner = flat(await page.getByTestId('revert-stopped').innerText().catch(() => ''))
  rec.note(`stopped banner: ${banner} | alerts ${JSON.stringify(await visibleErrors(page)).slice(0, 400)} | status ${await status(rec)}`)
  rec.check(banner.length > 0, 'a conflicting undo stops with a banner and choices', JSON.stringify(await visibleErrors(page)).slice(0, 200))
  rec.check(!/os error|panicked|exit code/i.test(banner), 'banner is in plain words')
  await rec.step('Abandon undo', async () => { await page.getByRole('button', { name: 'Abandon undo' }).click().catch(() => rec.note('no Abandon button')); await page.waitForTimeout(4000) })
  rec.check((await head(rec)) === base, 'abandon leaves the branch where it was')
  const afterAbort = await tree()
  rec.check(sameTree(before, afterAbort), 'abandon restores every file byte for byte', Object.keys(afterAbort).filter((k) => before[k] !== afterAbort[k]).concat(Object.keys(before).filter((k) => !(k in afterAbort))).join(','))
  rec.note(`status after abandon: ${await status(rec)}`)
  rec.check(clearStale(rec, 'abandon'), 'abandon leaves no operation in progress (BUG-020)')

  await rec.step('Again, then fix notes.txt outside and Continue undo', async () => {
    await pick(page, SUM[2]!); await undoMenu(page, /adds a new one/); await page.getByRole('dialog').getByRole('button', { name: /^Undo this version/ }).click(); await page.waitForTimeout(5000)
    rec.note(`notes.txt during conflict: ${JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8').slice(0, 200))}`)
    writeFileSync(join(repo, 'notes.txt'), 'resolved\n')
    const a = await rec.cli(repo, ['add', 'notes.txt'])
    rec.note(`add resolved: exit ${a.code} ${flat(a.stdout + a.stderr).slice(0, 200)}`)
    await refocus(page); await page.waitForTimeout(1500)
    await page.getByRole('button', { name: 'Continue undo' }).click().catch(() => rec.note('no Continue button'))
    await page.waitForTimeout(5000)
  })
  now = await log(rec)
  banner = flat(await page.getByTestId('revert-stopped').innerText().catch(() => ''))
  rec.note(`after continue: top "${now[0]?.summary}", ${now.length} versions, banner "${banner}", alerts ${JSON.stringify(await visibleErrors(page)).slice(0, 300)}`)
  rec.check(now.length === 6 && now[0]!.parents[0] === base, 'continue makes the undo version')
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'resolved\n' && (await sha256(join(repo, 'art/photo.jpeg'))) === (await sha256(join(CORPUS, 'freepik__talk__71826.jpeg'))), 'result: my fixed notes and the photo from before v3')
  rec.check(banner === '', 'the stopped banner is gone')
  rec.check(clearStale(rec, 'continue'), 'continue leaves no operation in progress (BUG-020)')
  await rec.cli(repo, ['reset', '--hard', base], { quietFrame: true })

  await rec.step('Undo v5 with "Let me look before it is saved"', async () => {
    await refocus(page); await toHistory(); await pick(page, SUM[4]!); await undoMenu(page, /adds a new one/)
    await page.getByRole('dialog').getByText('More options').click(); await page.waitForTimeout(300)
    await page.getByRole('dialog').getByText('Let me look before it is saved').click(); await page.waitForTimeout(300)
    await rec.shot('no-commit ticked')
    await page.getByRole('dialog').getByRole('button', { name: /^Undo this version/ }).click(); await page.waitForTimeout(4000)
  })
  rec.check((await head(rec)) === base, '--no-commit makes no version')
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'line3\n', 'but the file is changed back', JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8')))
  rec.note(`status after --no-commit: ${await status(rec)} | app says: ${JSON.stringify(await visibleErrors(page)).slice(0, 300)}`)
  await rec.step('Look at Changes after the no-save undo', async () => { await go(page, 'files/changes'); await page.waitForTimeout(2500) })
  const ch = flat(await page.locator('#app').innerText())
  rec.check(ch.includes('notes.txt'), 'Changes shows notes.txt waiting', ch.slice(0, 300))
  const ab = await rec.cli(repo, ['revert', '--abort'])
  rec.note(`revert --abort after --no-commit: exit ${ab.code} ${flat(ab.stdout + ab.stderr).slice(0, 200)}`)
  await rec.cli(repo, ['reset', '--hard', base], { quietFrame: true })
  rec.check(sameTree(before, await tree()), 'cleanup: tree equals the start')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('H-08 Reset soft, mixed, hard', async () => {
  const rec = await Recorder.start('H-08', 'reset', { gui: true })
  const page = rec.page!
  need()
  await rec.cli(repo, ['reset', '--hard', 'HEAD'], { quietFrame: true })
  const all = await log(rec)
  const base = all[0]!.oid
  const v4 = all.find((c) => c.summary === SUM[3])!.oid
  const v3 = all.find((c) => c.summary === SUM[2])!.oid
  const back = async () => { rmSync(join(repo, 'video/clip.mp4'), { force: true }); await rec.cli(repo, ['reset', '--hard', base], { quietFrame: true }); await refocus(page); await go(page, 'history/versions'); await page.getByPlaceholder('Search versions').fill(''); await page.waitForTimeout(1500) }
  await rec.step('Open History, pick v4, "Move branch back here, keep changes ready to save"', async () => { await open(page, 'history/versions'); await pick(page, SUM[3]!); await undoMenu(page, /ready to save/) })
  rec.note(`soft dialog: ${await dialogText(page)}`)
  await rec.step('Cancel once', async () => { await page.getByRole('alertdialog').getByRole('button', { name: /Cancel|Keep/ }).click(); await page.waitForTimeout(700) })
  rec.check((await head(rec)) === base, 'cancel does nothing')
  await rec.step('Soft: confirm', async () => { await undoMenu(page, /ready to save/); await page.getByRole('button', { name: 'Move branch back' }).click(); await page.waitForTimeout(4000) })
  rec.check((await head(rec)) === v4, 'soft: branch is at v4')
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'final\n', 'soft: files untouched')
  let st = await status(rec)
  rec.note(`status after soft: ${st}`)
  const cached = paths((await rec.cli(repo, ['diff', '--cached', '--machine'], { quietFrame: true })).stdout)
  rec.check(cached.join() === 'notes.txt', 'soft: only notes.txt is staged (v5 change)', cached.join(','))
  await rec.step('Changes after soft', async () => { await go(page, 'files/changes'); await page.waitForTimeout(2500) })
  const chs = await store<unknown>(page, 'changes', '({counts: s.counts, staged: (s.staged||[]).map(f=>f.path), unstaged: (s.unstaged||[]).map(f=>f.path)})')
  rec.note(`changes store after soft: ${JSON.stringify(chs)}`)
  const chText = flat(await page.locator('#app').innerText())
  rec.check(JSON.stringify(chs).includes('"total":1') && chText.includes('notes.txt') && !/banner\.ai|photo\.jpeg|clip\.mp4/.test(chText), 'Changes shows only notes.txt (no unchanged files as staged)', `${JSON.stringify(chs).slice(0, 120)} | ${chText.slice(-250)}`)
  const top = (await log(rec))[0]!
  await go(page, 'history/versions'); await page.waitForTimeout(1500)
  rec.check(flat(await list(page).getByRole('option').first().innerText()).includes(top.summary), 'History top row follows the reset')
  await back()

  await rec.step('Mixed: v3, "keep changes in my files"', async () => { await pick(page, SUM[2]!); await undoMenu(page, /in my files/); rec.note(`mixed dialog: ${await dialogText(page)}`); await page.getByRole('button', { name: 'Move branch back' }).click(); await page.waitForTimeout(4000) })
  rec.check((await head(rec)) === v3, 'mixed: branch is at v3')
  rec.check(existsSync(join(repo, 'video/clip.mp4')) && readFileSync(join(repo, 'notes.txt'), 'utf8') === 'final\n', 'mixed: files untouched')
  st = await status(rec)
  rec.note(`status after mixed: ${st}`)
  const c2 = paths((await rec.cli(repo, ['diff', '--cached', '--machine'], { quietFrame: true })).stdout)
  rec.check(c2.length === 0, 'mixed: nothing staged', c2.join(','))
  await back()

  writeFileSync(join(repo, 'notes.txt'), 'unsaved work\n')
  writeFileSync(join(repo, 'new-idea.txt'), 'new file, never saved\n')
  await rec.step('Hard: v4, "Discard everything back to here…" with unsaved work', async () => { await refocus(page); await page.waitForTimeout(1500); await pick(page, SUM[3]!); await undoMenu(page, /Discard everything/) })
  const hd = await dialogText(page)
  rec.note(`hard dialog: ${hd}`)
  rec.check(/1 file with unsaved changes|2 files with unsaved changes/.test(hd), 'hard dialog counts my unsaved files', hd.slice(0, 300))
  const go2 = page.getByRole('button', { name: 'Discard and go back' })
  rec.check(await go2.isDisabled(), 'the danger button is off until the box is ticked')
  await rec.step('Keep my files', async () => { await page.getByRole('button', { name: 'Keep my files' }).click(); await page.waitForTimeout(700) })
  rec.check((await head(rec)) === base && readFileSync(join(repo, 'notes.txt'), 'utf8') === 'unsaved work\n', '"Keep my files" does nothing')
  await rec.step('Hard: tick and confirm', async () => { await undoMenu(page, /Discard everything/); await page.getByRole('dialog').getByText('I understand my unsaved work will be lost').click(); await page.waitForTimeout(300); await rec.shot('ticked'); await go2.click(); await page.waitForTimeout(5000) })
  rec.check((await head(rec)) === v4, 'hard: branch is at v4')
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'line3\n', 'hard: notes.txt is the v4 content', JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8')))
  rec.note(`hard: untracked new-idea.txt ${existsSync(join(repo, 'new-idea.txt')) ? 'KEPT' : 'DELETED'} (dialog said: ${hd.match(/[^/]*unsaved[^/]*/)?.[0]})`)
  rec.note(`status after hard: ${await status(rec)} | alerts ${JSON.stringify(await visibleErrors(page)).slice(0, 300)}`)
  rmSync(join(repo, 'new-idea.txt'), { force: true })
  await intCheck(rec, repo)
  // left at v4 on purpose: H-09 recovers v5 through the undo log
  await rec.finish()
  await closeTerm()
})

test('H-09 Undo log (reflog) recovers the work', async () => {
  const rec = await Recorder.start('H-09', 'undo-log', { gui: true })
  const page = rec.page!
  need()
  const rl = await rec.cli(repo, ['reflog'])
  const rlm = await rec.cli(repo, ['reflog', '--machine'], { quietFrame: true })
  rec.note(`reflog --machine: ${rlm.stdout.slice(0, 600)}`)
  const lines = rl.stdout.split('\n').filter((l) => l.includes('@{'))
  rec.check(lines.length >= 8, 'reflog has an entry for each move made in H-07 and H-08', `${lines.length} lines`)
  const all = await rec.cli(repo, ['reflog', 'show', '--all'])
  rec.check(all.code === 0 && all.stdout.includes('main'), 'reflog show --all works', `exit ${all.code}`)
  const v5 = [...rlm.stdout.matchAll(/"(?:new_oid|new|oid)":\s*"([0-9a-f]{40,64})"/g)].map((m) => m[1]!)
  await rec.step('Open Settings > Advanced tools', () => open(page, 'settings/advanced'))
  rec.note(`advanced: ${flat(await page.locator('#app').innerText()).slice(0, 500)}`)
  await rec.step('Choose the undo log tool', async () => {
    const t = page.getByRole('tab', { name: /undo log|reflog/i }).or(page.getByRole('radio', { name: /undo log|reflog/i })).or(page.getByRole('button', { name: /undo log|reflog/i }))
    if (await t.count()) await t.first().click()
    else { await page.getByRole('combobox').first().click(); await page.getByRole('option', { name: /undo log|reflog/i }).click() }
    await page.waitForTimeout(2000)
  })
  const rows = await page.getByLabel('Reflog, newest first').locator('li').allInnerTexts().catch(() => [] as string[])
  rec.note(`panel rows (${rows.length}): ${JSON.stringify(rows.slice(0, 6).map(flat))}`)
  rec.check(rows.length === lines.length || rows.length >= 8, 'the panel lists the same moves as `reflog`', `${rows.length} vs ${lines.length}`)
  const target = rows.findIndex((r) => /v5 final notes|reset/i.test(r) && !/v4/.test(r))
  rec.note(`looking for the row before the hard reset; first candidate index ${target}; known oids ${v5.slice(0, 3).map((o) => o.slice(0, 7)).join(',')}`)
  await rec.step('Restore to @1 (the state before the hard reset)', async () => { await page.getByRole('button', { name: 'Restore to @1', exact: true }).click(); await page.waitForTimeout(800); rec.note(`restore dialog: ${await dialogText(page)}`); await rec.shot('restore dialog') })
  await rec.step('Confirm the restore', async () => {
    const dlg = page.locator('[role="dialog"], [role="alertdialog"]').first()
    const tick = dlg.getByRole('checkbox'); if (await tick.count()) await tick.first().check().catch(() => dlg.locator('label').first().click())
    await dlg.getByRole('button').last().click(); await page.waitForTimeout(5000)
  })
  const now = await log(rec)
  rec.note(`after restore: top "${now[0]?.summary}" (${now.length} versions), notes.txt ${JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8'))}, alerts ${JSON.stringify(await visibleErrors(page)).slice(0, 300)}`)
  rec.check(now[0]?.summary === SUM[4] && now.length === 5, 'the undo log brings v5 back as the tip')
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'line3\n', 'the undo log restore is a soft move: files are not touched, as its dialog says', JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8')))
  await rec.cli(repo, ['reset', '--hard', 'HEAD'])
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'final\n', 'after a discard the files of v5 are back')
  await rec.step('Look at: every branch and tag', async () => { await page.getByLabel('Look at').click(); await page.waitForTimeout(400); rec.note(`"Look at" options: ${JSON.stringify(await page.getByRole('option').allInnerTexts())}`); await page.getByRole('option').last().click(); await page.waitForTimeout(1500) })
  rec.note(`after picking the last option: ${flat(await page.locator('#app').innerText()).slice(0, 400)}`)
  const before = entries((await rec.cli(repo, ['reflog'], { quietFrame: true })).stdout)
  const ex = await rec.cli(repo, ['reflog', 'expire', '--keep', '3'])
  const after = entries((await rec.cli(repo, ['reflog'], { quietFrame: true })).stdout)
  rec.note(`reflog expire --keep 3: exit ${ex.code} ${flat(ex.stdout + ex.stderr).slice(0, 200)}; ${before} to ${after} lines`)
  rec.check(ex.code === 0 && after === 3, 'CLI `reflog expire --keep 3` leaves three entries', `${after}`)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('H-10 Copy a version to another branch from History', async () => {
  const rec = await Recorder.start('H-10', 'copy-to-branch', { gui: true })
  const page = rec.page!
  need()
  await rec.step('Open History and pick v3', async () => { await open(page, 'history/versions'); await pick(page, SUM[2]!) })
  const acts = await page.getByRole('group', { name: 'This version' }).getByRole('button').allInnerTexts()
  rec.note(`actions on a version: ${JSON.stringify(acts)}`)
  await rec.step('Open the Undo menu to list its entries', async () => { await page.getByRole('button', { name: 'Undo this version' }).first().click(); await page.waitForTimeout(500) })
  const items = await page.getByRole('menuitem').allInnerTexts()
  rec.note(`undo menu: ${JSON.stringify(items)}`)
  await page.keyboard.press('Escape')
  await rec.step('Right-click the version row', async () => { await list(page).getByRole('option').first().click({ button: 'right' }); await page.waitForTimeout(600) })
  const ctx = await page.getByRole('menuitem').allInnerTexts()
  rec.note(`row context menu: ${JSON.stringify(ctx)}`)
  await page.keyboard.press('Escape')
  rec.check([...acts, ...items, ...ctx].some((t) => /copy|cherry/i.test(t)), 'History offers "copy to another branch" for a version', JSON.stringify([...acts, ...items, ...ctx]))
  await rec.step('Advanced tools: Copy a version', async () => { await go(page, 'settings/advanced'); await page.waitForTimeout(1200) })
  rec.note(`advanced tools: ${flat(await page.locator('#app').innerText()).slice(0, 400)}`)
  await rec.finish()
  await closeTerm()
})
