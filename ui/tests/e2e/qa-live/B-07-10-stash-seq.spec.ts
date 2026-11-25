import { appendFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS, WORK } from './env'
import { ensureBig } from './big'
import { Recorder, closeTerm, workDir } from './rec'
import { choose, dialogText, flat, go, openRepo, refocus, toWelcome, visibleErrors } from './gui'
import { intCheck, sha256, walk } from './integrity'

// B-07 stash, B-08 rebase, B-09 cherry-pick, B-10 bisect. Each case builds its own small repository.
interface Commit { oid: string; parents: string[]; summary: string }
const banner = (n: number) => `creative-pairs/ai/banner_v${n}.ai`
const P1 = 'freepik__talk__71826.jpeg'
const P2 = 'freepik__talk__72772.jpeg'
const STATE = ['MERGE_HEAD', 'MERGE_MSG', 'REVERT_STATE', 'CHERRY_PICK_STATE', 'rebase-apply', 'BISECT_STATE', 'BISECT_LOG']

function kit(rec: Recorder, repo: string) {
  const cli = (...a: string[]) => rec.cli(repo, a, { quietFrame: true })
  const put = (file: string, body: string | { from: string }) => {
    mkdirSync(dirname(join(repo, file)), { recursive: true })
    if (typeof body === 'string') writeFileSync(join(repo, file), body)
    else cpSync(join(CORPUS, body.from), join(repo, file))
  }
  const save = async (message: string) => {
    await cli('add', '--all')
    const r = await cli('commit', '-m', message)
    if (r.code !== 0) rec.note(`setup commit failed: ${message}: ${flat(r.stdout + r.stderr).slice(-200)}`)
    return (await log())[0]!.oid
  }
  const log = async (...args: string[]): Promise<Commit[]> => {
    const r = await cli('log', '--machine', ...args)
    try { return (JSON.parse(r.stdout) as { data: { commits: Commit[] } }).data.commits } catch { rec.note(`log gave no JSON: ${flat(r.stdout + r.stderr).slice(0, 200)}`); return [] }
  }
  const read = (f: string) => (existsSync(join(repo, f)) ? readFileSync(join(repo, f), 'utf8') : '(missing)')
  const hash = (f: string) => sha256(join(repo, f))
  const left = () => STATE.filter((n) => existsSync(join(repo, '.mediagit', n)))
  const sweep = (when: string) => { const l = ['MERGE_HEAD', 'MERGE_MSG'].filter((n) => existsSync(join(repo, '.mediagit', n))); if (l.length) rec.note(`stale after ${when}: ${l.join(', ')} (removed by the harness)`); for (const n of l) rmSync(join(repo, '.mediagit', n)); return l.length === 0 }
  const status = async () => { const r = await cli('status', '--machine'); return { lines: r.stdout.split('\n').filter((l) => l.trim()).length, text: flat(r.stdout).slice(0, 500) } }
  const staged = async () => [...(await cli('diff', '--cached', '--machine')).stdout.matchAll(/"path":\s*"([^"]+)"/g)].map((m) => m[1]!).sort().join(',')
  const tree = async () => { const out: Record<string, string> = {}; for (const f of walk(repo)) out[f.slice(repo.length + 1).replace(/\\/g, '/')] = await sha256(f); return out }
  const init = async () => { rmSync(repo, { recursive: true, force: true }); mkdirSync(repo, { recursive: true }); await cli('init') }
  return { cli, put, save, log, read, hash, left, sweep, status, staged, tree, init }
}
const diffTree = (a: Record<string, string>, b: Record<string, string>) => [...new Set([...Object.keys(a), ...Object.keys(b)])].filter((k) => a[k] !== b[k])
const corpus = (f: string) => sha256(join(CORPUS, f))
const alerts = async (page: Page) => JSON.stringify(await visibleErrors(page)).slice(0, 500)
const raw = (s: string) => /os error|panicked|exit code|stderr|\bErr\(|protocol|unexpected output/i.test(s)
async function open(page: Page, repo: string, route: string) {
  await toWelcome(page)
  await openRepo(page, repo)
  await go(page, route)
  await page.waitForTimeout(1500)
}
async function tool(page: Page, name: RegExp) {
  await go(page, 'settings/advanced'); await refocus(page); await page.waitForTimeout(1200)
  await page.getByRole('tab', { name }).click()
  await page.waitForTimeout(1500)
}

test('B-07 Stash: every option, with one large file', async () => {
  const rec = await Recorder.start('B-07', 'stash', { gui: true })
  const page = rec.page!
  const repo = workDir('B-07')
  const k = kit(rec, repo)
  await k.init()
  k.put('art/banner.ai', { from: banner(1) }); k.put('art/photo.jpeg', { from: P1 }); k.put('notes.txt', 'base\n')
  ensureBig((s) => rec.note(s))
  const bigSrc = walk(join(WORK, 'big')).map((f) => ({ f, n: statSync(f).size })).sort((a, b) => b.n - a.n)[0]
  if (bigSrc) { mkdirSync(join(repo, 'video'), { recursive: true }); cpSync(bigSrc.f, join(repo, 'video/master.mov')); rec.note(`large file: ${bigSrc.f} ${(bigSrc.n / 1e9).toFixed(2)} GB`) } else rec.note('no large file found under work/big; large-file part skipped')
  await k.save('base')
  const base = await k.tree()
  const list = async () => { const r = await k.cli('stash', 'list', '--machine'); try { return (JSON.parse(r.stdout) as { data: { stashes: { index: number; message: string }[] } }).data.stashes } catch { return [] } }
  const rowsApp = () => page.getByLabel('Stashed work').locator('li').allInnerTexts().then((t) => t.map(flat)).catch(() => [] as string[])
  const edit = () => { k.put('art/banner.ai', { from: banner(2) }); writeFileSync(join(repo, 'notes.txt'), 'my edit\n'); k.put('scratch/idea.txt', 'new file\n') }
  const section = page.locator('section', { has: page.getByRole('heading', { name: 'Stash' }) })
  const stashNow = async (o: { message?: string; untracked?: boolean; only?: string[] } = {}) => {
    await refocus(page); await page.waitForTimeout(1500)
    await section.getByLabel('Message (optional)').fill(o.message ?? '')
    // BUG-027: the "Include new files" checkbox was removed (new files are always set aside).
    rec.check((await section.getByText('Include new files').count()) === 0, 'BUG-027: the stash form has no "Include new files" option (it never did anything)')
    if (o.only) { await section.getByText('Choose which files to set aside').click(); await page.waitForTimeout(400); for (const p of o.only) await page.getByTestId('stash-files').getByText(p, { exact: true }).click() }
    await page.waitForTimeout(300)
    await rec.shot('stash form')
    const btn = section.locator('button[type="submit"]')
    const label = `${flat(await btn.innerText())} disabled=${await btn.isDisabled()}`
    const t0 = Date.now()
    await btn.click()
    await page.waitForFunction(() => !document.querySelector('[data-testid="stash-progress"]'), null, { timeout: 300_000 }).catch(() => {})
    await page.waitForTimeout(2500)
    return `${label}, ${Date.now() - t0} ms`
  }

  edit()
  const mine = await k.tree()
  await rec.step('Open Branches > Stash with three changes (one new file)', () => open(page, repo, 'branches/stash'))
  rec.note(`page: ${flat(await page.locator('#app').innerText()).slice(-300)}`)
  await rec.step('Stash my changes, message "WIP lighting"', async () => { rec.note(`submit: ${await stashNow({ message: 'WIP lighting' })}`) })
  let t = await k.tree()
  rec.check(t['art/banner.ai'] === base['art/banner.ai'] && t['notes.txt'] === base['notes.txt'], 'tracked files are back to the saved version')
  rec.check(!('scratch/idea.txt' in t) && diffTree(t, base).length === 0, 'BUG-027: new files are always set aside with the stash (no option, help says so); the folder equals the saved version', diffTree(t, base).join(','))
  let l = await list()
  rec.check(l.length === 1 && l[0]!.message.includes('WIP lighting'), 'one stash with my message', JSON.stringify(l))
  rec.note(`rows: ${JSON.stringify(await rowsApp())}`)
  const show = await rec.cli(repo, ['stash', 'show'])
  rec.note(`stash show: ${flat(show.stdout + show.stderr).slice(0, 300)}`)
  await rec.step('Apply stash 0', async () => { await page.getByLabel('Stashed work').getByRole('button', { name: 'Apply stash 0', exact: true }).click(); await page.waitForTimeout(4000) })
  t = await k.tree()
  rec.check(diffTree(t, mine).length === 0, 'apply brings my work back byte for byte', diffTree(t, mine).join(','))
  rec.check((await list()).length === 1, 'apply keeps the entry')
  await rec.step('Apply again on top of the same changes', async () => { await page.getByLabel('Stashed work').getByRole('button', { name: 'Apply stash 0', exact: true }).click(); await page.waitForTimeout(3000) })
  const again = await alerts(page)
  rec.note(`second apply over my changes: ${again} | tree same ${diffTree(await k.tree(), mine).length === 0}`)
  rec.check(diffTree(await k.tree(), mine).length === 0, 'a second apply does not damage my files')
  rec.check(!raw(again), 'a refused apply is explained in plain words', again)
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  await k.cli('reset', '--hard', 'HEAD')
  await rec.step('Pop stash 0', async () => { await refocus(page); await page.waitForTimeout(1500); await page.getByRole('button', { name: 'Pop stash 0' }).click(); await page.waitForTimeout(4000) })
  rec.check(diffTree(await k.tree(), mine).length === 0 && (await list()).length === 0, 'pop brings the work back and removes the entry', `${(await list()).length} left`)

  await rec.step('Stash again (new files are set aside by default)', async () => { rec.note(`submit: ${await stashNow()}`) })
  t = await k.tree()
  rec.check(!('scratch/idea.txt' in t) && diffTree(t, base).length === 0, 'the new file is set aside too; the folder equals the saved version', diffTree(t, base).join(','))
  await rec.step('Pop it back', async () => { await page.getByRole('button', { name: 'Pop stash 0' }).click(); await page.waitForTimeout(4000) })
  rec.check(diffTree(await k.tree(), mine).length === 0, 'pop brings the new file back as well', diffTree(await k.tree(), mine).join(','))

  await rec.step('Choose which files: only notes.txt', async () => { rec.note(`submit: ${await stashNow({ only: ['notes.txt'] })}`) })
  t = await k.tree()
  rec.check(t['notes.txt'] === base['notes.txt'] && t['art/banner.ai'] === mine['art/banner.ai'], 'only notes.txt is set aside; banner.ai keeps my edit')
  await page.getByRole('button', { name: 'Pop stash 0' }).click(); await page.waitForTimeout(4000)

  await k.cli('add', 'art/banner.ai')
  rec.note(`before the --index test: staged = ${await k.staged()}`)
  await rec.step('Stash with banner.ai staged and notes.txt not', async () => { rec.note(`submit: ${await stashNow()}`) })
  await rec.step('Options… then "Also restore what was staged", Apply and remove', async () => {
    await page.getByRole('button', { name: 'Options for stash 0' }).click(); await page.waitForTimeout(600)
    rec.note(`options dialog: ${await dialogText(page)}`)
    await page.getByRole('dialog').getByText('Also restore what was staged for saving').click(); await page.waitForTimeout(300)
    await rec.shot('index ticked')
    await page.getByRole('dialog').getByRole('button', { name: 'Apply and remove' }).click(); await page.waitForTimeout(4000)
  })
  rec.check((await k.staged()) === 'art/banner.ai', '--index restores the Include state: only banner.ai is staged', await k.staged())
  rec.check((await k.tree())['notes.txt'] === mine['notes.txt'], 'and notes.txt has my edit, not staged')
  await k.cli('reset', '--hard', 'HEAD'); rmSync(join(repo, 'scratch'), { recursive: true, force: true })

  writeFileSync(join(repo, 'notes.txt'), 'stashed text\n')
  await rec.step('Stash a notes.txt edit, then edit notes.txt differently', async () => { rec.note(`submit: ${await stashNow({ message: 'clash' })}`); writeFileSync(join(repo, 'notes.txt'), 'newer text\n'); await refocus(page); await page.waitForTimeout(1500) })
  await rec.step('Pop on top of the newer edit', async () => { await page.getByRole('button', { name: 'Pop stash 0' }).click(); await page.waitForTimeout(3500) })
  const clash = await alerts(page)
  rec.note(`pop on conflict: ${clash} | notes ${JSON.stringify(k.read('notes.txt'))} | stashes ${(await list()).length}`)
  rec.check(k.read('notes.txt') === 'newer text\n', 'my newer edit is not overwritten')
  rec.check((await list()).length === 1, 'pop on conflict keeps the stash')
  rec.check(clash.length > 4 && !raw(clash), 'the refusal is in plain words', clash)
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  await rec.step('Options… "Overwrite my current changes", Overwrite and apply', async () => {
    await page.getByRole('button', { name: 'Options for stash 0' }).click(); await page.waitForTimeout(600)
    await page.getByRole('dialog').getByText('Overwrite my current changes to these files').click(); await page.waitForTimeout(400)
    rec.note(`force dialog: ${await dialogText(page)}`)
    await rec.shot('force ticked')
    await page.getByRole('dialog').getByRole('button', { name: 'Overwrite and apply', exact: true }).click(); await page.waitForTimeout(4000)
  })
  rec.check(k.read('notes.txt') === 'stashed text\n', '--force replaces my edit with the stashed one', JSON.stringify(k.read('notes.txt')))
  await k.cli('reset', '--hard', 'HEAD')

  if (bigSrc) {
    appendFileSync(join(repo, 'video/master.mov'), Buffer.alloc(1 << 20, 7))
    const bigMine = await k.hash('video/master.mov')
    await rec.step('Stash an edited large file', async () => { rec.note(`submit: ${await stashNow({ message: 'big edit' })}`) })
    rec.check((await k.hash('video/master.mov')) === base['video/master.mov'], 'the large file is back to the saved bytes')
    const t0 = Date.now()
    await rec.step('Pop the large file back', async () => { await page.getByRole('button', { name: 'Pop stash 0' }).click(); await page.waitForFunction(() => !document.querySelector('[data-testid="stash-progress"]'), null, { timeout: 300_000 }).catch(() => {}); await page.waitForTimeout(3000) })
    rec.note(`pop of the large file took about ${Date.now() - t0} ms`)
    rec.check((await k.hash('video/master.mov')) === bigMine, 'the edited large file comes back byte for byte')
    await k.cli('reset', '--hard', 'HEAD')
  }

  writeFileSync(join(repo, 'notes.txt'), 'a\n'); await k.cli('stash', 'push', '-m', 'one')
  writeFileSync(join(repo, 'notes.txt'), 'b\n'); await k.cli('stash', 'push', '-m', 'two')
  await rec.step('Drop stash 1: cancel, then confirm', async () => {
    await refocus(page); await page.waitForTimeout(1500)
    rec.note(`rows: ${JSON.stringify(await rowsApp())}`)
    await page.getByRole('button', { name: 'Drop stash 1' }).click(); await page.waitForTimeout(500); rec.note(`drop dialog: ${await dialogText(page)}`)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500)
    await page.getByRole('button', { name: 'Drop stash 1' }).click(); await page.waitForTimeout(500)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Drop stash' }).click(); await page.waitForTimeout(2500)
  })
  l = await list()
  rec.check(l.length === 2 && !l.some((s) => s.message === 'one'), 'the right entry is dropped (stash 1 of three: the middle one)', JSON.stringify(l.map((s) => s.message)))
  await rec.step('Remove all, confirm', async () => { await page.getByRole('button', { name: 'Remove all' }).click(); await page.waitForTimeout(500); rec.note(`clear dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: 'Remove all' }).click(); await page.waitForTimeout(2500) })
  rec.check((await list()).length === 0, 'every stash is removed')
  rec.check(/Nothing set aside/.test(await page.locator('#app').innerText()), 'the empty state shows')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
  rmSync(join(repo, 'video'), { recursive: true, force: true })
})

async function seqRepo(rec: Recorder, name: string) {
  const repo = workDir(name)
  const k = kit(rec, repo)
  await k.init()
  k.put('art/banner.ai', { from: banner(1) }); k.put('notes.txt', 'base\n')
  const v1 = await k.save('v1 base')
  await k.cli('branch', 'create', 'feat'); await k.cli('branch', 'switch', 'feat')
  k.put('art/extra.ai', { from: banner(3) }); const f1 = await k.save('feat: extra art')
  k.put('art/photo.jpeg', { from: P1 }); const f2 = await k.save('feat: photo')
  await k.cli('branch', 'create', 'clash', v1); await k.cli('branch', 'switch', 'clash')
  k.put('notes.txt', 'clash side\n'); const c1 = await k.save('clash: notes')
  k.put('art/second.jpeg', { from: P2 }); const c2 = await k.save('clash: second photo')
  await k.cli('branch', 'switch', 'main')
  k.put('art/banner.ai', { from: banner(2) }); k.put('notes.txt', 'main side\n'); const v2 = await k.save('v2 main moves on')
  return { repo, k, v1, v2, f1, f2, c1, c2 }
}
const clashRows = (page: Page) => page.getByLabel('Clashing files').locator('li').allInnerTexts().then((t) => t.map(flat)).catch(() => [] as string[])
const says = async (page: Page, kind: string) => `result: ${flat(await page.getByTestId(`${kind}-result`).innerText().catch(() => ''))} | summary: ${flat(await page.getByTestId(`${kind}-summary`).innerText().catch(() => ''))} | alerts ${await alerts(page)}`

test('B-08 Rebase: clean, conflict then continue, cancel', async () => {
  const rec = await Recorder.start('B-08', 'rebase', { gui: true })
  const page = rec.page!
  const { repo, k, v2, f2, c2 } = await seqRepo(rec, 'B-08')
  await k.cli('branch', 'switch', 'feat')
  await rec.step('Open Advanced tools > rebase, on branch feat', async () => { await open(page, repo, 'settings/advanced'); await tool(page, /rebase/i) })
  rec.note(`panel: ${flat(await page.locator('#app').innerText()).slice(-400)}`)
  await rec.step('Rebase onto main, cancel the question', async () => { await choose(page, 'Rebase onto', /main/); await page.getByRole('button', { name: 'Start rebase…' }).click(); await page.waitForTimeout(600); rec.note(`confirm: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500) })
  rec.check((await k.log())[0]!.oid === f2, 'cancel does nothing')
  await rec.step('Rebase onto main, confirm', async () => { await page.getByRole('button', { name: 'Start rebase…' }).click(); await page.waitForTimeout(600); await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click(); await page.waitForTimeout(6000) })
  let l = await k.log()
  rec.note(`${await says(page, 'rebase')} | log: ${l.map((c) => c.summary).join(' < ')}`)
  rec.check(l.map((c) => c.summary).join('|') === 'feat: photo|feat: extra art|v2 main moves on|v1 base' && l.every((c) => c.parents.length <= 1), 'history is linear: both feat versions now sit on top of main')
  rec.check(l[2]!.oid === v2 && l[0]!.oid !== f2, 'the versions were rewritten on top of v2')
  rec.check((await k.hash('art/banner.ai')) === (await corpus(banner(2))) && (await k.hash('art/extra.ai')) === (await corpus(banner(3))) && (await k.hash('art/photo.jpeg')) === (await corpus(P1)), 'files: main banner plus both feat files, by hash')
  rec.check(k.left().length === 0, 'no state left after a clean rebase', k.left().join(','))

  await k.cli('branch', 'switch', 'clash')
  const before = await k.tree()
  await rec.step('Switch to clash (outside), rebase onto main: notes.txt clashes', async () => { await tool(page, /rebase/i); await choose(page, 'Rebase onto', /main/); await page.getByRole('button', { name: 'Start rebase…' }).click(); await page.waitForTimeout(600); await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click(); await page.waitForTimeout(6000) })
  let rows = await clashRows(page)
  const st = await k.status()
  rec.note(`clash rows: ${JSON.stringify(rows)} | ${await says(page, 'rebase')} | status lines ${st.lines} | state ${k.left().join(',')} | notes ${JSON.stringify(k.read('notes.txt'))}`)
  rec.check(rows.length === 1 && rows[0]!.includes('notes.txt'), 'the clashing file is listed', JSON.stringify(rows))
  rec.check(!raw(await alerts(page)), 'no raw error on the panel', await alerts(page))
  await rec.step('Cancel rebase, confirm', async () => { await page.getByRole('button', { name: 'Cancel rebase' }).first().click(); await page.waitForTimeout(600); rec.note(`cancel dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel rebase' }).click(); await page.waitForTimeout(5000) })
  rec.check((await k.log())[0]!.oid === c2, 'cancel puts the branch back on its old tip')
  rec.check(diffTree(before, await k.tree()).length === 0, 'cancel restores every file by hash', diffTree(before, await k.tree()).join(','))
  rec.check(k.sweep('rebase cancel'), 'cancel leaves no operation in progress (BUG-020)')
  await k.cli('reset', '--hard', 'HEAD') // BUG-026 leaves markers in notes.txt; clean up so the continue path can be tested

  await rec.step('Rebase again, keep the second choice for notes.txt, continue', async () => {
    await tool(page, /rebase/i); await choose(page, 'Rebase onto', /main/); await page.getByRole('button', { name: 'Start rebase…' }).click(); await page.waitForTimeout(600); await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click(); await page.waitForTimeout(6000)
    const group = page.getByRole('group', { name: 'Choose a version of notes.txt' })
    rec.note(`choice buttons: ${JSON.stringify(await group.getByRole('button').allInnerTexts())}`)
    await group.getByRole('button').nth(1).click(); await page.waitForTimeout(2000)
    await rec.shot('choice made')
    await page.getByRole('button', { name: 'Continue rebase' }).first().click(); await page.waitForTimeout(6000)
  })
  l = await k.log()
  rows = await clashRows(page)
  rec.note(`${await says(page, 'rebase')} | log: ${l.map((c) => c.summary).join(' < ')} | notes ${JSON.stringify(k.read('notes.txt'))} | rows ${JSON.stringify(rows)}`)
  rec.check(l.map((c) => c.summary).join('|') === 'clash: second photo|clash: notes|v2 main moves on|v1 base', 'continue finishes: both versions sit on top of main')
  rec.check(['clash side\n', 'main side\n'].includes(k.read('notes.txt')), 'notes.txt is one whole side, no markers', JSON.stringify(k.read('notes.txt')))
  rec.note(`the second button kept: ${k.read('notes.txt') === 'clash side\n' ? 'the branch being rebased (clash)' : 'the branch rebased onto (main)'}`)
  rec.check((await k.hash('art/second.jpeg')) === (await corpus(P2)), 'the second version\'s file arrived by hash')
  rec.check(k.sweep('rebase continue'), 'continue leaves no operation in progress (BUG-020)')
  rec.check((await k.status()).lines === 1, 'status --machine is one line again')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('B-09 Cherry-pick: one, already applied, conflict, cancel, options', async () => {
  const rec = await Recorder.start('B-09', 'cherry-pick', { gui: true })
  const page = rec.page!
  const { repo, k, v2, f1, c1 } = await seqRepo(rec, 'B-09')
  const pickIt = async (branch: string, version: RegExp, ticks: string[] = []) => {
    await tool(page, /cherry-pick/i)
    await choose(page, 'From branch', branch)
    await choose(page, 'Copy this version', version)
    if (ticks.length) { if (!(await page.getByText(ticks[0]!).first().isVisible().catch(() => false))) await page.getByText('More options').first().click(); await page.waitForTimeout(300); for (const t of ticks) await page.getByText(t).first().click() }
    await rec.shot(`ready to copy ${version}`)
    await page.getByRole('button', { name: 'Copy version' }).first().click(); await page.waitForTimeout(600)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Copy version' }).click(); await page.waitForTimeout(6000)
    for (const t of ticks) await page.getByText(t).first().click().catch(() => {})
  }
  await rec.step('Open Advanced tools > cherry-pick on main', async () => { await open(page, repo, 'settings/advanced'); await tool(page, /cherry-pick/i) })
  rec.note(`panel: ${flat(await page.locator('#app').innerText()).slice(-400)}`)
  await rec.step('Copy "feat: extra art" from feat', () => pickIt('feat', /feat: extra art/))
  let l = await k.log()
  rec.note(`${await says(page, 'pick')} | log: ${l.map((c) => c.summary).join(' < ')}`)
  rec.check(l[0]!.summary === 'feat: extra art' && l[0]!.oid !== f1 && l[0]!.parents[0] === v2, 'a new version with the same message sits on top of main')
  rec.check((await k.hash('art/extra.ai')) === (await corpus(banner(3))) && !existsSync(join(repo, 'art/photo.jpeg')), 'only that version\'s file arrived, by hash')
  const tipA = l[0]!.oid
  await rec.step('Copy the same version again (already applied)', () => pickIt('feat', /feat: extra art/))
  const dup = await says(page, 'pick')
  rec.note(`already applied: ${dup} | state ${k.left().join(',')} | status lines ${(await k.status()).lines}`)
  rec.check((await k.log())[0]!.oid === tipA, 'no empty version is made')
  rec.check(/already|nothing|empty|no change/i.test(dup) && !raw(dup), 'the refusal says it is already there, in plain words', dup.slice(0, 300))
  rec.check(k.left().length === 0, 'an empty copy leaves no state behind', k.left().join(','))
  for (const n of STATE) rmSync(join(repo, '.mediagit', n), { recursive: true, force: true })
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})

  const before = await k.tree()
  await rec.step('Copy "clash: notes" (notes.txt clashes with main)', () => pickIt('clash', /clash: notes/))
  let rows = await clashRows(page)
  rec.note(`clash rows: ${JSON.stringify(rows)} | ${await says(page, 'pick')} | state ${k.left().join(',')} | notes ${JSON.stringify(k.read('notes.txt'))}`)
  rec.check(rows.length === 1 && rows[0]!.includes('notes.txt'), 'the clashing file is listed', JSON.stringify(rows))
  await rec.step('Cancel cherry-pick, confirm', async () => { await page.getByRole('button', { name: /^Cancel (copy|cherry-pick)/ }).first().click(); await page.waitForTimeout(600); rec.note(`cancel dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: /^Cancel (copy|cherry-pick)/ }).click(); await page.waitForTimeout(5000) })
  rec.check((await k.log())[0]!.oid === tipA && diffTree(before, await k.tree()).length === 0, 'cancel restores the branch and every file by hash', diffTree(before, await k.tree()).join(','))
  rec.check(k.sweep('cherry-pick cancel'), 'cancel leaves no operation in progress (BUG-020)')
  await rec.step('Copy it again, keep the second choice, continue', async () => {
    await pickIt('clash', /clash: notes/)
    const group = page.getByRole('group', { name: 'Choose a version of notes.txt' })
    rec.note(`choice buttons: ${JSON.stringify(await group.getByRole('button').allInnerTexts())}`)
    await group.getByRole('button').nth(1).click(); await page.waitForTimeout(2000)
    await page.getByRole('button', { name: /^Continue (copy|cherry-pick)/ }).first().click(); await page.waitForTimeout(6000)
  })
  l = await k.log()
  rec.note(`${await says(page, 'pick')} | top "${l[0]?.summary}" | notes ${JSON.stringify(k.read('notes.txt'))}`)
  rec.check(l[0]!.summary === 'clash: notes' && l[0]!.oid !== c1 && l[0]!.parents[0] === tipA, 'continue makes the copied version')
  rec.check(k.read('notes.txt') === 'clash side\n', 'the second choice is "theirs": the copied version\'s notes', JSON.stringify(k.read('notes.txt')))
  rec.check(k.sweep('cherry-pick continue'), 'continue leaves no operation in progress (BUG-020)')

  const tipB = l[0]!.oid
  await rec.step('Copy "feat: photo" with "Let me look" and "Note where it was copied from"', () => pickIt('feat', /feat: photo/, ['Let me look before it is saved']))
  rec.note(`no-commit: ${await says(page, 'pick')} | staged ${await k.staged()} | state ${k.left().join(',')}`)
  rec.check((await k.log())[0]!.oid === tipB && existsSync(join(repo, 'art/photo.jpeg')), '--no-commit: the file arrives, no version is made')
  await k.cli('reset', '--hard', 'HEAD'); rmSync(join(repo, 'art/photo.jpeg'), { force: true }); for (const n of STATE) rmSync(join(repo, '.mediagit', n), { recursive: true, force: true })
  await rec.step('Copy "feat: photo" with "Note where it was copied from"', () => pickIt('feat', /feat: photo/, ['Note where it was copied from']))
  const full = await rec.cli(repo, ['show', 'HEAD'])
  rec.check(/cherry picked from commit/.test(full.stdout), '--append-message adds the source note', flat(full.stdout).slice(0, 300))
  const two = await rec.cli(repo, ['cherry-pick', (await k.log('clash'))[0]!.oid])
  rec.note(`CLI twin, copy "clash: second photo": exit ${two.code} ${flat(two.stdout + two.stderr).slice(0, 200)}`)
  rec.check((await k.hash('art/second.jpeg')) === (await corpus(P2)), 'CLI cherry-pick brings the file by hash')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})

test('B-10 Bisect over 16 versions', async () => {
  const rec = await Recorder.start('B-10', 'bisect', { gui: true })
  const page = rec.page!
  const repo = workDir('B-10')
  const k = kit(rec, repo)
  await k.init()
  const oids: string[] = []
  for (let i = 1; i <= 16; i++) {
    k.put('state.txt', i >= 11 ? 'broken\n' : 'ok\n')
    k.put('art/banner.ai', { from: banner(1 + (i % 4)) })
    k.put(`shots/s${String(i).padStart(2, '0')}.txt`, `shot ${i}\n`)
    oids.push(await k.save(`version ${String(i).padStart(2, '0')}`))
  }
  const firstBad = oids[10]!
  const tip = oids[15]!
  await rec.step('Open Advanced tools > bisect', async () => { await open(page, repo, 'settings/advanced'); await tool(page, /bisect/i) })
  rec.note(`panel: ${flat(await page.locator('#app').innerText()).slice(-400)}`)
  await rec.step('Working version = version 01, Start bisect', async () => { await choose(page, 'A version that was working', /version 01/); await page.getByRole('button', { name: 'Start bisect' }).click(); await page.waitForTimeout(5000) })
  let steps = 0
  let text = flat(await page.getByTestId('bisect-state').innerText().catch(() => ''))
  rec.note(`after start: ${text} | alerts ${await alerts(page)}`)
  while (!/first bad version/i.test(text) && steps < 8) {
    const onDisk = k.read('state.txt').trim()
    steps++
    await rec.step(`Step ${steps}: state.txt on disk says "${onDisk}", so mark ${onDisk === 'ok' ? 'good' : 'bad'}`, async () => { await page.getByRole('button', { name: onDisk === 'ok' ? 'It works (good)' : 'It is broken (bad)' }).click(); await page.waitForTimeout(4500) })
    text = flat(await page.getByTestId('bisect-state').innerText().catch(() => ''))
    rec.note(`step ${steps}: ${text.slice(0, 260)}`)
  }
  rec.check(/first bad version/i.test(text), 'the bisect finishes with a verdict', text.slice(0, 200))
  rec.check(text.includes(firstBad.slice(0, 7)) || text.includes('version 11'), 'it names version 11 as the first bad one', text.slice(0, 200))
  rec.check(steps <= 5, 'it needs at most 5 answers for 16 versions', String(steps))
  const blog = await rec.cli(repo, ['bisect', 'log'])
  rec.note(`bisect log: ${flat(blog.stdout + blog.stderr).slice(0, 400)}`)
  writeFileSync(join(WORK, 'B-10-bisect.log'), blog.stdout)
  rec.note(`rail while bisecting: ${flat(await page.locator('#app').innerText()).slice(0, 80)} | status lines ${(await k.status()).lines}`)
  await rec.step('Visit Files > Changes while the bisect is open', async () => { await go(page, 'files/changes'); await page.waitForTimeout(2500) })
  rec.check(!raw(await alerts(page)), 'Changes works during a bisect', await alerts(page))
  await rec.step('End bisect…, confirm', async () => { await tool(page, /bisect/i); await page.getByRole('button', { name: 'End bisect…' }).click(); await page.waitForTimeout(600); rec.note(`end dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: 'End bisect' }).click(); await page.waitForTimeout(5000) })
  const br = await k.cli('branch', 'list', '--machine')
  rec.check(/"current":"main"/.test(br.stdout) && (await k.log())[0]!.oid === tip, 'ending returns to main at its tip', br.stdout.slice(0, 120))
  rec.check(k.read('state.txt') === 'broken\n' && existsSync(join(repo, 'shots/s16.txt')), 'the files of the tip are back on disk')
  rec.check(k.left().length === 0, 'no bisect state is left', k.left().join(','))
  const rp = await rec.cli(repo, ['bisect', 'replay', join(WORK, 'B-10-bisect.log')])
  rec.note(`CLI bisect replay: exit ${rp.code} ${flat(rp.stdout + rp.stderr).slice(0, 300)}`)
  const sk = await rec.cli(repo, ['bisect', 'reset'])
  rec.note(`CLI bisect reset: exit ${sk.code} ${flat(sk.stdout + sk.stderr).slice(0, 200)}`)
  rec.check((await k.log())[0]!.oid === tip, 'after replay and reset the branch is at its tip again')
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})
