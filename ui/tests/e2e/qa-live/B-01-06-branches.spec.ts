import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { choose, dialogText, flat, go, openRepo, refocus, rowMenu, store, toWelcome, visibleErrors } from './gui'
import { intCheck, sha256, walk } from './integrity'

// B-01..B-06 share one repository. Each case resets it to a known state through the CLI first.
const repo = workDir('B-01')
interface Commit { oid: string; parents: string[]; summary: string }
interface Branch { name: string; oid: string; protected: boolean }
const P1 = 'freepik__talk__71826.jpeg'
const P2 = 'freepik__talk__72772.jpeg'
const P3 = 'Workstation_cube_lid_off.webp'
const banner = (n: number) => `creative-pairs/ai/banner_v${n}.ai`
const STATE = ['MERGE_HEAD', 'MERGE_MSG', 'REVERT_STATE', 'CHERRY_PICK_STATE', 'rebase-apply']

const cli = (rec: Recorder, ...a: string[]) => rec.cli(repo, a, { quietFrame: true })
async function log(rec: Recorder, ...args: string[]): Promise<Commit[]> {
  const r = await cli(rec, 'log', '--machine', ...args)
  try { return (JSON.parse(r.stdout) as { data: { commits: Commit[] } }).data.commits } catch { rec.note(`log gave no JSON: ${(r.stdout + r.stderr).slice(0, 200)}`); return [] }
}
async function branches(rec: Recorder): Promise<{ current: string; list: Branch[] }> {
  const r = await cli(rec, 'branch', 'list', '--machine')
  try { const d = (JSON.parse(r.stdout) as { data: { current: string; branches: Branch[] } }).data; return { current: d.current, list: d.branches } } catch { rec.note(`branch list gave no JSON: ${(r.stdout + r.stderr).slice(0, 300)}`); return { current: '', list: [] } }
}
const names = async (rec: Recorder) => (await branches(rec)).list.map((b) => b.name).sort().join(',')
const tip = async (rec: Recorder, rev = 'HEAD') => (await log(rec, rev))[0]
const statusOut = async (rec: Recorder) => { const r = await cli(rec, 'status', '--machine'); return { lines: r.stdout.split('\n').filter((l) => l.trim()).length, text: flat(r.stdout).slice(0, 600) } }
const left = () => STATE.filter((n) => existsSync(join(repo, '.mediagit', n)))
const put = (file: string, body: string | { from: string }) => {
  mkdirSync(dirname(join(repo, file)), { recursive: true })
  if (typeof body === 'string') writeFileSync(join(repo, file), body)
  else cpSync(join(CORPUS, body.from), join(repo, file))
}
const hash = (file: string) => sha256(join(repo, file))
const corpus = (file: string) => sha256(join(CORPUS, file))
async function tree(): Promise<Record<string, string>> {
  const out: Record<string, string> = {}
  for (const f of walk(repo)) out[f.slice(repo.length + 1).replace(/\\/g, '/')] = await sha256(f)
  return out
}
const diffTree = (a: Record<string, string>, b: Record<string, string>) => [...new Set([...Object.keys(a), ...Object.keys(b)])].filter((k) => a[k] !== b[k])
async function save(rec: Recorder, message: string) {
  await cli(rec, 'add', '--all')
  const r = await cli(rec, 'commit', '-m', message)
  if (r.code !== 0) rec.note(`setup commit failed: ${message}: ${flat(r.stdout + r.stderr).slice(-200)}`)
}
const need = () => { if (!existsSync(join(repo, '.mediagit'))) throw new Error('needs the repository made by B-01') }
async function open(page: Page, route: string) {
  await toWelcome(page)
  await openRepo(page, repo)
  await go(page, route)
  await page.waitForTimeout(1500)
}
const rows = (page: Page) => page.getByLabel('Branches', { exact: true }).locator('li').allInnerTexts().then((t) => t.map(flat))
const alerts = async (page: Page) => JSON.stringify(await visibleErrors(page)).slice(0, 500)
const raw = (s: string) => /os error|panicked|exit code|stderr|\bErr\(|protocol|unexpected output/i.test(s)
/** Remove every branch but main and put main back on `oid`. */
async function clean(rec: Recorder, oid: string) {
  for (const n of STATE) rmSync(join(repo, '.mediagit', n), { recursive: true, force: true })
  await cli(rec, 'branch', 'switch', 'main', '--force')
  await cli(rec, 'reset', '--hard', oid)
  for (const b of (await branches(rec)).list) if (b.name !== 'main') await cli(rec, 'branch', 'delete', b.name, '--force')
  for (const f of walk(repo)) if (!/art[\\/](banner\.ai|photo\.jpeg)$|notes\.txt$/.test(f)) rmSync(f, { force: true })
}
let V1 = ''
let V2 = ''
const ids = async (rec: Recorder) => { const all = await log(rec, 'main'); V2 = all.find((c) => c.summary === 'v2 banner colours')!.oid; V1 = all.find((c) => c.summary === 'v1 first banner')!.oid }

test('B-01 Create, switch, rename, details, protect', async () => {
  const rec = await Recorder.start('B-01', 'branch-basics', { gui: true })
  const page = rec.page!
  rmSync(repo, { recursive: true, force: true }); mkdirSync(repo, { recursive: true })
  await cli(rec, 'init')
  put('art/banner.ai', { from: banner(1) }); put('art/photo.jpeg', { from: P1 }); put('notes.txt', 'base\n'); await save(rec, 'v1 first banner')
  put('art/banner.ai', { from: banner(2) }); await save(rec, 'v2 banner colours')
  await ids(rec)
  const dlg = page.getByRole('dialog')
  await rec.step('Open Branches', () => open(page, 'branches/branches'))
  rec.note(`rows: ${JSON.stringify(await rows(page))}`)
  await rec.step('New branch: bad names', async () => {
    await page.getByRole('button', { name: 'New branch' }).click(); await page.waitForTimeout(500)
    for (const n of ['has space', 'main', '..', 'a~b', '-lead', 'x/', 'CON', 'grade.lock']) { await dlg.getByLabel('Branch name').fill(n); await page.waitForTimeout(300); rec.note(`name "${n}": ${flat(await dlg.innerText()).match(/Branch name \/ [^/]*/)?.[0]} | create disabled=${await dlg.getByRole('button', { name: 'Create and switch' }).isDisabled()}`) }
  })
  await rec.step('Create and switch: grade-pass', async () => { await dlg.getByLabel('Branch name').fill('grade-pass'); await page.waitForTimeout(300); await dlg.getByRole('button', { name: 'Create and switch' }).click(); await page.waitForTimeout(2500) })
  let b = await branches(rec)
  rec.check(b.current === 'grade-pass' && b.list.find((x) => x.name === 'grade-pass')?.oid === V2, 'grade-pass exists at the tip and is current', `${b.current} ${await names(rec)}`)
  await rec.step('Create (no switch) from an older version: from-v1', async () => {
    await page.getByRole('button', { name: 'New branch' }).click(); await page.waitForTimeout(500)
    await dlg.getByLabel('Branch name').fill('from-v1')
    if (!(await dlg.getByLabel('Start from').isVisible().catch(() => false))) await dlg.getByText('More options').click()
    await dlg.getByLabel('Start from').fill(V1); await page.waitForTimeout(400)
    await rec.shot('start point filled')
    await dlg.getByRole('button', { name: 'Create', exact: true }).click(); await page.waitForTimeout(2500)
  })
  b = await branches(rec)
  rec.check(b.current === 'grade-pass' && b.list.find((x) => x.name === 'from-v1')?.oid === V1, 'from-v1 points at v1 and the current branch did not move', `${b.current} ${b.list.find((x) => x.name === 'from-v1')?.oid.slice(0, 7)}`)
  await rec.step('New branch from a start point that does not exist', async () => {
    await page.getByRole('button', { name: 'New branch' }).click(); await page.waitForTimeout(500)
    await dlg.getByLabel('Branch name').fill('ghost')
    if (!(await dlg.getByLabel('Start from').isVisible().catch(() => false))) await dlg.getByText('More options').click()
    await dlg.getByLabel('Start from').fill('no-such-thing'); await page.waitForTimeout(400)
    const btn = dlg.getByRole('button', { name: 'Create', exact: true })
    if (!(await btn.isDisabled())) { await btn.click(); await page.waitForTimeout(2000) }
  })
  const ghost = `${await dialogText(page)} | ${await alerts(page)}`
  rec.note(`bad start point: ${ghost}`)
  rec.check(!(await names(rec)).includes('ghost'), 'no branch is made from a bad start point')
  rec.check(!raw(ghost), 'bad start point is explained in plain words', ghost.slice(0, 300))
  await page.keyboard.press('Escape'); await page.waitForTimeout(400)
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})

  await rec.step('Switch to from-v1', async () => { await page.getByLabel('Branches', { exact: true }).locator('li', { hasText: 'from-v1' }).getByRole('button', { name: 'Switch to' }).click(); await page.waitForTimeout(4000) })
  rec.check((await branches(rec)).current === 'from-v1', 'the current branch is from-v1')
  rec.check((await hash('art/banner.ai')) === (await corpus(banner(1))), 'banner.ai on disk is the v1 file after the switch')
  rec.note(`rows: ${JSON.stringify(await rows(page))} | rail: ${flat(await page.locator('#app').innerText()).slice(0, 60)}`)
  await rec.step('Rename from-v1 to old-look', async () => { await rowMenu(page, 'from-v1', /Rename/); await dlg.getByLabel('New name').fill('old-look'); await page.waitForTimeout(300); await dlg.getByRole('button', { name: 'Rename' }).click(); await page.waitForTimeout(2500) })
  b = await branches(rec)
  rec.check(b.current === 'old-look' && !b.list.some((x) => x.name === 'from-v1'), 'rename of the current branch works and it stays current', `${b.current} ${await names(rec)}`)
  await rec.step('Rename old-look to a taken name (grade-pass)', async () => { await rowMenu(page, 'old-look', /Rename/); await dlg.getByLabel('New name').fill('grade-pass'); await page.waitForTimeout(400) })
  rec.note(`taken name: ${await dialogText(page)} | rename disabled=${await dlg.getByRole('button', { name: 'Rename' }).isDisabled()}`)
  await page.keyboard.press('Escape'); await page.waitForTimeout(400)
  await rec.step('Details of old-look', async () => { await rowMenu(page, 'old-look', /Details/); await page.waitForTimeout(1200) })
  const det = flat(await page.getByTestId('branch-detail').innerText().catch(() => '(none)'))
  const show = await rec.cli(repo, ['branch', 'show', 'old-look'])
  rec.note(`detail: ${det} | cli show: ${flat(show.stdout + show.stderr).slice(0, 300)}`)
  rec.check(det.includes(V1.slice(0, 7)), 'details show the version it points at', det.slice(0, 200))
  await page.getByRole('button', { name: 'Close' }).last().click()
  await rec.step('Protect main', async () => { await rowMenu(page, 'main', /Protect branch/); rec.note(`protect dialog: ${await dialogText(page)}`); await dlg.getByRole('button', { name: 'Protect branch' }).click(); await page.waitForTimeout(2000) })
  rec.check((await branches(rec)).list.find((x) => x.name === 'main')?.protected === true, 'main is protected per `branch list`', await alerts(page))
  rec.note(`rows: ${JSON.stringify(await rows(page))}`)
  await rec.step('Open the menu of protected main', async () => { await page.getByRole('button', { name: 'Actions for main', exact: true }).click(); await page.waitForTimeout(400) })
  const items = await page.getByRole('menuitem').evaluateAll((els) => els.map((e) => `${(e as HTMLElement).innerText}${e.getAttribute('aria-disabled') === 'true' || e.hasAttribute('data-disabled') ? ' [off]' : ''}`))
  rec.note(`menu of protected main: ${JSON.stringify(items)}`)
  rec.check(items.some((i) => /Delete \(protected\).*\[off\]/.test(i)), 'Delete is off for a protected branch', JSON.stringify(items))
  await page.keyboard.press('Escape')
  const del = await rec.cli(repo, ['branch', 'delete', 'main', '--force'])
  rec.check(del.code !== 0 && (await names(rec)).includes('main'), 'CLI also refuses to delete protected main, even with --force', flat(del.stdout + del.stderr).slice(0, 200))
  if (!(await names(rec)).includes('main')) await rec.cli(repo, ['branch', 'create', 'main', 'grade-pass']) // restore so B-02..B-06 can run (the deletion is already recorded as a failed check)
  const rs = await rec.cli(repo, ['branch', 'switch', 'main'])
  const hard = await rec.cli(repo, ['reset', '--hard', V1])
  rec.note(`reset --hard on protected main to v1: exit ${hard.code} ${flat(hard.stdout + hard.stderr).slice(0, 200)} (switch exit ${rs.code})`)
  await cli(rec, 'reset', '--hard', V2)
  await rec.step('Remove protection from main', async () => { await refocus(page); await page.waitForTimeout(1500); await rowMenu(page, 'main', /Remove protection/); rec.note(`unprotect dialog: ${await dialogText(page)}`); await page.getByRole('button', { name: 'Remove protection' }).click(); await page.waitForTimeout(2000) })
  rec.check((await branches(rec)).list.find((x) => x.name === 'main')?.protected === false, 'protection is removed')
  const app = (await rows(page)).map((r) => r.split(' / ')[0]!.trim()).sort().join(',')
  rec.check((await rows(page)).length === 3 && ['grade-pass', 'main', 'old-look'].every((n) => app.includes(n)), 'the app lists the same three branches as `branch list`', `${app} | ${await names(rec)}`)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('B-02 Switch with unsaved changes', async () => {
  const rec = await Recorder.start('B-02', 'switch-unsaved', { gui: true })
  const page = rec.page!
  need(); await ids(rec); await clean(rec, V2)
  await cli(rec, 'branch', 'create', 'old-look', V1)
  const sw = (name: string) => page.getByLabel('Branches', { exact: true }).locator('li', { hasText: name }).getByRole('button', { name: 'Switch to' }).click()
  // notes.txt is the same on both branches; banner.ai differs.
  put('notes.txt', 'unsaved note\n'); put('scratch/idea.txt', 'new file\n')
  await rec.step('Open Branches with an unsaved notes.txt and a new file', () => open(page, 'branches/branches'))
  await rec.step('Switch to old-look (the changed file is the same on both branches)', async () => { await sw('old-look'); await page.waitForTimeout(4000) })
  let cur = (await branches(rec)).current
  rec.note(`clean-carry switch: current ${cur}, notes ${JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8'))}, idea ${existsSync(join(repo, 'scratch/idea.txt'))}, alerts ${await alerts(page)}`)
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'unsaved note\n' && existsSync(join(repo, 'scratch/idea.txt')), 'unsaved work survives the switch attempt (carried or blocked, never lost)')
  if (cur !== 'main') { await cli(rec, 'branch', 'switch', 'main'); await refocus(page); await page.waitForTimeout(1500) }
  put('art/banner.ai', { from: banner(4) })
  const mine = await hash('art/banner.ai')
  await rec.step('Edit banner.ai (differs between branches), then Switch to old-look', async () => { await refocus(page); await page.waitForTimeout(1500); await sw('old-look'); await page.waitForTimeout(4000) })
  cur = (await branches(rec)).current
  const blockedText = await alerts(page)
  rec.note(`blocked switch: current ${cur} | ${blockedText}`)
  rec.check(cur === 'main', 'the switch is blocked')
  rec.check((await hash('art/banner.ai')) === mine, 'my unsaved banner.ai is untouched')
  rec.check(/banner\.ai|unsaved|changes|lost/i.test(blockedText) && !raw(blockedText), 'the block is explained in plain words', blockedText.slice(0, 300))
  const anyway = page.getByRole('button', { name: 'Switch anyway…' })
  rec.check(await anyway.isVisible().catch(() => false), '"Switch anyway…" is offered')
  await rec.step('Switch anyway…, then cancel', async () => { await anyway.click(); await page.waitForTimeout(600); rec.note(`force dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: /Cancel|Keep/ }).click(); await page.waitForTimeout(700) })
  rec.check((await branches(rec)).current === 'main' && (await hash('art/banner.ai')) === mine, 'cancel keeps the branch and my file')
  await rec.step('Switch anyway…, confirm', async () => { await page.getByRole('button', { name: 'Switch anyway…' }).click(); await page.waitForTimeout(600); await page.getByRole('button', { name: 'Discard changes and switch' }).click(); await page.waitForTimeout(5000) })
  cur = (await branches(rec)).current
  rec.check(cur === 'old-look' && (await hash('art/banner.ai')) === (await corpus(banner(1))), 'forced switch lands on old-look with its banner.ai', cur)
  rec.note(`after force: notes ${JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8'))} (was "unsaved note"), new file idea.txt ${existsSync(join(repo, 'scratch/idea.txt')) ? 'kept' : 'DELETED'}, alerts ${await alerts(page)}`)
  const twin = await rec.cli(repo, ['branch', 'switch', 'main'])
  rec.note(`CLI switch back: exit ${twin.code} ${flat(twin.stdout + twin.stderr).slice(0, 200)}`)
  await clean(rec, V2)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('B-03 Delete: merged, unmerged, current', async () => {
  const rec = await Recorder.start('B-03', 'branch-delete', { gui: true })
  const page = rec.page!
  need(); await ids(rec); await clean(rec, V2)
  await cli(rec, 'branch', 'create', 'merged-one')
  await cli(rec, 'branch', 'create', 'solo-work'); await cli(rec, 'branch', 'switch', 'solo-work')
  put('art/photo.jpeg', { from: P2 }); await save(rec, 'solo: new photo')
  const solo = (await tip(rec))!.oid
  await cli(rec, 'branch', 'switch', 'main')
  const dlg = page.getByRole('dialog')
  await rec.step('Open Branches', () => open(page, 'branches/branches'))
  await rec.step('Menu of the current branch (main)', async () => { await page.getByRole('button', { name: 'Actions for main', exact: true }).click(); await page.waitForTimeout(400) })
  const items = await page.getByRole('menuitem').evaluateAll((els) => els.map((e) => `${(e as HTMLElement).innerText}${e.getAttribute('aria-disabled') === 'true' || e.hasAttribute('data-disabled') ? ' [off]' : ''}`))
  rec.check(items.some((i) => /switch to another branch first.*\[off\]/.test(i)), 'deleting the current branch is off and says why', JSON.stringify(items))
  await page.keyboard.press('Escape')
  await rec.step('Delete merged-one: cancel first', async () => { await rowMenu(page, 'merged-one', /^Delete/); rec.note(`delete dialog: ${await dialogText(page)}`); await dlg.getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(600) })
  rec.check((await names(rec)).includes('merged-one'), 'cancel keeps the branch')
  await rec.step('Delete merged-one: confirm', async () => { await rowMenu(page, 'merged-one', /^Delete/); await dlg.getByRole('button', { name: 'Delete branch' }).click(); await page.waitForTimeout(2500) })
  rec.check(!(await names(rec)).includes('merged-one'), 'a merged branch is deleted', await alerts(page))
  await rec.step('Delete solo-work (not merged) without the tick', async () => { await rowMenu(page, 'solo-work', /^Delete/); await dlg.getByRole('button', { name: 'Delete branch' }).click(); await page.waitForTimeout(2500) })
  const warn = `${await dialogText(page)} | ${await alerts(page)}`
  rec.note(`unmerged delete: ${warn}`)
  rec.check((await names(rec)).includes('solo-work'), 'an unmerged branch is not deleted without the extra tick')
  rec.check(/not (fully )?merged|unmerged|only this branch|lost/i.test(warn) && !raw(warn), 'the refusal says the work is not merged, in plain words', warn.slice(0, 300))
  await rec.step('Tick "Delete even if its work is not merged anywhere", confirm', async () => {
    if (!(await dlg.isVisible().catch(() => false))) await rowMenu(page, 'solo-work', /^Delete/)
    await dlg.getByText('More options').click(); await page.waitForTimeout(300)
    await dlg.getByText('Delete even if its work is not merged anywhere').click(); await page.waitForTimeout(400)
    rec.note(`dialog with the tick: ${await dialogText(page)}`)
    await rec.shot('force tick')
    await dlg.getByRole('button', { name: 'Delete branch' }).click(); await page.waitForTimeout(2500)
  })
  rec.check(!(await names(rec)).includes('solo-work'), 'forced delete removes the branch', await alerts(page))
  const back = await rec.cli(repo, ['branch', 'create', 'rescued', solo])
  rec.check(back.code === 0 && (await tip(rec, 'rescued'))?.oid === solo, 'the deleted branch\'s version can still be rescued by id', flat(back.stdout + back.stderr).slice(0, 200))
  rec.check((await rows(page)).length === (await branches(rec)).list.length - 1 || (await rows(page)).length === (await branches(rec)).list.length, 'the list follows', JSON.stringify(await rows(page)))
  await clean(rec, V2)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

/** Start a merge of `branch` from the Merge page with the given options ticked. */
async function startMerge(rec: Recorder, page: Page, branch: string, o: { ticks?: string[]; message?: string; strategy?: RegExp } = {}) {
  const dlg = page.getByRole('dialog')
  await go(page, 'branches/merge'); await refocus(page); await page.waitForTimeout(1500)
  await page.getByRole('button', { name: 'Start a merge' }).click(); await page.waitForTimeout(600)
  await choose(page, 'Merge this branch', branch, 400)
  if (o.strategy) await choose(page, 'When files clash', o.strategy, 400)
  if (o.message || o.ticks?.some((t) => !/Always/.test(t))) if (!(await dlg.getByLabel('Message for the merge version').isVisible().catch(() => false))) await dlg.getByText('More options').click()
  if (o.message) await dlg.getByLabel('Message for the merge version').fill(o.message)
  for (const t of o.ticks ?? []) await dlg.getByText(t).first().click()
  await page.waitForTimeout(300)
  await rec.shot(`merge dialog: ${branch} ${(o.ticks ?? []).join('+')}`)
  await dlg.getByRole('button', { name: 'Start merge' }).click()
  await page.waitForTimeout(5000)
}
const mergeSays = async (page: Page) => `result: ${flat(await page.getByTestId('merge-result').innerText().catch(() => ''))} | summary: ${flat(await page.getByTestId('merge-summary').innerText().catch(() => ''))} | alerts ${await alerts(page)}`

test('B-04 Clean merges: fast-forward, no-ff, squash, ff-only, no-commit, message', async () => {
  const rec = await Recorder.start('B-04', 'merge-clean', { gui: true })
  const page = rec.page!
  need(); await ids(rec); await clean(rec, V2)
  await cli(rec, 'branch', 'create', 'grade'); await cli(rec, 'branch', 'switch', 'grade')
  put('art/banner.ai', { from: banner(3) }); await save(rec, 'grade: banner v3')
  const G = (await tip(rec))!.oid
  await cli(rec, 'branch', 'switch', 'main')
  await rec.step('Open Branches > Merge', () => open(page, 'branches/merge'))
  rec.note(`merge page: ${flat(await page.locator('#app').innerText()).slice(-300)}`)

  await rec.step('Fast-forward merge of grade', () => startMerge(rec, page, 'grade'))
  let t = (await tip(rec))!
  rec.note(await mergeSays(page))
  rec.check(t.oid === G, 'fast-forward: main is now exactly the tip of grade (no merge version)', t.summary)
  rec.check((await hash('art/banner.ai')) === (await corpus(banner(3))), 'fast-forward: banner.ai on disk is v3')

  await cli(rec, 'reset', '--hard', V2)
  await rec.step('Always create a merge version (--no-ff) with a message', () => startMerge(rec, page, 'grade', { ticks: ['Always create a merge version'], message: 'Bring in the grade' }))
  t = (await tip(rec))!
  rec.note(await mergeSays(page))
  rec.check(t.parents.length === 2 && t.parents.includes(G) && t.parents.includes(V2), 'no-ff: a merge version with two parents', JSON.stringify(t.parents.map((p) => p.slice(0, 7))))
  rec.check(t.summary === 'Bring in the grade', 'the custom message is used', t.summary)

  await cli(rec, 'reset', '--hard', V2)
  await rec.step('Combine into one change (--squash)', () => startMerge(rec, page, 'grade', { ticks: ['Combine into one change'] }))
  t = (await tip(rec))!
  const sq = await statusOut(rec)
  rec.note(`${await mergeSays(page)} | tip "${t.summary}" parents ${t.parents.length} | status ${sq.text} | state files ${left().join(',')}`)
  rec.check(!t.parents.includes(G) && (await hash('art/banner.ai')) === (await corpus(banner(3))), 'squash: files merged, no link to grade')
  await rec.step('Changes after the squash', async () => { await go(page, 'files/changes'); await page.waitForTimeout(2500) })
  rec.note(`changes after squash: ${JSON.stringify(await store(page, 'changes', '({counts: s.counts, error: s.error})'))} | ${flat(await page.locator('#app').innerText()).slice(-250)}`)
  rec.check(!raw(await alerts(page)), 'the app shows no raw error after a squash', await alerts(page))

  await clean(rec, V2)
  await cli(rec, 'branch', 'create', 'grade', G)
  put('notes.txt', 'main moved on\n'); await save(rec, 'main: notes')
  const M = (await tip(rec))!.oid
  await rec.step('Only if it can simply catch up (--ff-only) when it cannot', () => startMerge(rec, page, 'grade', { ticks: ['Only if it can simply catch up'] }))
  const ff = `${await mergeSays(page)} | dialog ${await dialogText(page)}`
  rec.note(`ff-only refused: ${ff}`)
  rec.check((await tip(rec))!.oid === M, 'ff-only: refused, the branch did not move')
  rec.check(/catch up|fast-forward|both|diverged|new work/i.test(ff) && !raw(ff), 'the refusal is in plain words', ff.slice(0, 300))
  await page.keyboard.press('Escape'); await page.waitForTimeout(400)
  rec.check(left().length === 0, 'a refused merge leaves no state behind', left().join(','))
  await cli(rec, 'reset', '--hard', M) // BUG-024 merged anyway; put main back so the next step is a real test

  await rec.step('Let me look before it is saved (--no-commit)', () => startMerge(rec, page, 'grade', { ticks: ['Let me look before it is saved'] }))
  const nc = await statusOut(rec)
  rec.note(`${await mergeSays(page)} | status lines ${nc.lines}: ${nc.text} | state ${left().join(',')} | app: ${flat(await page.locator('#app').innerText()).slice(-300)}`)
  rec.check((await tip(rec))!.oid === M, 'no-commit: no version is made yet')
  rec.check((await hash('art/banner.ai')) === (await corpus(banner(3))) && readFileSync(join(repo, 'notes.txt'), 'utf8') === 'main moved on\n', 'no-commit: the merged files are on disk')
  rec.check(!raw(await alerts(page)), 'the app shows no raw error while the merge waits (BUG-021)', await alerts(page))
  await rec.step('Changes while the merge waits', async () => { await go(page, 'files/changes'); await page.waitForTimeout(2500) })
  const ch = `${JSON.stringify(await store(page, 'changes', '({counts: s.counts, error: s.error})'))} | ${await alerts(page)}`
  rec.note(`changes during a waiting merge: ${ch}`)
  rec.check(!raw(ch), 'Changes works while a merge waits (BUG-021)', ch.slice(0, 300))
  await rec.step('Finish merge', async () => { await go(page, 'branches/merge'); await page.waitForTimeout(1500); await page.getByRole('button', { name: 'Finish merge' }).first().click().catch(() => rec.note('no Finish merge button')); await page.waitForTimeout(5000) })
  t = (await tip(rec))!
  rec.note(`${await mergeSays(page)} | tip "${t.summary}"`)
  rec.check(t.parents.length === 2 && t.parents.includes(G) && t.parents.includes(M), 'finish makes the merge version with two parents', JSON.stringify(t.parents.map((p) => p.slice(0, 7))))
  rec.check(left().length === 0, 'no merge state is left', left().join(','))
  await rec.step('History graph after the merges', async () => { await go(page, 'history/versions'); await page.waitForTimeout(2500) })
  await clean(rec, V2)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

async function conflictSetup(rec: Recorder) {
  await ids(rec); await clean(rec, V2)
  await cli(rec, 'branch', 'create', 'retouch'); await cli(rec, 'branch', 'switch', 'retouch')
  put('art/photo.jpeg', { from: P3 }); put('notes.txt', 'retouch notes\n'); put('art/extra.ai', { from: banner(4) }); await save(rec, 'retouch: photo, notes, extra')
  const T = (await tip(rec))!.oid
  await cli(rec, 'branch', 'switch', 'main')
  put('art/photo.jpeg', { from: P2 }); put('notes.txt', 'main notes\n'); await save(rec, 'main: photo and notes')
  return { T, M: (await tip(rec))!.oid }
}
const conflictRows = (page: Page) => page.getByLabel('Conflicting files').locator('li').allInnerTexts().then((t) => t.map(flat)).catch(() => [] as string[])
const keep = (page: Page, path: string, side: 'Keep mine' | 'Keep theirs') => page.getByRole('group', { name: `Choose a version of ${path}` }).getByRole('button', { name: /^Keep / }).nth(side === 'Keep mine' ? 0 : 1).click()

test('B-05 Conflicting merge on binary files: mine, theirs, finish', async () => {
  const rec = await Recorder.start('B-05', 'merge-conflict', { gui: true })
  const page = rec.page!
  need()
  const { T, M } = await conflictSetup(rec)
  await rec.step('Open Merge and merge retouch (photo.jpeg and notes.txt clash)', async () => { await open(page, 'branches/merge'); await startMerge(rec, page, 'retouch') })
  let list = await conflictRows(page)
  const st = await statusOut(rec)
  rec.note(`conflict rows: ${JSON.stringify(list)} | ${await mergeSays(page)} | status --machine stdout lines: ${st.lines}`)
  rec.check(list.length === 2 && list.some((r) => r.includes('art/photo.jpeg')) && list.some((r) => r.includes('notes.txt')), 'both clashing files are listed', JSON.stringify(list))
  rec.check(st.lines === 1, '`status --machine` prints one line during a merge conflict (BUG-021)', `${st.lines} lines`)
  rec.check(!raw(await alerts(page)), 'no raw error on the Merge page', await alerts(page))
  const onDisk = await hash('art/photo.jpeg')
  rec.note(`photo.jpeg on disk during the conflict: ${onDisk === (await corpus(P2)) ? 'mine' : onDisk === (await corpus(P3)) ? 'theirs' : 'something else'}; notes.txt ${JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8').slice(0, 120))}; extra.ai present ${existsSync(join(repo, 'art/extra.ai'))}`)
  rec.check(await page.getByRole('button', { name: 'Finish merge' }).first().isDisabled(), 'Finish merge is off until every clash is decided')
  await rec.step('Visit Files > Changes during the conflict', async () => { await go(page, 'files/changes'); await page.waitForTimeout(3000) })
  const ch = `${JSON.stringify(await store(page, 'changes', '({counts: s.counts, error: s.error})'))} | ${await alerts(page)} | rail: ${flat(await page.locator('#app').innerText()).slice(0, 80)}`
  rec.note(`Changes during the conflict: ${ch}`)
  rec.check(!raw(ch), 'Changes works during a merge conflict (BUG-021)', ch.slice(0, 400))
  await rec.step('Visit History during the conflict', async () => { await go(page, 'history/versions'); await page.waitForTimeout(2500) })
  rec.check(!raw(await alerts(page)), 'History works during a merge conflict', await alerts(page))
  await rec.step('Back to Merge: keep mine for photo.jpeg', async () => { await go(page, 'branches/merge'); await page.waitForTimeout(2000); await keep(page, 'art/photo.jpeg', 'Keep mine'); await page.waitForTimeout(2000) })
  await rec.step('Keep theirs for notes.txt', async () => { await keep(page, 'notes.txt', 'Keep theirs'); await page.waitForTimeout(2000) })
  list = await conflictRows(page)
  rec.note(`after choosing: ${JSON.stringify(list)} | ${await mergeSays(page)}`)
  rec.check(list.some((r) => /photo\.jpeg.*Kept (mine|main)/.test(r)) && list.some((r) => /notes\.txt.*Kept (theirs|retouch)/.test(r)), 'each row shows the kept side', JSON.stringify(list))
  await rec.step('Finish merge', async () => { await page.getByRole('button', { name: 'Finish merge' }).first().click(); await page.waitForTimeout(6000) })
  let t = (await tip(rec))!
  rec.note(`${await mergeSays(page)} | tip "${t.summary}"`)
  rec.check(/^Merge branch 'retouch'( into main)?$/.test(t.summary) || !/theirs|HEAD/.test(t.summary), "BUG-023: the Finish merge version is named after the merged branch, not \"Merge branch 'theirs'\"", t.summary)
  rec.check(t.parents.length === 2 && t.parents.includes(T) && t.parents.includes(M), 'the merge version has both parents', JSON.stringify(t.parents.map((p) => p.slice(0, 7))))
  rec.check((await hash('art/photo.jpeg')) === (await corpus(P2)), 'photo.jpeg on disk is MINE by hash')
  rec.check(readFileSync(join(repo, 'notes.txt'), 'utf8') === 'retouch notes\n', 'notes.txt on disk is THEIRS', JSON.stringify(readFileSync(join(repo, 'notes.txt'), 'utf8')))
  rec.check(existsSync(join(repo, 'art/extra.ai')) && (await hash('art/extra.ai')) === (await corpus(banner(4))), 'the non-clashing new file from retouch came along')
  rec.check(left().length === 0 && (await statusOut(rec)).lines === 1, 'no merge state is left', left().join(','))
  const stored = await rec.cli(repo, ['reset', '--hard', 'HEAD'])
  rec.check(stored.code === 0 && (await hash('art/photo.jpeg')) === (await corpus(P2)) && readFileSync(join(repo, 'notes.txt'), 'utf8') === 'retouch notes\n', 'the saved merge version holds the same choices (restored from the store)')

  await cli(rec, 'reset', '--hard', M); rmSync(join(repo, 'art/extra.ai'), { force: true })
  await rec.step('Again with "Keep their version of every conflict"', () => startMerge(rec, page, 'retouch', { strategy: /Keep their version/ }))
  t = (await tip(rec))!
  rec.note(`theirs strategy: ${await mergeSays(page)} | tip "${t.summary}" parents ${t.parents.length} | rows ${JSON.stringify(await conflictRows(page))}`)
  rec.check(t.parents.length === 2 && (await hash('art/photo.jpeg')) === (await corpus(P3)) && readFileSync(join(repo, 'notes.txt'), 'utf8') === 'retouch notes\n', 'strategy "theirs": merged at once with their bytes for both files')
  await cli(rec, 'reset', '--hard', M); rmSync(join(repo, 'art/extra.ai'), { force: true })
  await rec.step('Again with "Keep my version of every conflict"', () => startMerge(rec, page, 'retouch', { strategy: /Keep my version/ }))
  t = (await tip(rec))!
  rec.note(`ours strategy: ${await mergeSays(page)} | tip "${t.summary}" parents ${t.parents.length}`)
  rec.check(t.parents.length === 2 && (await hash('art/photo.jpeg')) === (await corpus(P2)) && readFileSync(join(repo, 'notes.txt'), 'utf8') === 'main notes\n', 'strategy "ours": merged at once with my bytes for both files')
  rec.check(existsSync(join(repo, 'art/extra.ai')), 'strategy "ours" still brings the non-clashing file')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('B-06 Conflicting merge, then cancel', async () => {
  const rec = await Recorder.start('B-06', 'merge-abort', { gui: true })
  const page = rec.page!
  need()
  const { M } = await conflictSetup(rec)
  put('scratch/idea.txt', 'my new unsaved file\n')
  const before = await tree()
  await rec.step('Open Merge and merge retouch', async () => { await open(page, 'branches/merge'); await startMerge(rec, page, 'retouch') })
  rec.note(`rows ${JSON.stringify(await conflictRows(page))} | ${await mergeSays(page)}`)
  await rec.step('Keep theirs for photo.jpeg (half-way)', async () => { await keep(page, 'art/photo.jpeg', 'Keep theirs').catch(() => rec.note('no conflict row to click')); await page.waitForTimeout(2000) })
  await rec.step('Cancel merge, then back out of the question', async () => { await page.getByRole('button', { name: 'Cancel merge' }).first().click(); await page.waitForTimeout(600); rec.note(`cancel dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button').first().click(); await page.waitForTimeout(700) })
  rec.check(left().includes('MERGE_HEAD'), 'backing out keeps the merge going')
  await rec.step('Cancel merge, confirm', async () => { await page.getByRole('button', { name: 'Cancel merge' }).first().click(); await page.waitForTimeout(600); await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel merge' }).click(); await page.waitForTimeout(5000) })
  const after = await tree()
  rec.note(`${await mergeSays(page)} | app: ${flat(await page.locator('#app').innerText()).slice(-250)}`)
  rec.check((await tip(rec))!.oid === M, 'the branch is where it was')
  rec.check(diffTree(before, after).length === 0, 'every file is as before by hash, the unsaved new file too', diffTree(before, after).join(','))
  rec.check(left().length === 0, 'no merge state is left', left().join(','))
  const st = await statusOut(rec)
  rec.check(st.lines === 1 && /"staged":\[\]/.test(st.text) && /"modified":\[\]/.test(st.text), 'status is clean (only the new file is untracked)', st.text.slice(0, 300))
  await rec.step('Changes after the cancel', async () => { await go(page, 'files/changes'); await page.waitForTimeout(2500) })
  const ch = JSON.stringify(await store(page, 'changes', '({counts: s.counts, error: s.error})'))
  rec.check(/"total":1\b/.test(ch), 'Changes shows only my one new file', ch)
  rmSync(join(repo, 'scratch'), { recursive: true, force: true })
  await clean(rec, V2)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})
