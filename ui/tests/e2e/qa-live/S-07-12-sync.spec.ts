import { execFileSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS, WORK } from './env'
import { ensureBig } from './big'
import { Recorder, closeTerm, workDir } from './rec'
import { answerPicker, dialogText, flat, go, openRepo, refocus, rowMenu, toWelcome, visibleErrors } from './gui'
import { intCheck, sha256, walk } from './integrity'
import { startServer, type LocalServer } from './server'

// S-07 remotes and token, S-09 diverged push, S-10 pull variants, S-11 locks, S-12 remote files.
// One harness server without sign-in (work/S-07-srv). Each case makes its own client repositories.
interface Commit { oid: string; parents: string[]; summary: string }
const banner = (n: number) => `creative-pairs/ai/banner_v${n}.ai`
const P1 = 'freepik__talk__71826.jpeg'
const P2 = 'freepik__talk__72772.jpeg'
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
let srv: LocalServer | null = null
async function server(...repos: string[]): Promise<LocalServer> {
  if (!srv) srv = await startServer(join(WORK, 'S-07-srv'))
  for (const r of repos) if (!existsSync(join(srv.dir, 'repos', r))) srv.addRepo(r)
  return srv
}
function kit(rec: Recorder, repo: string) {
  const cli = (...a: string[]) => rec.cli(repo, a, { quietFrame: true, env: NOKEY })
  const loud = (...a: string[]) => rec.cli(repo, a, { env: NOKEY })
  const put = (file: string, body: string | { from: string }) => { mkdirSync(dirname(join(repo, file)), { recursive: true }); if (typeof body === 'string') writeFileSync(join(repo, file), body); else cpSync(join(CORPUS, body.from), join(repo, file)) }
  const log = async (...args: string[]): Promise<Commit[]> => { const r = await cli('log', '--machine', ...args); try { return (JSON.parse(r.stdout) as { data: { commits: Commit[] } }).data.commits } catch { return [] } }
  const save = async (message: string) => { await cli('add', '--all'); const r = await cli('commit', '-m', message); if (r.code !== 0) rec.note(`setup commit failed: ${message}: ${flat(r.stdout + r.stderr).slice(-200)}`); return (await log())[0]!.oid }
  const read = (f: string) => (existsSync(join(repo, f)) ? readFileSync(join(repo, f), 'utf8') : '(missing)')
  const hash = (f: string) => sha256(join(repo, f))
  const state = () => ['MERGE_HEAD', 'MERGE_MSG', 'rebase-apply'].filter((n) => existsSync(join(repo, '.mediagit', n)))
  const init = async () => { rmSync(repo, { recursive: true, force: true }); mkdirSync(repo, { recursive: true }); await cli('init') }
  return { repo, cli, loud, put, log, save, read, hash, state, init }
}
const corpus = (f: string) => sha256(join(CORPUS, f))
const alerts = async (page: Page) => JSON.stringify(await visibleErrors(page)).slice(0, 600)
const raw = (s: string) => /os error|panicked|exit code|stderr|\bErr\(|protocol|unexpected output|status: \d{3}|failed with status/i.test(s)
async function open(page: Page, repo: string, route: string) { await toWelcome(page); await openRepo(page, repo); await go(page, route); await page.waitForTimeout(2000) }
const idle = (page: Page, ms = 120_000) => page.waitForFunction(() => !document.querySelector('[data-testid="transfer-progress"]') && !document.querySelector('[data-activity-pill] [role="progressbar"]'), null, { timeout: ms }).catch(() => {})
// Scoped to the header group: the Activity pill's name also starts with "Send origin…" / "Get latest origin…".
const header = (page: Page, name: 'Get latest' | 'Send') => page.getByRole('group', { name: 'Share work' }).getByRole('button', { name: new RegExp(`^${name}`) }).first()
async function act(page: Page, name: 'Get latest' | 'Send', confirm?: string) {
  const drawer = page.getByRole('button', { name: 'Close', exact: true })
  if (await page.getByRole('heading', { name: 'Activity' }).isVisible().catch(() => false)) await drawer.first().click().catch(() => {})
  await header(page, name).click()
  await page.waitForTimeout(1200)
  if (confirm) { const b = page.getByRole('alertdialog').getByRole('button', { name: confirm }); if (await b.isVisible().catch(() => false)) await b.click() }
  await page.waitForTimeout(1500)
  await idle(page)
  await page.waitForTimeout(1500)
}
async function options(page: Page, verb: 'Fetch' | 'Get latest' | 'Send') {
  const more = page.getByTestId('sync-more')
  if (!(await more.getByLabel('Options for').isVisible().catch(() => false))) await more.locator('summary, button').first().click()
  await page.waitForTimeout(300)
  await more.getByLabel('Options for').getByRole('radio', { name: verb }).or(more.getByLabel('Options for').getByRole('button', { name: verb })).first().click()
  await page.waitForTimeout(400)
  return more
}
const keychain = () => { try { return execFileSync('cmdkey', ['/list'], { encoding: 'utf8' }).split(/\r?\n/).filter((l) => /target/i.test(l) && /mediagit/i.test(l)).map((l) => l.trim()) } catch { return ['(cmdkey failed)'] } }

test('S-07 Remotes: add with a token, rename, change address, remove', async () => {
  const rec = await Recorder.start('S-07', 'remotes', { gui: true })
  const page = rec.page!
  const s = await server('show', 'other')
  const k = kit(rec, workDir('S-07'))
  await k.init(); k.put('art/banner.ai', { from: banner(1) }); await k.save('first')
  const TOKEN = 'qa-live-fake-token-0123456789abcdef'
  const before = keychain()
  rec.note(`keychain entries for mediagit before: ${JSON.stringify(before)}`)
  const dlg = page.getByRole('dialog')
  await rec.step('Open Branches > Sync (no remote yet)', () => open(page, k.repo, 'branches/sync'))
  rec.check(/No remote yet/.test(await page.locator('#app').innerText()), 'the empty state says "No remote yet"')
  rec.note(`header buttons with no remote: Get latest disabled=${await header(page, 'Get latest').isDisabled().catch(() => 'absent')}, Send disabled=${await header(page, 'Send').isDisabled().catch(() => 'absent')}`)
  await rec.step('Add remote: bad name and bad address', async () => {
    await page.getByRole('button', { name: 'Add remote' }).first().click(); await page.waitForTimeout(500)
    for (const [n, u] of [['has space', s.url('show')], ['origin', 'not a url'], ['origin', 'ftp://x/y'], ['origin', 'http://'], ['-x', s.url('show')]] as const) { await dlg.getByLabel('Name').fill(n); await dlg.getByLabel('Address').fill(u); await page.waitForTimeout(350); rec.note(`name "${n}" address "${u.slice(0, 30)}": ${flat(await dlg.innerText()).replace(/More options.*/, '').slice(60, 300)} | add disabled=${await dlg.getByRole('button', { name: 'Add remote' }).isDisabled()}`) }
  })
  await rec.step('Add origin with an access token', async () => {
    await dlg.getByLabel('Name').fill('origin'); await dlg.getByLabel('Address').fill(s.url('show'))
    const more = page.getByTestId('remote-more')
    if (!(await more.getByLabel('Access token').isVisible().catch(() => false))) await more.locator('summary, button').first().click()
    await page.waitForTimeout(300)
    rec.note(`token field type: ${await more.getByLabel('Access token').getAttribute('type')} | options: ${flat(await more.innerText()).slice(0, 300)}`)
    await more.getByLabel('Access token').fill(TOKEN); await page.waitForTimeout(300)
    await rec.shot('token typed')
    await dlg.getByRole('button', { name: 'Add remote' }).click(); await page.waitForTimeout(3500)
  })
  const rl = await k.loud('remote', 'list', '--verbose')
  rec.check(rl.stdout.includes('origin') && rl.stdout.includes(s.url('show')), 'origin is in `remote list` with its address', flat(rl.stdout).slice(0, 200))
  const cfg = readFileSync(join(k.repo, '.mediagit', 'config.toml'), 'utf8')
  rec.note(`config.toml remote section: ${flat(cfg.slice(cfg.indexOf('[remotes'))).slice(0, 300).replace(TOKEN, '<TOKEN IN CLEAR TEXT>')}`)
  rec.check(!cfg.includes(TOKEN), 'the token is NOT in .mediagit\\config.toml')
  const inRepo = walk(join(k.repo, '.mediagit'), () => false).filter((f) => statSync(f).size < 5e6 && readFileSync(f).includes(TOKEN))
  rec.check(inRepo.length === 0, 'the token is in no file under .mediagit', inRepo.join(', '))
  const after = keychain()
  const added = after.filter((e) => !before.includes(e))
  rec.note(`keychain entries added: ${JSON.stringify(added)}`)
  rec.check(added.length >= 1, 'a new entry is in Windows Credential Manager', JSON.stringify(after))
  rec.note(`rows: ${flat(await page.locator('section.remotes').innerText().catch(() => ''))} | alerts ${await alerts(page)}`)
  rec.check(!(await page.locator('#app').innerText()).includes(TOKEN), 'the token is not shown anywhere on the page')
  await rec.step('Fetch with the stored (fake) token against a no-sign-in server', async () => { await page.getByRole('button', { name: 'Fetch' }).first().click(); await page.waitForTimeout(1500); await idle(page); await page.waitForTimeout(1000) })
  rec.note(`fetch result: ${await alerts(page)}`)
  await rec.step('Add origin again (duplicate name)', async () => { await page.getByRole('button', { name: 'Add remote' }).first().click(); await page.waitForTimeout(500); await dlg.getByLabel('Name').fill('origin'); await dlg.getByLabel('Address').fill(s.url('other')); await page.waitForTimeout(400) })
  const dup = flat(await dlg.innerText())
  rec.check(/exists|taken|already/i.test(dup) && (await dlg.getByRole('button', { name: 'Add remote' }).isDisabled()), 'a duplicate remote name is refused in the form', dup.slice(0, 250))
  await dlg.getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(400)
  await rec.step('Details…', async () => { await rowMenu(page, 'origin', /Details/); await page.waitForTimeout(1500) })
  rec.note(`details: ${flat(await page.getByTestId('remote-detail').innerText().catch(() => dialogText(page))).replace(TOKEN, '<TOKEN SHOWN>')}`)
  await page.getByRole('dialog').getByRole('button', { name: 'Close' }).last().click(); await page.waitForTimeout(300)
  await rec.step('Rename… to studio', async () => { await rowMenu(page, 'origin', /Rename/); await dlg.getByLabel('New name').fill('studio'); await page.waitForTimeout(300); await dlg.getByRole('button', { name: 'Rename' }).click(); await page.waitForTimeout(2500) })
  const r2 = await k.cli('remote', 'list', '--verbose')
  rec.check(r2.stdout.includes('studio') && !/\borigin\b/.test(r2.stdout), 'the remote is renamed', flat(r2.stdout).slice(0, 200))
  const kc2 = keychain()
  rec.note(`keychain after rename: ${JSON.stringify(kc2.filter((e) => !before.includes(e)))}`)
  await rec.step('Change address… only where versions are sent', async () => {
    await rowMenu(page, 'studio', /Change address/); await dlg.getByLabel('Address').fill(s.url('other'))
    const more = page.getByTestId('url-more'); await more.locator('summary, button').first().click(); await page.waitForTimeout(300)
    await more.getByText('Only where versions are sent').click(); await page.waitForTimeout(300)
    await rec.shot('push-only address')
    await dlg.getByRole('button', { name: 'Change address' }).click(); await page.waitForTimeout(2500)
  })
  const r3 = await k.loud('remote', 'show', 'studio')
  rec.note(`remote show: ${flat(r3.stdout + r3.stderr).slice(0, 400)}`)
  rec.check(r3.stdout.includes(s.url('show')) && r3.stdout.includes(s.url('other')), 'fetch keeps the old address, push uses the new one', flat(r3.stdout).slice(0, 300))
  await rec.step('Send (goes to the push address "other")', () => act(page, 'Send'))
  const tipOther = await rec.cli(WORK, ['clone', s.url('other'), join(WORK, 'S-07-clone')], { env: NOKEY })
  rec.check(tipOther.code === 0 && existsSync(join(WORK, 'S-07-clone', 'art', 'banner.ai')), 'the version arrived in "other", the push address', flat(tipOther.stdout + tipOther.stderr).slice(-200))
  rmSync(join(WORK, 'S-07-clone'), { recursive: true, force: true })
  await rec.step('Remove…: cancel, then confirm', async () => {
    await rowMenu(page, 'studio', /Remove/); const rmDlg = await dialogText(page); rec.note(`remove dialog: ${rmDlg}`)
    rec.check(/sign-in for that server is kept/i.test(rmDlg) && /Sign out/i.test(rmDlg), 'BUG-034: the Remove dialog says the saved sign-in is kept and where to remove it (Sign out in Security)', rmDlg.slice(0, 300))
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500)
    await rowMenu(page, 'studio', /Remove/); await page.getByRole('alertdialog').getByRole('button', { name: 'Remove remote' }).click(); await page.waitForTimeout(2500)
  })
  const r4 = await k.cli('remote', 'list')
  rec.check(!r4.stdout.includes('studio'), 'the remote is removed', flat(r4.stdout).slice(0, 100))
  const left = keychain().filter((e) => !before.includes(e))
  rec.note(`keychain entries left after the remote was removed: ${JSON.stringify(left)}`)
  // Ruling 2026-11-18: one keychain sign-in per server, shared by projects; Remove keeps it. Sign out (auth logout) removes it.
  rec.check(left.length >= 1, 'BUG-034 (ruling): removing the remote keeps the shared per-server sign-in; the form and dialog now say so', JSON.stringify(left))
  for (const e of left) { const t = e.replace(/^.*?target:\s*/i, '').replace(/^[A-Za-z]+:target=/i, ''); try { execFileSync('cmdkey', [`/delete:${e.split(/target:\s*/i)[1] ?? t}`]); rec.note(`harness removed the test keychain entry ${t}`) } catch (x) { rec.note(`could not remove the keychain entry ${e}: ${String(x).slice(0, 120)}`) } }
  rec.note(`keychain at the end: ${JSON.stringify(keychain().filter((e) => !before.includes(e)))}`)
  await rec.finish()
  await closeTerm()
})

/** Two clients of one server repository, both at the same first version. */
async function pair(rec: Recorder, name: string, repoName: string) {
  const s = await server(repoName)
  const a = kit(rec, workDir(`${name}-a`))
  await a.init(); a.put('art/banner.ai', { from: banner(1) }); a.put('notes.txt', 'base\n'); const v1 = await a.save('v1 base')
  await a.cli('remote', 'add', 'origin', s.url(repoName)); const p = await a.cli('push', '-u', 'origin', 'main', '--force')
  if (p.code !== 0) rec.note(`setup push failed: ${flat(p.stdout + p.stderr).slice(-200)}`)
  const bDir = join(WORK, `${name}-b`); rmSync(bDir, { recursive: true, force: true })
  await rec.cli(WORK, ['clone', s.url(repoName), bDir], { quietFrame: true, env: NOKEY })
  const b = kit(rec, bDir)
  const remoteTip = async () => { const d = join(WORK, `${name}-probe`); rmSync(d, { recursive: true, force: true }); await rec.cli(WORK, ['clone', s.url(repoName), d], { quietFrame: true, env: NOKEY }); const t = (await kit(rec, d).log())[0]; rmSync(d, { recursive: true, force: true }); return t }
  return { s, a, b, v1, remoteTip }
}

test('S-09 Diverged history: refused, overwrite safely, lease protects, overwrite anyway', async () => {
  const rec = await Recorder.start('S-09', 'force-with-lease', { gui: true })
  const page = rec.page!
  const { a, b, remoteTip } = await pair(rec, 'S-09', 'lease')
  b.put('art/photo.jpeg', { from: P1 }); const b1 = await b.save('b1: ben adds a photo'); await b.cli('push', 'origin', 'main')
  a.put('art/banner.ai', { from: banner(2) }); const a1 = await a.save('a1: ana changes the banner')
  await rec.step('Open Sync on Ana\'s copy (she has not fetched Ben\'s version)', () => open(page, a.repo, 'branches/sync'))
  rec.note(`where you stand: ${flat(await page.getByTestId('sync-state').innerText().catch(() => ''))}`)
  await rec.step('Send (plain push)', () => act(page, 'Send'))
  const refused = await alerts(page)
  rec.note(`plain push on diverged history: ${refused}`)
  rec.check((await remoteTip())?.oid === b1, 'a plain push is refused; Ben\'s version is still the remote tip')
  rec.check(/fetch|get latest|newer|behind|moved|diverged|non-fast/i.test(refused) && !raw(refused), 'the refusal says the remote has newer work, in plain words', refused)
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  await rec.step('Fetch, so Ana knows about Ben\'s version', async () => { await page.getByRole('button', { name: 'Fetch' }).first().click(); await page.waitForTimeout(1500); await idle(page); await page.waitForTimeout(1500) })
  rec.note(`where you stand after fetch: ${flat(await page.getByTestId('sync-state').innerText().catch(() => ''))}`)
  await rec.step('Options for Send: "Overwrite safely"', async () => { const more = await options(page, 'Send'); await page.getByTestId('push-risk').getByText('Overwrite safely').click(); await page.waitForTimeout(400); rec.note(`hint: ${flat(await page.getByTestId('overwrite-hint').innerText())}`); void more })
  await rec.step('Send, then cancel the question', async () => { await header(page, 'Send').click(); await page.waitForTimeout(1200); rec.note(`overwrite dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click().catch(() => rec.note('no confirmation was asked')); await page.waitForTimeout(800) })
  rec.check((await remoteTip())?.oid === b1, 'cancel sends nothing')
  await rec.step('Send, confirm Overwrite', () => act(page, 'Send', 'Overwrite'))
  rec.note(`after overwrite safely: ${await alerts(page)}`)
  rec.check((await remoteTip())?.oid === a1, 'with the lease the remote tip is now Ana\'s version')
  // The remote moves again without Ana knowing: Ben fetches, resets to a1 and adds b2.
  const rs = await b.loud('reset', '--hard', 'origin/main')
  rec.note(`Ben: fetch, then \`reset --hard origin/main\`: exit ${rs.code} ${flat(rs.stdout + rs.stderr).slice(0, 160)}`)
  rmSync(b.repo, { recursive: true, force: true }); await rec.cli(WORK, ['clone', (await server()).url('lease'), b.repo], { quietFrame: true, env: NOKEY })
  b.put('notes.txt', 'ben again\n'); const b2 = await b.save('b2: ben again'); const pb = await b.loud('push', 'origin', 'main')
  rec.check(pb.code === 0 && (await remoteTip())?.oid === b2, 'setup: Ben (fresh clone) pushed b2', flat(pb.stdout + pb.stderr).slice(-200))
  await a.cli('reset', '--hard', 'HEAD~1'); a.put('art/banner.ai', { from: banner(3) }); const a2 = await a.save('a2: ana rewrites her version')
  await rec.step('Ana rewrote her version and sends with "Overwrite safely"; she has NOT fetched Ben\'s b2', async () => { await refocus(page); await page.waitForTimeout(1500); await act(page, 'Send', 'Overwrite') })
  const lease = await alerts(page)
  rec.note(`lease with a moved remote: ${lease}`)
  rec.check((await remoteTip())?.oid === b2, 'the lease protects Ben\'s new version: nothing is overwritten', `remote tip ${(await remoteTip())?.summary}`)
  rec.check(lease.length > 4 && !raw(lease), 'the lease refusal is in plain words', lease)
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  await rec.step('"Overwrite anyway" (force)', async () => { await options(page, 'Send'); await page.getByTestId('push-risk').getByText('Overwrite anyway').click(); await page.waitForTimeout(400); rec.note(`hint: ${flat(await page.getByTestId('overwrite-hint').innerText())}`); await header(page, 'Send').click(); await page.waitForTimeout(1200); rec.note(`force dialog: ${await dialogText(page)}`); await rec.shot('force question'); await page.getByRole('alertdialog').getByRole('button', { name: 'Overwrite' }).click(); await page.waitForTimeout(1500); await idle(page); await page.waitForTimeout(1500) })
  rec.check((await remoteTip())?.oid === a2, 'force overwrites: the remote tip is Ana\'s rewritten version')
  await rec.step('Back to "Stop if the remote moved"', async () => { await options(page, 'Send'); await page.getByTestId('push-risk').getByText('Stop if the remote moved').click(); await page.waitForTimeout(400) })
  const twin = await b.loud('push', 'origin', 'main', '--force-with-lease')
  rec.note(`CLI twin: Ben (stale) pushes with --force-with-lease: exit ${twin.code} ${flat(twin.stdout + twin.stderr).slice(-250)}`)
  rec.check(twin.code !== 0 && (await remoteTip())?.oid === a2, 'CLI --force-with-lease from a stale copy is refused too')
  await intCheck(rec, a.repo)
  await intCheck(rec, b.repo)
  await rec.finish()
  await closeTerm()
})

test('S-10 Get latest: fast-forward, merge, rebase, conflict, unsaved changes', async () => {
  const rec = await Recorder.start('S-10', 'pull-variants', { gui: true })
  const page = rec.page!
  const { a, b, v1 } = await pair(rec, 'S-10', 'pull')
  b.put('art/photo.jpeg', { from: P1 }); const b1 = await b.save('b1: photo'); await b.cli('push', 'origin', 'main')
  await rec.step('Open Ana\'s copy; Ben has pushed one version', () => open(page, a.repo, 'branches/sync'))
  await rec.step('Fetch', async () => { await page.getByRole('button', { name: 'Fetch' }).first().click(); await page.waitForTimeout(1500); await idle(page); await page.waitForTimeout(1500) })
  const stand = flat(await page.getByTestId('sync-state').innerText().catch(() => ''))
  rec.note(`where you stand: ${stand} | header: ${flat(await header(page, 'Get latest').innerText())}`)
  rec.check(/1/.test(await header(page, 'Get latest').innerText()), 'the header button shows 1 version to get')
  rec.check(!existsSync(join(a.repo, 'art/photo.jpeg')), 'fetch does not change files')
  await rec.step('Get latest (fast-forward)', () => act(page, 'Get latest'))
  rec.check((await a.log())[0]!.oid === b1 && (await a.hash('art/photo.jpeg')) === (await corpus(P1)), 'fast-forward: Ana has Ben\'s version and the photo by hash', await alerts(page))

  b.put('art/extra.ai', { from: banner(3) }); const b2 = await b.save('b2: extra'); const pb2 = await b.loud('push', 'origin', 'main')
  rec.check(pb2.code === 0, 'setup: Ben pushed b2', flat(pb2.stdout + pb2.stderr).slice(-200))
  a.put('notes.txt', 'ana notes\n'); const a1 = await a.save('a1: notes')
  await rec.step('Both sides have one new version (different files). Get latest', async () => { await refocus(page); await act(page, 'Get latest') })
  let t = (await a.log())[0]!
  if (t.parents.length !== 2) { const twin = await a.loud('pull', 'origin', 'main'); t = (await a.log())[0]!; rec.note(`the app's Get latest made no merge; CLI \`pull origin main\` right after: exit ${twin.code} ${flat(twin.stdout + twin.stderr).slice(-300)}; tip now "${t.summary}" parents ${t.parents.length}`); rec.check(false, 'the app\'s Get latest merges when both sides have new versions') }
  rec.note(`merge pull: tip "${t.summary}" parents ${t.parents.length} | ${await alerts(page)}`)
  rec.check(t.parents.length === 2 && t.parents.includes(a1) && t.parents.includes(b2), 'a merge version joins both')
  rec.check((await a.hash('art/extra.ai')) === (await corpus(banner(3))) && a.read('notes.txt') === 'ana notes\n', 'both changes are on disk')
  await a.cli('push', 'origin', 'main')

  await b.cli('pull', 'origin', 'main'); b.put('art/second.jpeg', { from: P2 }); const b3 = await b.save('b3: second photo'); await b.cli('push', 'origin', 'main')
  a.put('todo.txt', 'ana todo\n'); await a.save('a2: todo')
  await rec.step('Options for Get latest: "Replay my versions on top" (--rebase), then Get latest', async () => { await refocus(page); const more = await options(page, 'Get latest'); await more.getByText('Replay my versions on top').click(); await page.waitForTimeout(300); await rec.shot('rebase ticked'); await act(page, 'Get latest') })
  const l = await a.log()
  rec.note(`rebase pull: ${l.slice(0, 3).map((c) => `${c.summary}(${c.parents.length})`).join(' < ')} | ${await alerts(page)} | state ${a.state().join(',')}`)
  rec.check(l[0]!.summary === 'a2: todo' && l[0]!.parents.length === 1 && l[0]!.parents[0] === b3, 'with --rebase my version sits on top of the remote tip, no merge version')
  rec.check(a.state().length === 0, 'a clean rebase pull leaves no state behind', a.state().join(','))
  await rec.step('Untick the rebase option', async () => { const more = await options(page, 'Get latest'); await more.getByText('Replay my versions on top').click(); await page.waitForTimeout(300) })
  await a.cli('push', 'origin', 'main')

  await b.cli('pull', 'origin', 'main'); b.put('notes.txt', 'ben notes\n'); await b.save('b4: ben notes'); await b.cli('push', 'origin', 'main')
  a.put('scratch.txt', 'unsaved new file\n'); a.put('notes.txt', 'ana unsaved edit\n')
  const mineHash = await a.hash('notes.txt')
  await rec.step('Ana has an UNSAVED edit of notes.txt; Ben changed it too. Get latest', async () => { await refocus(page); await act(page, 'Get latest') })
  const dirty = await alerts(page)
  rec.note(`pull over unsaved changes: ${dirty}`)
  rec.check((await a.hash('notes.txt')) === mineHash && existsSync(join(a.repo, 'scratch.txt')), 'my unsaved work is untouched')
  rec.check(/notes\.txt|unsaved|changes|save/i.test(dirty) && !raw(dirty), 'the refusal names the problem in plain words', dirty)
  await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  await a.save('a3: ana notes (clashes with Ben)')
  const before = (await a.log())[0]!.oid
  await rec.step('Now saved: both changed notes.txt. Get latest (conflict)', async () => { await refocus(page); await act(page, 'Get latest') })
  const conf = await alerts(page)
  const st = await a.cli('status', '--machine')
  rec.note(`conflict pull: ${conf} | route ${await page.evaluate(() => window.location.hash)} | notes ${JSON.stringify(a.read('notes.txt').slice(0, 120))} | state ${a.state().join(',')} | status lines ${st.stdout.split('\n').filter((x) => x.trim()).length}`)
  rec.check((await a.log())[0]!.oid === before, 'no version is made while the clash is open')
  rec.check(a.state().includes('MERGE_HEAD'), 'a merge is waiting')
  await rec.step('Open Branches > Merge', async () => { await go(page, 'branches/merge'); await page.waitForTimeout(2500) })
  const rows = await page.getByLabel('Conflicting files').locator('li').allInnerTexts().then((x) => x.map(flat)).catch(() => [] as string[])
  rec.check(rows.length === 1 && rows[0]!.includes('notes.txt'), 'the Merge section lists the clash from the pull', `${JSON.stringify(rows)} | ${flat(await page.locator('#app').innerText()).slice(-200)}`)
  await rec.step('Keep theirs, Finish merge', async () => { await page.getByRole('group', { name: 'Choose a version of notes.txt' }).getByRole('button', { name: /^Keep / }).nth(1).click(); await page.waitForTimeout(2000); await page.getByRole('button', { name: 'Finish merge' }).first().click(); await page.waitForTimeout(5000) })
  t = (await a.log())[0]!
  rec.check(t.parents.length === 2 && a.read('notes.txt') === 'ben notes\n', 'the pull merge finishes with Ben\'s notes', `${t.summary} | ${JSON.stringify(a.read('notes.txt'))}`)
  rec.check(existsSync(join(a.repo, 'scratch.txt')), 'the unsaved new file survived the whole pull')
  void v1
  await intCheck(rec, a.repo)
  await rec.finish()
  await closeTerm()
})

test('S-11 Locks', async () => {
  const rec = await Recorder.start('S-11', 'locks', { gui: true })
  const page = rec.page!
  const { a, b } = await pair(rec, 'S-11', 'locks')
  a.put('art/photo.jpeg', { from: P1 }); await a.save('photo'); await a.cli('push', 'origin', 'main'); await b.cli('pull', 'origin', 'main')
  const dlg = page.getByRole('dialog')
  const table = () => page.locator('tr[data-lock]').allInnerTexts().then((t) => t.map((r) => flat(r).replace(/\t+/g, ' | ')))
  const cliLocks = async () => { const r = await a.cli('lock', 'list', '--machine'); return flat(r.stdout + r.stderr).slice(0, 500) }
  await rec.step('Open Team > Locks', () => open(page, a.repo, 'team/locks'))
  rec.check(/Nothing is locked/.test(await page.locator('#app').innerText()), 'empty state: "Nothing is locked"', await alerts(page))
  await rec.step('Lock a file: art/banner.ai', async () => { await page.getByRole('button', { name: 'Lock a file' }).first().click(); await page.waitForTimeout(500); rec.note(`lock dialog: ${await dialogText(page)}`); await dlg.getByLabel('File path').fill('art/banner.ai'); await page.waitForTimeout(300); await dlg.getByRole('button', { name: 'Lock file' }).click(); await page.waitForTimeout(2500) })
  let rows = await table()
  rec.note(`rows: ${JSON.stringify(rows)} | alerts ${await alerts(page)} | cli: ${await cliLocks()}`)
  if (!rows.length) {
    await rec.step('The server has no sign-in: set "Lock as" to ana', async () => { if (!(await dlg.isVisible().catch(() => false))) { await page.getByRole('button', { name: 'Lock a file' }).first().click(); await page.waitForTimeout(500); await dlg.getByLabel('File path').fill('art/banner.ai') } const more = page.getByTestId('lock-more'); await more.locator('summary, button').first().click(); await page.waitForTimeout(300); await more.getByLabel('Lock as').fill('ana'); await dlg.getByRole('button', { name: 'Lock file' }).click(); await page.waitForTimeout(2500) })
    rows = await table()
    rec.note(`rows after "Lock as": ${JSON.stringify(rows)} | alerts ${await alerts(page)}`)
  }
  rec.check(rows.length === 1 && rows[0]!.includes('art/banner.ai'), 'the lock is listed', JSON.stringify(rows))
  rec.check(/You/.test(rows[0] ?? ''), 'my own lock carries the "You" chip', rows[0])
  const other = await b.loud('lock', 'create', 'art/photo.jpeg', '--owner', 'ben')
  rec.note(`Ben locks photo.jpeg by CLI: exit ${other.code} ${flat(other.stdout + other.stderr).slice(0, 200)}`)
  const same = await b.loud('lock', 'create', 'art/banner.ai', '--owner', 'ben')
  rec.check(same.code !== 0, 'Ben cannot lock the file Ana holds', flat(same.stdout + same.stderr).slice(0, 250))
  rec.check(/ana|locked|held|already/i.test(same.stdout + same.stderr), 'the refusal names the holder', flat(same.stdout + same.stderr).slice(0, 250))
  await rec.step('Refocus: Ben\'s lock shows', async () => { await refocus(page); await page.waitForTimeout(3000) })
  rows = await table()
  rec.check(rows.length === 2 && rows.some((r) => r.includes('art/photo.jpeg') && r.includes('ben')), 'both locks are listed with their holders', JSON.stringify(rows))
  for (const f of ['Mine', 'Others', 'All'] as const) { await rec.step(`Whose locks: ${f}`, async () => { await page.getByLabel('Whose locks').getByText(f, { exact: true }).click(); await page.waitForTimeout(700) }); const r = await table(); rec.check(f === 'All' ? r.length === 2 : r.length === 1 && r[0]!.includes(f === 'Mine' ? 'banner.ai' : 'photo.jpeg'), `filter "${f}" shows the right rows`, JSON.stringify(r)) }
  rec.note(`cli lock list: ${await cliLocks()}`)
  await rec.step('Files > Changes after Ana edits the file Ben locked', async () => { a.put('art/photo.jpeg', { from: P2 }); await go(page, 'files/changes'); await refocus(page); await page.waitForTimeout(3000) })
  const ch = flat(await page.locator('#app').innerText())
  rec.note(`Changes with a file locked by Ben: ${ch.slice(-400)}`)
  rec.check(/lock|ben/i.test(ch), 'Changes warns that photo.jpeg is locked by Ben', ch.slice(-250))
  await a.cli('add', '--all'); await a.cli('commit', '-m', 'ana edits a file ben locked')
  const pushLocked = await a.loud('push', 'origin', 'main')
  rec.note(`push of a change to a file locked by someone else: exit ${pushLocked.code} ${flat(pushLocked.stdout + pushLocked.stderr).slice(-250)}`)
  rec.check(pushLocked.code !== 0, 'the server refuses a push that changes a file locked by another person')
  await rec.step('Locks: Force unlock… Ben\'s lock, cancel then confirm', async () => {
    await go(page, 'team/locks'); await page.waitForTimeout(2500)
    await page.locator('tr[data-lock="art/photo.jpeg"]').getByRole('button', { name: 'Force unlock…' }).click(); await page.waitForTimeout(500); rec.note(`force dialog: ${await dialogText(page)}`)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500)
    await page.locator('tr[data-lock="art/photo.jpeg"]').getByRole('button', { name: 'Force unlock…' }).click(); await page.waitForTimeout(500)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Force unlock' }).click(); await page.waitForTimeout(2500)
  })
  rows = await table()
  rec.note(`after force unlock: ${JSON.stringify(rows)} | ${await alerts(page)}`)
  rec.check(!rows.some((r) => r.includes('photo.jpeg')), 'Ben\'s lock is gone after a confirmed force unlock (no-sign-in server: anyone may)')
  await rec.step('Unlock my own lock', async () => { await page.locator('tr[data-lock="art/banner.ai"]').getByRole('button', { name: 'Unlock' }).click(); await page.waitForTimeout(2500) })
  rec.check((await table()).length === 0, 'my lock is released')
  await rec.step('Lock a path that does not exist, and a path outside the project', async () => {
    for (const p of ['art/no-such-file.psd', '../outside.txt', 'C:\\Windows\\win.ini', '']) { await page.getByRole('button', { name: 'Lock a file' }).first().click().catch(() => {}); await page.waitForTimeout(400); await dlg.getByLabel('File path').fill(p); await page.waitForTimeout(350); const btn = dlg.getByRole('button', { name: 'Lock file' }); const off = await btn.isDisabled(); if (!off) { await btn.click(); await page.waitForTimeout(2000) } rec.note(`lock "${p}": submit ${off ? 'off' : 'pressed'} | ${(await dialogText(page)).slice(0, 200)} | ${await alerts(page)} | rows ${JSON.stringify(await table())}`); await page.keyboard.press('Escape'); await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {}); await page.waitForTimeout(300) }
  })
  const bad = await table()
  rec.check(!bad.some((r) => /outside|win\.ini/.test(r)), 'a path outside the project cannot be locked', JSON.stringify(bad))
  rec.note(`a missing path ${bad.some((r) => r.includes('no-such-file')) ? 'CAN be locked (reserving a name before the file exists)' : 'cannot be locked'}`)
  for (const r of bad) { const p = r.split(' | ')[0]!.trim(); await a.cli('lock', 'unlock', p, '--force') }
  const p2 = await a.loud('push', 'origin', 'main')
  rec.check(p2.code === 0, 'with the lock gone the push goes through', flat(p2.stdout + p2.stderr).slice(-200))
  await intCheck(rec, a.repo)
  await rec.finish()
  await closeTerm()
})

test('S-12 Remote files: browse, preview, download a small and a large file', async () => {
  const rec = await Recorder.start('S-12', 'remote-files', { gui: true })
  const page = rec.page!
  const s = await server('files', 'bigfiles')
  const k = kit(rec, workDir('S-12'))
  await k.init()
  k.put('art/banner.ai', { from: banner(1) }); k.put('art/photo.jpeg', { from: P1 }); k.put('art/deep/er/second.jpeg', { from: P2 }); k.put('video/clip.mp4', { from: '101394-video-720.mp4' }); k.put('notes.txt', 'hello\n')
  await k.save('five files'); await k.cli('remote', 'add', 'origin', s.url('files')); await k.cli('push', '-u', 'origin', 'main')
  await k.cli('branch', 'create', 'look-dev'); await k.cli('branch', 'switch', 'look-dev'); k.put('art/look.ai', { from: banner(4) }); await k.save('look-dev only'); await k.cli('push', 'origin', 'look-dev'); await k.cli('branch', 'switch', 'main')
  const out = join(WORK, 'S-12-dl'); rmSync(out, { recursive: true, force: true }); mkdirSync(out, { recursive: true })
  const rows = () => page.getByLabel('Remote folders and files').locator('button.row').evaluateAll((els) => els.map((e) => (e as HTMLElement).innerText.replace(/\s+/g, ' ').trim()))
  const pane = page.getByTestId('remote-pane')
  const waitDl = async () => { await page.waitForTimeout(800); await page.waitForFunction(() => !document.querySelector('[data-testid="download-progress"]'), null, { timeout: 900_000 }).catch(() => {}); await page.waitForTimeout(1500) }
  await rec.step('Open Team > Remote files', () => open(page, k.repo, 'team/remote'))
  let r = await rows()
  const ls = await k.loud('remote', 'ls-tree', 'origin')
  rec.note(`rows: ${JSON.stringify(r)} | cli remote ls-tree: exit ${ls.code} ${flat(ls.stdout + ls.stderr).slice(0, 300)}`)
  rec.check(r.length === 3 && r.some((x) => x.startsWith('art')) && r.some((x) => x.startsWith('video')) && r.some((x) => x.startsWith('notes.txt')), 'the root lists art, video and notes.txt', JSON.stringify(r))
  await rec.step('Open folder art, then deep, then er', async () => { for (const d of ['art', 'deep', 'er']) { await page.getByRole('button', { name: `Open folder ${d}` }).click(); await page.waitForTimeout(1500) } })
  r = await rows()
  rec.check(r.length === 1 && r[0]!.startsWith('second.jpeg'), 'three levels down shows second.jpeg', JSON.stringify(r))
  rec.note(`breadcrumbs: ${flat(await page.getByLabel('Folder path').innerText())}`)
  await rec.step('Pick second.jpeg (preview)', async () => { await page.getByLabel('Remote folders and files').getByRole('button', { name: 'second.jpeg' }).click(); await page.waitForTimeout(5000) })
  const img = await pane.locator('img').evaluateAll((els) => (els as HTMLImageElement[]).map((i) => `${i.naturalWidth}x${i.naturalHeight}`))
  rec.note(`pane: ${flat(await pane.innerText()).slice(0, 300)} | image ${JSON.stringify(img)}`)
  rec.check(img.length === 1 && img[0] !== '0x0', 'the remote picture previews without a download step', JSON.stringify(img))
  await rec.step('Download to… cancelled picker', async () => { await answerPicker(page, null); await pane.getByRole('button', { name: 'Download to…' }).click(); await page.waitForTimeout(1500) })
  rec.check(readdirSync(out).length === 0 && !raw(await alerts(page)), 'a cancelled picker downloads nothing and shows no error', await alerts(page))
  await rec.step('Download to… work\\S-12-dl', async () => { await answerPicker(page, out); await pane.getByRole('button', { name: 'Download to…' }).click(); await waitDl() })
  const got = walk(out, () => false)
  rec.note(`downloaded: ${got.map((f) => f.slice(out.length + 1)).join(', ')} | ${await alerts(page)}`)
  rec.check(got.length === 1 && (await sha256(got[0]!)) === (await corpus(P2)), 'the downloaded file equals the original by hash')
  await rec.step('Download the same file again into the same folder', async () => { await pane.getByRole('button', { name: 'Download to…' }).click(); await waitDl() })
  rec.note(`second download to the same place: ${walk(out, () => false).map((f) => f.slice(out.length + 1)).join(', ')} | ${await alerts(page)}`)
  await rec.step('Breadcrumb back to the top, pick video/clip.mp4', async () => { await page.getByLabel('Folder path').getByRole('button').first().click(); await page.waitForTimeout(1500); await page.getByRole('button', { name: 'Open folder video' }).click(); await page.waitForTimeout(1500); await page.getByLabel('Remote folders and files').getByRole('button', { name: 'clip.mp4' }).click(); await page.waitForTimeout(6000) })
  const vid = await pane.locator('video').evaluateAll((els) => (els as HTMLVideoElement[]).map((v) => `${v.videoWidth}x${v.videoHeight} ${Math.round(v.duration)}s ready=${v.readyState}`))
  rec.note(`video preview: ${JSON.stringify(vid)} | ${flat(await pane.innerText()).slice(0, 200)}`)
  rec.check(vid.length === 1 && !vid[0]!.startsWith('0x0'), 'the remote video previews', JSON.stringify(vid))
  await rec.step('Version picker: look-dev', async () => { await page.locator('.field').filter({ has: page.locator('label', { hasText: /^Version$/ }) }).locator('.trigger').click(); await page.waitForTimeout(400); rec.note(`version options: ${JSON.stringify(await page.getByRole('option').allInnerTexts())}`); await page.getByRole('option', { name: /look-dev/ }).first().click(); await page.waitForTimeout(2500); await page.getByLabel('Folder path').getByRole('button').first().click().catch(() => {}); await page.waitForTimeout(1200); await page.getByRole('button', { name: 'Open folder art' }).click().catch(() => {}); await page.waitForTimeout(1500) })
  r = await rows()
  rec.check(r.some((x) => x.startsWith('look.ai')), 'on look-dev the file that only exists there is listed', JSON.stringify(r))
  const dl = await rec.cli(out, ['download', 'art/photo.jpeg', '--remote', 'origin', '-C', k.repo], { env: NOKEY })
  rec.note(`CLI download twin: exit ${dl.code} ${flat(dl.stdout + dl.stderr).slice(0, 300)} | files now ${walk(out, () => false).concat(walk(k.repo).filter((f) => /photo\.jpeg$/.test(f))).map((f) => f.split(/[\\/]/).slice(-2).join('/')).join(', ')}`)

  const big = ensureBig((s) => rec.note(s))
  const bigFiles = existsSync(join(big, '.mediagit')) ? walk(big).map((f) => ({ f, n: statSync(f).size })).filter((x) => x.n > 1e9).sort((x, y) => x.n - y.n) : []
  if (!bigFiles.length) rec.note('no repository with files over 1 GB under work/big: the large download is NOT tested')
  else {
    const pick = bigFiles[0]!
    const rel = pick.f.slice(big.length + 1).replace(/\\/g, '/')
    const kb = kit(rec, big)
    await kb.cli('remote', 'remove', 'qa12'); await kb.cli('remote', 'add', 'qa12', s.url('bigfiles'))
    const t0 = Date.now(); const pb = await kb.loud('push', 'qa12', 'main'); rec.note(`push of the big repository for this case: exit ${pb.code}, ${Date.now() - t0} ms, peak ${pb.peakMb} MB`)
    await rec.step(`Open the big repository, Remote files, pick ${rel} (${(pick.n / 1e9).toFixed(2)} GB)`, async () => {
      await open(page, big, 'team/remote')
      const sel = page.locator('.field').filter({ has: page.locator('label', { hasText: /^Remote$/ }) }).locator('.trigger')
      if (!(await sel.innerText()).includes('qa12')) { await sel.click(); await page.waitForTimeout(400); await page.getByRole('option', { name: 'qa12' }).click(); await page.waitForTimeout(2500) }
      const parts = rel.split('/')
      for (const d of parts.slice(0, -1)) { await page.getByRole('button', { name: `Open folder ${d}` }).click(); await page.waitForTimeout(1500) }
      await page.getByLabel('Remote folders and files').getByRole('button', { name: parts[parts.length - 1]!, exact: true }).click(); await page.waitForTimeout(4000)
    })
    rec.note(`pane for the large file: ${flat(await pane.innerText()).slice(0, 300)} | ${await alerts(page)}`)
    const samples: string[] = []
    const t1 = Date.now()
    await rec.step('Download to… (large file), watching the progress', async () => {
      await answerPicker(page, out); await pane.getByRole('button', { name: 'Download to…' }).click()
      for (let i = 0; i < 600; i++) { await page.waitForTimeout(500); const p = page.getByTestId('download-progress'); if (!(await p.isVisible().catch(() => false))) { if (i > 3) break; continue } const bar = await p.locator('[role="progressbar"]').getAttribute('aria-valuenow').catch(() => null); const txt = flat(await p.innerText().catch(() => '')).slice(0, 80); if (samples.length < 400) samples.push(`${Date.now() - t1}:${bar}:${txt}`); if (samples.length === 6) await rec.shot('download in progress') }
      await page.waitForTimeout(2000)
    })
    const ms = Date.now() - t1
    const bars = samples.map((x) => Number(x.split(':')[1])).filter((n) => !Number.isNaN(n))
    rec.note(`large download: ${ms} ms, ${(pick.n / 1e6 / (ms / 1000)).toFixed(0)} MB/s, ${samples.length} progress samples, bar values ${[...new Set(bars)].slice(0, 12).join(',')}… | first ${samples[0]} | last ${samples[samples.length - 1]}`)
    rec.check(new Set(bars).size >= 3, 'the progress bar moves through real values during the large download', `${new Set(bars).size} distinct values`)
    rec.check(bars.every((v, i) => i === 0 || v >= bars[i - 1]!), 'progress never goes backwards')
    const file = walk(out, () => false).find((f) => f.endsWith(rel.split('/').pop()!))
    rec.check(!!file && statSync(file).size === pick.n, 'the large file arrived with the right size', file ? String(statSync(file).size) : 'missing')
    if (file) rec.check((await sha256(file)) === (await sha256(pick.f)), 'the large download equals the original by hash')
    rec.note(`alerts after the large download: ${await alerts(page)}`)
    await kb.cli('remote', 'remove', 'qa12')
  }
  rmSync(out, { recursive: true, force: true })
  await intCheck(rec, k.repo)
  s.stop(); srv = null
  rmSync(join(WORK, 'S-07-srv', 'storage', 'bigfiles'), { recursive: true, force: true }); rmSync(join(WORK, 'S-07-srv', 'repos', 'bigfiles'), { recursive: true, force: true })
  await rec.finish()
  await closeTerm()
})
