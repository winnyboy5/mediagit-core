import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS, WORK, pidsOf } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { answerPicker, choose, dialogText, flat, go, openRepo, store, toWelcome, visibleErrors } from './gui'
import { intCheck } from './integrity'

// S-01..S-06: a local mediagit-server run from the app (Team > Server), then people, access, keys, repositories.
// The server lives in work/S-01-srv and is left running between cases. S-06 stops it at the end.
// The harness starts the app with MEDIAGIT_DESKTOP_SERVER_DIR, so the folder is fixed to work/app-server.
// Choosing another folder is therefore NOT tested: without that variable the app would use the real default folder.
const SRV = join(WORK, 'app-server')
const client = workDir('S-cli')
const ADMIN = { user: 'qaadmin', email: 'qaadmin@example.test', pass: 'Qa-Live!2026-admin' }
const ARTIST = { user: 'artist1', email: 'artist1@example.test', pass: 'Qa-Live!2026-artist' }
const LEAD = { user: 'lead1', email: 'lead1@example.test', pass: 'Qa-Live!2026-lead' }
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }

const tab = async (page: Page, name: string) => { await page.getByLabel('Server pages').getByRole('button', { name, exact: true }).click(); await page.waitForTimeout(1500) }
const alerts = async (page: Page) => JSON.stringify(await visibleErrors(page)).slice(0, 500)
const raw = (s: string) => /os error|panicked|exit code|stderr|\bErr\(|protocol|unexpected output|status code|HTTP \d{3}|\b40[0-9]\b|\b50[0-9]\b/i.test(s)
const local = (page: Page) => store<{ state: string; url: string | null; port: number | null; lan: boolean } | null>(page, 'server', 's.local')
const up = async (base: string) => { try { return (await fetch(`${base}/health`)).ok } catch { return false } }
async function base(page: Page): Promise<string> { const l = await local(page); if (!l?.url) throw new Error('the local server has no address'); return l.url.replace(/\/$/, '') }
/** Sign in over HTTP. Returns the token, or the refusal text. */
async function login(b: string, who: { user: string; pass: string }): Promise<{ token?: string; status: number; body: string }> {
  for (const body of [{ username: who.user, password: who.pass }, { identifier: who.user, password: who.pass }, { email: who.user, password: who.pass }]) {
    const r = await fetch(`${b}/auth/login`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
    const text = await r.text()
    if (r.status === 422 || r.status === 400) continue
    let token: string | undefined
    try { const j = JSON.parse(text) as Record<string, unknown>; token = ((j.tokens as Record<string, unknown> | undefined)?.access_token ?? j.access_token ?? j.token) as string | undefined } catch { /* not JSON */ }
    return { token, status: r.status, body: text.slice(0, 300) }
  }
  return { status: 0, body: 'no login body shape was accepted' }
}
const as = (token: string) => ({ ...NOKEY, MEDIAGIT_TOKEN: token })
async function openServer(page: Page) {
  if (existsSync(join(client, '.mediagit'))) { await toWelcome(page); await openRepo(page, client) }
  await go(page, 'team/server')
  await page.waitForTimeout(2500)
}
const table = (page: Page, id: string) => page.getByTestId(id).locator('tbody tr').allInnerTexts().then((t) => t.map((r) => flat(r).replace(/\t+/g, ' | '))).catch(() => [] as string[])
export async function signInApp(page: Page, who: { user: string; pass: string }) {
  const b = page.getByRole('button', { name: 'Sign in' }).first()
  if (!(await b.isVisible().catch(() => false))) return false
  await b.click(); await page.waitForTimeout(500)
  await page.getByRole('dialog').getByLabel('Username or email').fill(who.user)
  await page.getByRole('dialog').getByLabel('Password').fill(who.pass)
  await page.getByRole('dialog').getByRole('button', { name: 'Sign in' }).click()
  await page.waitForTimeout(2500)
  return true
}
const filesWith = (dir: string, needle: string, hits: string[] = []): string[] => {
  for (const n of existsSync(dir) ? readdirSync(dir) : []) {
    const p = join(dir, n)
    const st = statSync(p)
    try {
      if (st.isDirectory()) { if (!/objects|packs|Cache|GPUCache|blob_storage/i.test(n)) filesWith(p, needle, hits) } else if (st.size < 20e6 && readFileSync(p).includes(needle)) hits.push(p)
    } catch { /* a file locked by the running server or app: skipped */ }
  }
  return hits
}

test('S-01 Local server from the app: folder, set up, start, stop', async () => {
  const rec = await Recorder.start('S-01', 'local-server', { gui: true })
  const page = rec.page!
  if (pidsOf('mediagit-server.exe').length === 0) for (const n of readdirSync(SRV)) rmSync(join(SRV, n), { recursive: true, force: true })
  rmSync(client, { recursive: true, force: true }); mkdirSync(join(client, 'art'), { recursive: true })
  await rec.cli(client, ['init'], { quietFrame: true })
  cpSync(join(CORPUS, 'creative-pairs/ai/banner_v1.ai'), join(client, 'art/banner.ai')); cpSync(join(CORPUS, 'freepik__talk__71826.jpeg'), join(client, 'art/photo.jpeg'))
  await rec.cli(client, ['add', '--all'], { quietFrame: true }); await rec.cli(client, ['commit', '-m', 'first art'], { quietFrame: true })
  const before = pidsOf('mediagit-server.exe')
  await rec.step('Open Team > Server', () => openServer(page))
  let l = await local(page)
  rec.note(`on arrival: local=${JSON.stringify(l)} | folder ${await page.getByTestId('folder-path').innerText().catch(() => '?')} | ${flat(await page.locator('#app').innerText()).slice(-500)}`)
  if (l?.state === 'running') { rec.note('a server from an earlier run is up; stopping it first'); await page.getByRole('button', { name: 'Stop server…' }).click(); await page.getByRole('button', { name: 'Stop server', exact: true }).click(); await page.waitForTimeout(3000) }
  const pinned = await page.getByRole('button', { name: 'Choose folder…' }).isDisabled().catch(() => false)
  if (pinned) rec.note('Choose folder… is disabled: the harness pins the server folder, so the picker cases are skipped')
  else {
    await rec.step('Choose folder…: a folder inside a client repository', async () => { await answerPicker(page, join(client, 'art')); await page.getByRole('button', { name: 'Choose folder…' }).click(); await page.waitForTimeout(2000) })
    const inside = `${await page.getByTestId('folder-error').innerText().catch(() => '')} | path ${await page.getByTestId('folder-path').innerText().catch(() => '?')}`
    rec.note(`folder inside a repository: ${inside}`)
    rec.check(!(await page.getByTestId('folder-path').innerText()).includes('S-cli'), 'a folder inside a project is not taken')
    if (!(await page.getByRole('button', { name: 'Choose folder…' }).isDisabled())) await rec.step('Choose folder…: cancel the picker', async () => { const p0 = await page.getByTestId('folder-path').innerText(); await answerPicker(page, null); await page.getByRole('button', { name: 'Choose folder…' }).click(); await page.waitForTimeout(1200); rec.check((await page.getByTestId('folder-path').innerText()) === p0, 'cancel keeps the folder') })
  }
  rec.note('choosing a different server folder is not tested (the harness pins the folder for safety)')
  await page.reload(); await page.waitForTimeout(2500)
  l = await local(page)
  rec.note(`after picking the folder: ${JSON.stringify(l)}`)
  const dlg = page.getByRole('dialog')
  await rec.step('Set up local server…: passwords that differ', async () => {
    await page.getByRole('button', { name: 'Set up local server…' }).click(); await page.waitForTimeout(600)
    rec.note(`setup dialog: ${await dialogText(page)}`)
    await dlg.getByLabel('Admin username').fill(ADMIN.user); await dlg.getByLabel('Admin email').fill(ADMIN.email)
    await dlg.getByLabel('Password', { exact: true }).fill(ADMIN.pass); await dlg.getByLabel('Repeat password').fill('something else'); await page.waitForTimeout(400)
  })
  rec.check(await dlg.getByRole('button', { name: 'Set up and start' }).isDisabled(), 'mismatched passwords cannot be submitted', flat(await dlg.innerText()).slice(-200))
  await rec.step('A weak password: "abc"', async () => { await dlg.getByLabel('Password', { exact: true }).fill('abc'); await dlg.getByLabel('Repeat password').fill('abc'); await page.waitForTimeout(400); rec.note(`weak password: ${flat(await dlg.innerText()).slice(-260)} | submit disabled=${await dlg.getByRole('button', { name: 'Set up and start' }).isDisabled()}`) })
  const t0 = Date.now()
  await rec.step('Good passwords, Set up and start', async () => {
    await dlg.getByLabel('Password', { exact: true }).fill(ADMIN.pass); await dlg.getByLabel('Repeat password').fill(ADMIN.pass); await page.waitForTimeout(300)
    await dlg.getByRole('button', { name: 'Set up and start' }).click()
    await page.getByTestId('local-state').waitFor({ timeout: 60_000 })
    await page.waitForFunction(() => /running|Running/.test((document.querySelector('[data-testid="local-state"]') as HTMLElement | null)?.innerText ?? ''), null, { timeout: 60_000 }).catch(() => {})
  })
  const setupMs = Date.now() - t0
  l = await local(page)
  rec.note(`after set up (${setupMs} ms): ${JSON.stringify(l)} | state text "${await page.getByTestId('local-state').innerText().catch(() => '')}" | alerts ${await alerts(page)}`)
  rec.check(l?.state === 'running', 'the server is running after set up', JSON.stringify(l))
  const b = await base(page)
  rec.check(await up(b), `the address answers /health (${b})`)
  rec.check(/127\.0\.0\.1|localhost/.test(b) && l?.lan === false, 'it listens on this computer only by default', b)
  const mine = pidsOf('mediagit-server.exe').filter((p) => !before.includes(p))
  rec.note(`server process: ${mine.join(',')} | folder now holds: ${readdirSync(SRV).join(', ')}`)
  rec.check(mine.length === 1, 'exactly one new mediagit-server process', String(mine.length))
  const me = flat(await page.getByTestId('me-card').innerText().catch(() => ''))
  rec.note(`account card: ${me}`)
  rec.check(me.includes(ADMIN.user), 'the app is signed in as the new admin right away', me.slice(0, 200))
  const secrets = filesWith(SRV, ADMIN.pass).concat(filesWith(join(WORK, 'logs'), ADMIN.pass))
  rec.check(secrets.length === 0, 'the admin password is in no file of the server folder or the app log', secrets.join(', '))
  const logTxt = flat(await page.getByLabel('Server log').innerText().catch(() => ''))
  rec.note(`server log card: ${logTxt.slice(0, 300)}`)
  await rec.shot('running')

  await rec.step('Stop server…, cancel', async () => { await page.getByRole('button', { name: 'Stop server…' }).click(); await page.waitForTimeout(500); rec.note(`stop dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(600) })
  rec.check(await up(b), 'cancel keeps it running')
  const t1 = Date.now()
  await rec.step('Stop server…, confirm', async () => { await page.getByRole('button', { name: 'Stop server…' }).click(); await page.waitForTimeout(500); await page.getByRole('button', { name: 'Stop server', exact: true }).click(); await page.waitForFunction(() => !!document.querySelector('[data-testid="local-state"].stopped') || /stopped|Stopped|not running/i.test((document.querySelector('[data-testid="local-state"]') as HTMLElement | null)?.innerText ?? ''), null, { timeout: 30_000 }).catch(() => {}) })
  rec.note(`stop took ${Date.now() - t1} ms; state ${JSON.stringify(await local(page))}`)
  rec.check(!(await up(b)), 'after Stop the address no longer answers')
  rec.check(pidsOf('mediagit-server.exe').filter((p) => mine.includes(p)).length === 0, 'the server process is really gone')
  await rec.step('Tick "Let other computers on my network connect", then untick', async () => { const c = page.getByText('Let other computers on my network connect'); await c.click(); await page.waitForTimeout(800); rec.note(`lan ticked: ${JSON.stringify(await local(page))} | ${flat(await page.getByTestId('local-card').innerText()).slice(0, 300)}`); await c.click(); await page.waitForTimeout(800) })
  const t2 = Date.now()
  await rec.step('Start server', async () => { await page.getByRole('button', { name: 'Start server' }).click(); for (let i = 0; i < 100 && !(await up(b)); i++) await page.waitForTimeout(100) })
  const startMs = Date.now() - t2
  rec.note(`start to healthy: ${startMs} ms`)
  rec.check(await up(b), 'it answers again after Start')
  rec.check(startMs <= 3000, 'healthy within 3 s (budget)', `${startMs} ms`)
  await page.waitForTimeout(2000)
  rec.check((await local(page))?.state === 'running', 'the card says running', JSON.stringify(await local(page)))
  const again = flat(await page.getByTestId('me-card').innerText().catch(() => ''))
  rec.check(again.includes(ADMIN.user), 'still signed in after a restart of the server', again.slice(0, 160))
  rec.check((await login(b, ADMIN)).token !== undefined, 'the admin can sign in over HTTP with the password set in the dialog', (await login(b, ADMIN)).body)
  rec.check((await login(b, { user: ADMIN.user, pass: 'wrong-pass' })).token === undefined, 'a wrong password is refused')
  await rec.finish()
  await closeTerm()
})

test('S-06a + S-03 Repositories and people', async () => {
  const rec = await Recorder.start('S-03', 'people', { gui: true })
  const page = rec.page!
  await rec.step('Open Team > Server', () => openServer(page))
  const b = await base(page)
  const admin = (await login(b, ADMIN)).token!
  const dlg = page.getByRole('dialog')
  await rec.step('Repositories: New repository… bad names', async () => {
    await tab(page, 'Repositories'); await page.getByRole('button', { name: 'New repository…' }).click(); await page.waitForTimeout(500)
    for (const n of ['has space', '..', 'a/b', 'CON', 'film.', 'x'.repeat(200)]) { await dlg.getByLabel('Name').fill(n); await page.waitForTimeout(300); rec.note(`repo name "${n.slice(0, 20)}": ${flat(await dlg.innerText()).match(/Name \/ [^/]*/)?.[0]} | create disabled=${await dlg.getByRole('button', { name: 'Create repository' }).isDisabled()}`) }
  })
  for (const odd of ['CON', 'film.', 'a/b']) {
    await rec.step(`Try to create a repository named "${odd}"`, async () => { await dlg.getByLabel('Name').fill(odd); await page.waitForTimeout(300); const btn = dlg.getByRole('button', { name: 'Create repository' }); if (!(await btn.isDisabled())) { await btn.click(); await page.waitForTimeout(2500) } })
    rec.note(`repository "${odd}": dialog ${(await dialogText(page)).slice(0, 260)} | alerts ${await alerts(page)} | table ${JSON.stringify(await table(page, 'repos'))} | on disk ${existsSync(join(SRV, 'repos')) ? readdirSync(join(SRV, 'repos')).join(', ') : readdirSync(SRV).join(', ')}`)
    await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
    if (!(await dlg.isVisible().catch(() => false))) { await page.getByRole('button', { name: 'New repository…' }).click(); await page.waitForTimeout(500) }
  }
  await rec.step('Create repository hero-film', async () => { await dlg.getByLabel('Name').fill('hero-film'); await page.waitForTimeout(300); await dlg.getByRole('button', { name: 'Create repository' }).click(); await page.waitForTimeout(2500) })
  rec.note(`repos table: ${JSON.stringify(await table(page, 'repos'))} | alerts ${await alerts(page)}`)
  rec.check((await table(page, 'repos')).some((r) => r.includes('hero-film')), 'hero-film is listed')
  const rr = await rec.cli(client, ['remote', 'repos', 'list', '--server', b], { env: as(admin) })
  rec.note(`CLI remote repos: exit ${rr.code} ${flat(rr.stdout + rr.stderr).slice(0, 300)}`)
  await rec.step('Create a repository named "film." (BUG-031)', async () => { await page.getByRole('button', { name: 'New repository…' }).click(); await page.waitForTimeout(500); await dlg.getByLabel('Name').fill('film.'); await page.waitForTimeout(400); const btn = dlg.getByRole('button', { name: 'Create repository' }); if (!(await btn.isDisabled())) { await btn.click(); await page.waitForTimeout(2000) } })
  const badName = `${await dialogText(page)} | ${await alerts(page)}`
  rec.check(!raw(badName) && /name|dot|cannot|can't|invalid|letters/i.test(badName) && !(await table(page, 'repos')).some((r) => /\bfilm\./.test(r)), 'BUG-031: a bad repository name ("film.") is refused with a stated reason, never "failed with status 400"', badName.slice(0, 300))
  await page.keyboard.press('Escape'); await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  await rec.step('Create hero-film again (duplicate)', async () => { await page.getByRole('button', { name: 'New repository…' }).click(); await page.waitForTimeout(500); await dlg.getByLabel('Name').fill('hero-film'); await page.waitForTimeout(400); const btn = dlg.getByRole('button', { name: 'Create repository' }); if (!(await btn.isDisabled())) { await btn.click(); await page.waitForTimeout(2000) } })
  const dup = `${await dialogText(page)} | ${await alerts(page)}`
  rec.note(`duplicate repository: ${dup}`)
  rec.check(/exists|taken|already/i.test(dup) && !raw(dup), 'a duplicate repository name is refused in plain words', dup.slice(0, 300))
  await page.keyboard.press('Escape'); await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})

  const addPerson = async (who: typeof ARTIST, level: RegExp) => {
    await page.getByRole('button', { name: 'Add person…' }).click(); await page.waitForTimeout(500)
    await dlg.getByLabel('Username').fill(who.user); await dlg.getByLabel('Email').fill(who.email)
    await choose(page, 'What they can do', level, 300)
    await dlg.getByLabel('Starting password').fill(who.pass); await page.waitForTimeout(300)
    await rec.shot(`add ${who.user}`)
    const addBtn = dlg.getByRole('button', { name: 'Add person' }); if (await addBtn.isEnabled()) await addBtn.click(); else rec.note('Add person is disabled: the form refuses before sending'); await page.waitForTimeout(2500)
  }
  await rec.step('People: add artist1 (can look)', async () => { await tab(page, 'People'); rec.note(`people on arrival: ${JSON.stringify(await table(page, 'people'))}`); await addPerson(ARTIST, /Can look/) })
  await rec.step('Add lead1 (can change files)', () => addPerson(LEAD, /Can change files/))
  let people = await table(page, 'people')
  rec.note(`people: ${JSON.stringify(people)} | alerts ${await alerts(page)}`)
  rec.check(people.length === 3 && [ADMIN, ARTIST, LEAD].every((w) => people.some((r) => r.includes(w.user))), 'three people are listed')
  const lu = await rec.cli(client, ['auth', 'admin', 'list-users', '--server', b], { env: as(admin) })
  rec.note(`CLI list-users: exit ${lu.code} ${flat(lu.stdout + lu.stderr).slice(0, 500)}`)
  rec.check(lu.code === 0 && [ADMIN, ARTIST, LEAD].every((w) => lu.stdout.includes(w.user)), 'the same three are in `auth admin list-users`')
  await rec.step('Add a person with the password "abc"', async () => { await addPerson({ user: 'weakpw', email: 'weakpw@example.test', pass: 'abc' }, /Can look/) })
  const weak = `${await dialogText(page)} | ${await alerts(page)}`
  rec.note(`weak password "abc": ${weak} | account made: ${(await table(page, 'people')).some((r) => r.includes('weakpw'))}`)
  rec.check(!(await table(page, 'people')).some((r) => r.includes('weakpw')), 'a three-letter password is refused')
  rec.check(/at least 8 characters/i.test(weak) && !raw(weak), 'BUG-031: the weak-password text states the rule ("at least 8 characters"), no HTTP status line', weak.slice(0, 300))
  await page.keyboard.press('Escape'); await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  await rec.step('Add artist1 again (duplicate)', async () => { await addPerson({ ...ARTIST, pass: ARTIST.pass }, /Can look/) })
  const dupU = `${await dialogText(page)} | ${await alerts(page)}`
  rec.note(`duplicate person: ${dupU}`)
  rec.check(/exists|taken|already|in use/i.test(dupU) && !raw(dupU), 'a duplicate username is refused in plain words', dupU.slice(0, 300))
  await page.keyboard.press('Escape'); await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  rec.check((await login(b, ARTIST)).token !== undefined, 'artist1 can sign in with the starting password', (await login(b, ARTIST)).body)

  await rec.step('Change role: artist1 to "can change files"', async () => { await page.getByRole('button', { name: `Change what ${ARTIST.user} can do` }).click(); await page.waitForTimeout(500); await choose(page, 'What they can do', /Can change files/, 300); await dlg.getByRole('button', { name: 'Save' }).click(); await page.waitForTimeout(2500) })
  people = await table(page, 'people')
  rec.check(people.some((r) => r.includes(ARTIST.user) && /change files|write/i.test(r)), 'the table shows the new role', JSON.stringify(people))
  const lu2 = await rec.cli(client, ['auth', 'admin', 'list-users', '--server', b], { env: as(admin), quietFrame: true })
  rec.check(new RegExp(`${ARTIST.user}[^\\n]*write`, 'i').test(lu2.stdout), 'the CLI agrees on the role', flat(lu2.stdout).slice(0, 300))
  await rec.step('Disable artist1: cancel, then confirm', async () => {
    await page.getByRole('button', { name: `Stop ${ARTIST.user} signing in` }).click(); await page.waitForTimeout(500); rec.note(`disable dialog: ${await dialogText(page)}`)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500)
    await page.getByRole('button', { name: `Stop ${ARTIST.user} signing in` }).click(); await page.waitForTimeout(500)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Disable' }).click(); await page.waitForTimeout(2500)
  })
  const off = await login(b, ARTIST)
  rec.note(`disabled sign-in: status ${off.status} ${off.body} | row ${JSON.stringify((await table(page, 'people')).filter((r) => r.includes(ARTIST.user)))}`)
  rec.check(off.token === undefined, 'a disabled person cannot sign in')
  const wh = await rec.cli(client, ['auth', 'login', '--server', b, '--username', ARTIST.user], { env: NOKEY, input: `${ARTIST.pass}\n` })
  rec.note(`CLI login as the disabled person: exit ${wh.code} ${flat(wh.stdout + wh.stderr).slice(0, 300)}`)
  rec.check(wh.code !== 0 && /disabled|suspended|not allowed|turned off/i.test(wh.stdout + wh.stderr), 'the CLI refusal says the account is disabled, in plain words', flat(wh.stdout + wh.stderr).slice(0, 200))
  await rec.step('Enable artist1', async () => { await page.getByRole('button', { name: `Let ${ARTIST.user} sign in again` }).click(); await page.waitForTimeout(2500) })
  rec.check((await login(b, ARTIST)).token !== undefined, 'after Enable the person can sign in again')
  const NEW = 'Qa-Live!2026-artist-new'
  await rec.step('Reset password for artist1', async () => { await page.getByRole('button', { name: `Reset password for ${ARTIST.user}` }).click(); await page.waitForTimeout(500); rec.note(`reset dialog: ${await dialogText(page)}`); await dlg.getByLabel('New password').fill(NEW); await dlg.getByRole('button', { name: 'Reset password' }).click(); await page.waitForTimeout(2500) })
  rec.check((await login(b, ARTIST)).token === undefined, 'the old password stops working')
  rec.check((await login(b, { user: ARTIST.user, pass: NEW })).token !== undefined, 'the new password works')
  await rec.step('Reset it back, so later cases know the password', async () => { await page.getByRole('button', { name: `Reset password for ${ARTIST.user}` }).click(); await page.waitForTimeout(500); await dlg.getByLabel('New password').fill(ARTIST.pass); await dlg.getByRole('button', { name: 'Reset password' }).click(); await page.waitForTimeout(2500) })
  await rec.step('Try to disable myself (the only admin)', async () => { const me = page.getByRole('button', { name: `Stop ${ADMIN.user} signing in` }); rec.note(`disable-self button: visible=${await me.isVisible().catch(() => false)} disabled=${await me.isDisabled().catch(() => 'n/a')}`); if (await me.isEnabled().catch(() => false)) { await me.click(); await page.waitForTimeout(500); rec.note(`self-disable dialog: ${await dialogText(page)}`); await page.getByRole('alertdialog').getByRole('button', { name: 'Disable' }).click(); await page.waitForTimeout(2500) } })
  const self = await login(b, ADMIN)
  rec.note(`after trying to disable the only admin: sign-in ${self.token ? 'works' : `REFUSED ${self.body}`} | alerts ${await alerts(page)}`)
  rec.check(self.token !== undefined, 'the only admin cannot lock themselves out')
  await rec.finish()
  await closeTerm()
})

test('S-04 Access: grant, try a push as read-only, revoke', async () => {
  const rec = await Recorder.start('S-04', 'access', { gui: true })
  const page = rec.page!
  await rec.step('Open Team > Server > Access', async () => { await openServer(page); await tab(page, 'Access') })
  const b = await base(page)
  const admin = (await login(b, ADMIN)).token!
  const artist = (await login(b, ARTIST)).token!
  rec.check(!!artist, 'setup: artist1 can sign in')
  await rec.cli(client, ['remote', 'remove', 'origin'], { quietFrame: true })
  await rec.cli(client, ['remote', 'add', 'origin', `${b}/hero-film`], { quietFrame: true })
  const push = async (token: string, label: string) => { const r = await rec.cli(client, ['push', '-u', 'origin', 'main'], { env: as(token) }); rec.note(`${label}: exit ${r.code} ${flat(r.stdout + r.stderr).slice(-300)}`); return r }
  let r = await push(artist, 'push as artist1 with no grant (server role: write)')
  const noGrant = r.code === 0
  rec.note(noGrant ? 'a person with server role "write" and no grant on the repository CAN push' : 'a person with no grant is refused')
  rec.note(`access page: ${flat(await page.locator('#app').innerText()).slice(-500)}`)
  const give = async (level: RegExp) => { await choose(page, 'Person', ARTIST.user, 400); await choose(page, 'Repository', 'hero-film', 400); await choose(page, 'What they can do here', level, 400); rec.note(`current: ${await page.getByTestId('current-level').innerText().catch(() => '')}`); await page.getByRole('button', { name: 'Give access' }).click(); await page.waitForTimeout(2500) }
  await rec.step('Give artist1 "can look" on hero-film', () => give(/Can look/))
  rec.note(`user grants: ${flat(await page.getByTestId('user-grants').innerText().catch(() => ''))} | repo access: ${flat(await page.getByTestId('repo-access').innerText().catch(() => ''))} | alerts ${await alerts(page)}`)
  const lg = await rec.cli(client, ['auth', 'admin', 'list-grants', ARTIST.user, '--server', b], { env: as(admin) })
  rec.check(lg.code === 0 && /hero-film[\s\S]*read|read[\s\S]*hero-film/i.test(lg.stdout), '`auth admin list-grants` shows read on hero-film', flat(lg.stdout + lg.stderr).slice(0, 300))
  writeFileSync(join(client, 'notes.txt'), `note ${Date.now()}\n`); await rec.cli(client, ['add', '--all'], { quietFrame: true }); await rec.cli(client, ['commit', '-m', 'a note'], { quietFrame: true })
  r = await push(artist, 'push as artist1 with read only')
  rec.check(r.code !== 0, 'a read-only person cannot push')
  rec.check(/read|permission|not allowed|access|forbidden/i.test(r.stdout + r.stderr) && !/panicked|os error/i.test(r.stdout + r.stderr), 'the refusal names the missing right', flat(r.stdout + r.stderr).slice(-250))
  const cl = join(WORK, 'S-04-clone'); rmSync(cl, { recursive: true, force: true })
  const c = await rec.cli(WORK, ['clone', `${b}/hero-film`, cl], { env: as(artist) })
  rec.note(`clone as read-only: exit ${c.code} ${flat(c.stdout + c.stderr).slice(-200)}`)
  await rec.step('Raise artist1 to "can change files" on hero-film', () => give(/Can change files/))
  r = await push(artist, 'push as artist1 with write')
  rec.check(r.code === 0, 'with write the push goes through')
  const ra = await rec.cli(client, ['auth', 'admin', 'repo-access', 'hero-film', '--server', b], { env: as(admin) })
  rec.check(ra.code === 0 && ra.stdout.includes(ARTIST.user), '`auth admin repo-access` lists artist1', flat(ra.stdout + ra.stderr).slice(0, 300))
  await rec.step('Remove access…: cancel, then confirm', async () => {
    await page.getByRole('button', { name: 'Remove access…' }).click(); await page.waitForTimeout(500); rec.note(`remove dialog: ${await dialogText(page)}`)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500)
    await page.getByRole('button', { name: 'Remove access…' }).click(); await page.waitForTimeout(500)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Remove access' }).click(); await page.waitForTimeout(2500)
  })
  writeFileSync(join(client, 'notes.txt'), `note ${Date.now()} b\n`); await rec.cli(client, ['add', '--all'], { quietFrame: true }); await rec.cli(client, ['commit', '-m', 'another note'], { quietFrame: true })
  r = await push(artist, 'push as artist1 after the grant is removed')
  rec.check((r.code === 0) === noGrant, 'after removal the person is back to the no-grant behaviour', `exit ${r.code}, no-grant push worked: ${noGrant}`)
  const lg2 = await rec.cli(client, ['auth', 'admin', 'list-grants', ARTIST.user, '--server', b], { env: as(admin), quietFrame: true })
  rec.check(!lg2.stdout.includes('hero-film'), 'the grant is gone from `list-grants`', flat(lg2.stdout).slice(0, 200))
  rec.note(`user grants after removal: ${flat(await page.getByTestId('user-grants').innerText().catch(() => ''))}`)
  rmSync(cl, { recursive: true, force: true })
  await rec.cli(client, ['push', '-u', 'origin', 'main'], { env: as(admin), quietFrame: true })
  await intCheck(rec, client)
  await rec.finish()
  await closeTerm()
})

test('S-05 API keys: make, use, revoke', async () => {
  const rec = await Recorder.start('S-05', 'api-keys', { gui: true })
  const page = rec.page!
  await rec.step('Open Team > Server > API keys', async () => { await openServer(page); await tab(page, 'API keys') })
  const b = await base(page)
  const dlg = page.getByRole('dialog')
  rec.note(`keys on arrival: ${JSON.stringify(await table(page, 'keys'))} | ${flat(await page.locator('#app').innerText()).slice(-200)}`)
  await rec.step('New key… "build machine"', async () => { await page.getByRole('button', { name: 'New key…' }).click(); await page.waitForTimeout(500); await dlg.getByLabel('Name').fill('build machine'); await dlg.getByText('Limit what it can do').click(); await page.waitForTimeout(400); rec.note(`new key dialog: ${await dialogText(page)}`); await dlg.getByRole('button', { name: 'Make key' }).click(); await page.waitForTimeout(2500) })
  const key = (await page.getByTestId('new-key-value').innerText().catch(() => '')).trim()
  rec.note(`key shown: ${key ? `${key.slice(0, 6)}… (${key.length} characters)` : 'NOT SHOWN'} | dialog: ${(await dialogText(page)).replace(key, '<key>')}`)
  rec.check(key.length >= 20, 'the new key is shown once')
  rec.check(await page.getByRole('button', { name: 'Done' }).isDisabled(), 'Done is off until "I saved it" is ticked')
  await rec.step('Escape does not close the key dialog', async () => { await page.keyboard.press('Escape'); await page.waitForTimeout(500) })
  rec.check(await page.getByTestId('new-key-value').isVisible().catch(() => false), 'the key dialog cannot be dismissed by accident')
  await rec.step('Tick "I saved it", Done', async () => { await page.getByText('I saved it').click(); await page.waitForTimeout(300); await page.getByRole('button', { name: 'Done' }).click(); await page.waitForTimeout(1500) })
  const rows = await table(page, 'keys')
  rec.note(`keys table: ${JSON.stringify(rows).replace(key, '<FULL KEY IN TABLE>')}`)
  rec.check(rows.some((r) => r.includes('build machine')) && !JSON.stringify(rows).includes(key), 'the key is listed by name, never in full')
  rec.check(!(await page.locator('#app').innerText()).includes(key), 'the key is nowhere on the page after Done')
  const use = { ...NOKEY, MEDIAGIT_API_KEY: key }
  const kl = await rec.cli(client, ['auth', 'key', 'list', '--server', b], { env: use })
  rec.note(`CLI auth key list: exit ${kl.code} ${flat(kl.stdout + kl.stderr).slice(0, 300).replace(key, '<key>')}`)
  writeFileSync(join(client, 'notes.txt'), `key note ${Date.now()}\n`); await rec.cli(client, ['add', '--all'], { quietFrame: true }); await rec.cli(client, ['commit', '-m', 'pushed with a key'], { quietFrame: true })
  let r = await rec.cli(client, ['push', 'origin', 'main'], { env: use })
  rec.check(r.code === 0, 'a push with the API key works', flat(r.stdout + r.stderr).slice(-200))
  const who = await rec.cli(client, ['auth', 'whoami', '--server', b], { env: use })
  rec.check(who.code === 0 && who.stdout.includes(ADMIN.user), '`auth whoami` with the key is the admin', flat(who.stdout + who.stderr).slice(0, 200))
  const leaks = [...filesWith(SRV, key), ...filesWith(join(WORK, 'logs'), key), ...filesWith(join(client, '.mediagit'), key), ...filesWith(join(WORK, 'app-server'), key)]
  rec.check(leaks.length === 0, 'the key is in no server file, client config or log', leaks.join(', '))
  await rec.step('Revoke…: cancel, then confirm', async () => {
    await page.getByRole('button', { name: 'Revoke key build machine' }).click(); await page.waitForTimeout(500); rec.note(`revoke dialog: ${await dialogText(page)}`)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500)
    await page.getByRole('button', { name: 'Revoke key build machine' }).click(); await page.waitForTimeout(500)
    const t0 = Date.now()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Revoke key' }).click(); await page.waitForTimeout(2500)
    rec.note(`revoke returned after about ${Date.now() - t0} ms`)
  })
  rec.check(!(await table(page, 'keys')).some((x) => x.includes('build machine')), 'the key leaves the list', JSON.stringify(await table(page, 'keys')))
  writeFileSync(join(client, 'notes.txt'), `key note ${Date.now()} b\n`); await rec.cli(client, ['add', '--all'], { quietFrame: true }); await rec.cli(client, ['commit', '-m', 'after revoke'], { quietFrame: true })
  r = await rec.cli(client, ['push', 'origin', 'main'], { env: use })
  rec.check(r.code !== 0, 'a revoked key is refused')
  rec.check(/key|revoked|sign in|not valid|credential|unauthor/i.test(r.stdout + r.stderr) && !(r.stdout + r.stderr).includes(key), 'the refusal is clear and does not echo the key', flat(r.stdout + r.stderr).slice(-250).replace(key, '<KEY ECHOED>'))
  await rec.cli(client, ['push', 'origin', 'main'], { env: as((await login(b, ADMIN)).token!), quietFrame: true })
  await intCheck(rec, client)
  await rec.finish()
  await closeTerm()
})

test('S-06 Delete a server repository, then stop the server', async () => {
  const rec = await Recorder.start('S-06', 'server-repos', { gui: true })
  const page = rec.page!
  await rec.step('Open Team > Server > Repositories', async () => { await openServer(page); await tab(page, 'Repositories') })
  const b = await base(page)
  const admin = (await login(b, ADMIN)).token!
  const dlg = page.getByRole('dialog')
  await rec.step('Create an empty repository "scratch"', async () => { await page.getByRole('button', { name: 'New repository…' }).click(); await page.waitForTimeout(500); await dlg.getByLabel('Name').fill('scratch'); await dlg.getByRole('button', { name: 'Create repository' }).click(); await page.waitForTimeout(2500) })
  rec.note(`repos: ${JSON.stringify(await table(page, 'repos'))}`)
  const onDisk = () => (existsSync(join(SRV, 'repos')) ? readdirSync(join(SRV, 'repos')) : readdirSync(SRV)).join(', ')
  rec.note(`server folder repos: ${onDisk()}`)
  await rec.step('Delete scratch: wrong text typed', async () => { await page.getByRole('button', { name: 'Delete scratch' }).click(); await page.waitForTimeout(500); rec.note(`delete dialog: ${await dialogText(page)}`); await dlg.getByLabel('Type scratch to confirm').fill('scrach'); await page.waitForTimeout(300) })
  rec.check(await dlg.getByRole('button', { name: 'Delete repository' }).isDisabled(), 'a wrong name keeps Delete off')
  await rec.step('Cancel', async () => { await dlg.getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(600) })
  rec.check((await table(page, 'repos')).some((r) => r.includes('scratch')), 'cancel deletes nothing')
  await rec.step('Delete scratch: right name', async () => { await page.getByRole('button', { name: 'Delete scratch' }).click(); await page.waitForTimeout(500); await dlg.getByLabel('Type scratch to confirm').fill('scratch'); await page.waitForTimeout(300); await dlg.getByRole('button', { name: 'Delete repository' }).click(); await page.waitForTimeout(3000) })
  rec.check(!(await table(page, 'repos')).some((r) => r.includes('scratch')), 'the empty repository is deleted', await alerts(page))
  rec.note(`retained note: ${flat(await page.getByTestId('retained').innerText().catch(() => '(none)'))} | on disk: ${onDisk()}`)
  await rec.step('Delete hero-film (holds versions): right name, no tick', async () => { await page.getByRole('button', { name: 'Delete hero-film' }).click(); await page.waitForTimeout(500); rec.note(`delete dialog: ${await dialogText(page)}`); await dlg.getByLabel('Type hero-film to confirm').fill('hero-film'); await page.waitForTimeout(300); await rec.shot('typed, not ticked'); await dlg.getByRole('button', { name: 'Delete repository' }).click(); await page.waitForTimeout(3000) })
  const noTick = `${await dialogText(page)} | ${await alerts(page)}`
  rec.note(`without the tick: ${noTick}`)
  rec.check((await table(page, 'repos')).some((r) => r.includes('hero-film')), 'a repository with versions is not deleted without the extra tick')
  rec.check(!raw(noTick), 'the refusal is in plain words', noTick.slice(0, 300))
  await rec.step('Tick "Delete even though it holds…", delete', async () => {
    if (!(await dlg.isVisible().catch(() => false))) { await page.getByRole('button', { name: 'Delete hero-film' }).click(); await page.waitForTimeout(500); await dlg.getByLabel('Type hero-film to confirm').fill('hero-film') }
    await dlg.getByText(/Delete even though it holds/).click(); await page.waitForTimeout(300)
    await rec.shot('ticked')
    await dlg.getByRole('button', { name: 'Delete repository' }).click(); await page.waitForTimeout(4000)
  })
  rec.check(!(await table(page, 'repos')).some((r) => r.includes('hero-film')), 'hero-film is deleted', await alerts(page))
  const kept = flat(await page.getByTestId('retained').innerText().catch(() => '(none)'))
  rec.note(`retained note: ${kept} | on disk: ${onDisk()} | server folder: ${readdirSync(SRV).join(', ')}`)
  rec.check(!existsSync(join(SRV, 'repos', 'hero-film')), 'the repository folder is gone from the server folder', readdirSync(SRV).join(', '))
  const f = await rec.cli(client, ['fetch', 'origin'], { env: as(admin) })
  rec.check(f.code !== 0 && !/panicked|os error/i.test(f.stdout + f.stderr), 'a client fetch from the deleted repository fails cleanly', flat(f.stdout + f.stderr).slice(-250))
  const rr = await rec.cli(client, ['remote', 'repos', 'list', '--server', b], { env: as(admin) })
  rec.note(`CLI remote repos: exit ${rr.code} ${flat(rr.stdout + rr.stderr).slice(0, 200)}`)
  await intCheck(rec, client)
  await rec.step('Overview: Stop server', async () => { await tab(page, 'Overview'); await page.getByRole('button', { name: 'Stop server…' }).click(); await page.waitForTimeout(500); await page.getByRole('button', { name: 'Stop server', exact: true }).click(); await page.waitForTimeout(4000) })
  rec.check(!(await up(b)), 'the server is stopped at the end of the area')
  await rec.step('People tab while the server is down', async () => { await tab(page, 'People'); await page.waitForTimeout(1000) })
  rec.note(`people while down: ${flat(await page.locator('#app').innerText()).slice(-250)}`)
  rec.check(/not running/i.test(await page.locator('#app').innerText()) && !raw(await alerts(page)), 'a stopped server gives a plain "not running" page', await alerts(page))
  await rec.finish()
  await closeTerm()
})
