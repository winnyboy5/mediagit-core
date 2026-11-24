import { execFileSync, spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { mkdirSync, openSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CLI, CLI_ENV, SERVER, WORK, stopTracked, trackPid, waitFor } from './env'
import { Recorder, closeTerm } from './rec'
import { dialogText, flat, go, openRepo, toWelcome, visibleErrors } from './gui'

// App pass on gaps left by the first run: BUG-041 through "Include all", and the Account card (S-15).
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
const RAW = /os error|panicked|exit code|stderr|\bErr\(|protocol|unexpected output|status code|HTTP \d{3}|\b40[0-9]\b|\b50[0-9]\b|deserialize/i
const fresh = (name: string) => { const d = join(WORK, name); rmSync(d, { recursive: true, force: true }); mkdirSync(d, { recursive: true }); return d }
const alerts = async (page: Page) => flat((await visibleErrors(page)).join(' || ')).slice(0, 400)
const fwd = (p: string) => p.replace(/\\/g, '/')

test('B-12 Sparse checkout seen from the app: Include all and Save', async () => {
  const rec = await Recorder.start('B-12', 'sparse-in-app', { gui: true })
  const page = rec.page!
  const repo = fresh('B-12')
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY, quietFrame: true })
  mkdirSync(join(repo, 'keep')); mkdirSync(join(repo, 'big'))
  writeFileSync(join(repo, 'keep/k.txt'), 'k v1\n'); writeFileSync(join(repo, 'big/b1.txt'), 'b1\n'); writeFileSync(join(repo, 'big/b2.txt'), 'b2\n')
  await run('init'); await run('add', '--all'); await run('commit', '-m', 'v1')
  const set = await run('sparse-checkout', 'set', 'keep')
  rec.check(set.code === 0, 'setup: sparse-checkout set keep', flat(set.stdout + set.stderr))
  writeFileSync(join(repo, 'keep/k.txt'), 'k v2\n')

  await toWelcome(page)
  await rec.step('Open the sparse repository, Files > Changes', async () => { await openRepo(page, repo); await go(page, 'files/changes'); await page.waitForTimeout(2500) })
  const summary0 = flat(await page.getByTestId('changes-summary').innerText().catch(() => '(no summary)'))
  const body0 = flat(await page.locator('main').innerText().catch(() => '')).slice(0, 500)
  rec.note(`Changes before Include all: summary "${summary0}" | ${body0}`)
  rec.check(!/big\/b1|b1\.txt/.test(body0), 'excluded files are not listed as changes', body0)
  rec.note(`any hint in the app that this project is sparse: ${/sparse|partial|only part/i.test(flat(await page.locator('#app').innerText())) ? 'yes' : 'none found on this screen'}`)
  await rec.step('Include all', async () => { await page.getByRole('button', { name: 'Include all', exact: true }).click(); await page.waitForTimeout(3000) })
  const body1 = flat(await page.locator('main').innerText().catch(() => '')).slice(0, 600)
  rec.note(`Changes after Include all: summary "${flat(await page.getByTestId('changes-summary').innerText().catch(() => ''))}" | ${body1} | alerts ${await alerts(page)}`)
  const st = await run('status', '--machine')
  rec.note(`status --machine after Include all: ${st.stdout.slice(0, 500)}`)
  rec.check(!/big\/b1|b1\.txt|deleted/i.test(body1), 'the app does not show the excluded files as included deletions', body1)
  await rec.step('Message, Save version', async () => {
    await page.getByLabel('Message').first().fill('v2 from the app while sparse'); await page.waitForTimeout(300)
    await page.getByRole('button', { name: 'Save version', exact: true }).click(); await page.waitForTimeout(4000)
  })
  rec.note(`after Save: alerts ${await alerts(page)}`)
  const d = await rec.cli(repo, ['diff', 'HEAD~1', 'HEAD'], { env: NOKEY })
  rec.check(/v2 from the app/.test(d.stdout), 'the version was saved from the app', flat(d.stdout).slice(0, 300))
  rec.check(!/deleted:/.test(d.stdout), 'BUG-041 in the app: Include all + Save must not delete the sparse-excluded files', flat(d.stdout).slice(0, 400))
  await go(page, 'history'); await page.waitForTimeout(2500)
  rec.note(`History after the save: ${flat(await page.locator('main').innerText().catch(() => '')).slice(0, 400)}`)
  await rec.shot('history').catch(() => {})
  await toWelcome(page)
  await rec.finish(); await closeTerm()
})

test('S-15 Account card: create account, sign in, change password, sign out', async () => {
  const rec = await Recorder.start('S-15', 'account', { gui: true })
  const page = rec.page!
  const root = fresh('S-15')
  // An auth-enabled server with open sign-up, started by the harness.
  const sdir = join(root, 'srv'); mkdirSync(join(sdir, 'repos', 'r'), { recursive: true })
  execFileSync(CLI, ['init', '--bare', join(sdir, 'repos', 'r')], { env: { ...process.env, ...CLI_ENV }, stdio: 'ignore' })
  const port = 20000 + Math.floor(Math.random() * 20000)
  writeFileSync(join(sdir, 'server.toml'), `port = ${port}\nhost = "127.0.0.1"\nrepos_dir = "${fwd(join(sdir, 'repos'))}"\nenable_auth = true\nallow_open_registration = true\njwt_secret = "${randomBytes(32).toString('hex')}"\n`)
  const out = openSync(join(sdir, 'server.log'), 'a')
  const sp = spawn(SERVER, ['--config', join(sdir, 'server.toml')], { cwd: sdir, env: { ...process.env }, stdio: ['ignore', out, out], windowsHide: true })
  if (sp.pid) trackPid(sp.pid, 'mediagit-server')
  const base = `http://127.0.0.1:${port}`
  const upOk = await waitFor('auth server health', 30_000, async () => (await fetch(`${base}/health`)).ok).then(() => true).catch(() => false)
  rec.check(upOk, 'setup: an auth-enabled server with open sign-up starts', base)
  if (!upOk) { await rec.finish(); await closeTerm(); return }
  const http = async (path: string, body: unknown) => { const r = await fetch(`${base}${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); return { status: r.status, text: (await r.text()).slice(0, 160).replace(/"[A-Za-z0-9._-]{40,}"/g, '"<hidden>"') } }
  const canLogin = async (pass: string) => (await http('/auth/login', { identifier: U.user, password: pass })).status === 200

  const repo = join(root, 'client'); mkdirSync(repo)
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY, quietFrame: true })
  writeFileSync(join(repo, 'a.txt'), 'a\n'); await run('init'); await run('add', '--all'); await run('commit', '-m', 'v1'); await run('remote', 'add', 'origin', `${base}/r`)
  const U = { user: 'newartist', email: 'newartist@example.test', pass: 'Qa-Live!2026-new', pass2: 'Qa-Live!2026-changed' }
  const acct = page.getByTestId('account')
  const dlg = page.getByRole('dialog')
  const card = async () => flat(await acct.innerText().catch(() => '(no account card)'))

  await toWelcome(page)
  await rec.step('Open the project, Settings > Security', async () => { await openRepo(page, repo); await go(page, 'settings/security'); await page.waitForTimeout(3000) })
  rec.note(`account card on arrival: ${await card()}`)
  rec.check(/not signed in/i.test(await card()), 'the card says you are not signed in, and to which server', await card())

  await rec.step('Create account…: passwords differ', async () => {
    await acct.getByRole('button', { name: 'Create account…' }).click(); await page.waitForTimeout(500)
    rec.note(`dialog: ${await dialogText(page)}`)
    await dlg.getByLabel('Username', { exact: true }).fill(U.user); await dlg.getByLabel('Email').fill(U.email)
    await dlg.getByLabel('Password', { exact: true }).fill(U.pass); await dlg.getByLabel('Repeat password').fill('different'); await page.waitForTimeout(300)
  })
  rec.check(await dlg.getByRole('button', { name: 'Create account', exact: true }).isDisabled(), 'mismatched passwords cannot be submitted')
  await rec.step('Create account: weak password "abc"', async () => {
    await dlg.getByLabel('Password', { exact: true }).fill('abc'); await dlg.getByLabel('Repeat password').fill('abc'); await page.waitForTimeout(300)
    if (!(await dlg.getByRole('button', { name: 'Create account', exact: true }).isDisabled())) { await dlg.getByRole('button', { name: 'Create account', exact: true }).click(); await page.waitForTimeout(2500) }
  })
  const weak = `${await alerts(page)} | dialog open: ${await dlg.isVisible().catch(() => false)}`
  rec.note(`weak password result: ${weak}`)
  rec.check(!(await canLogin('abc')), 'a 3-character password is not accepted', weak)
  rec.check(!RAW.test(weak), 'the weak-password text is in plain words', weak)
  await rec.step('Create account: good values', async () => {
    if (!(await dlg.isVisible().catch(() => false))) { await acct.getByRole('button', { name: 'Create account…' }).click(); await page.waitForTimeout(500); await dlg.getByLabel('Username', { exact: true }).fill(U.user); await dlg.getByLabel('Email').fill(U.email) }
    await dlg.getByLabel('Password', { exact: true }).fill(U.pass); await dlg.getByLabel('Repeat password').fill(U.pass); await page.waitForTimeout(300)
    await dlg.getByRole('button', { name: 'Create account', exact: true }).click(); await page.waitForTimeout(3500)
  })
  rec.note(`after Create account: alerts ${await alerts(page)} | card ${await card()} | toasts ${flat(await page.locator('[role="status"]').allInnerTexts().then((t) => t.join(' | ')).catch(() => '')).slice(0, 200)}`)
  const firstTry = await canLogin(U.pass)
  rec.check(firstTry, 'Create account with the server field left empty works (the hint says it uses this repository\'s remote)', await alerts(page))
  if (!firstTry) {
    const help = await rec.cli(repo, ['auth', 'register', '--help'], { quietFrame: true })
    rec.note(`CLI auth register --help: ${flat(help.stdout + help.stderr).slice(0, 500)}`)
    const twin = await rec.cli(repo, ['auth', 'register', '--username', 'clitwin', '--email', 'clitwin@example.test', '--secret-stdin'], { env: NOKEY, input: `${U.pass}\n` })
    rec.note(`CLI twin from inside the repository, no --server: exit ${twin.code} ${flat(twin.stdout + twin.stderr).slice(0, 300)}`)
    await page.getByRole('button', { name: 'Dismiss' }).first().click().catch(() => {})
    await rec.step('Create account again, server address typed in', async () => {
      if (!(await dlg.isVisible().catch(() => false))) { await acct.getByRole('button', { name: 'Create account…' }).click(); await page.waitForTimeout(500) }
      await dlg.getByLabel('Server address (optional)').fill(base); await dlg.getByLabel('Username', { exact: true }).fill(U.user); await dlg.getByLabel('Email').fill(U.email)
      await dlg.getByLabel('Password', { exact: true }).fill(U.pass); await dlg.getByLabel('Repeat password').fill(U.pass); await page.waitForTimeout(300)
      await dlg.getByRole('button', { name: 'Create account', exact: true }).click(); await page.waitForTimeout(3500)
    })
    rec.note(`second try: alerts ${await alerts(page)}`)
  }
  rec.check(await canLogin(U.pass), 'the new account exists on the server (HTTP sign-in works)')
  if (await dlg.isVisible().catch(() => false)) await dlg.getByRole('button', { name: 'Cancel' }).click().catch(() => {})

  const signIn = async (pass: string) => {
    if (!(await acct.getByRole('button', { name: 'Sign in' }).isVisible().catch(() => false))) return
    await acct.getByRole('button', { name: 'Sign in' }).click(); await page.waitForTimeout(500)
    await dlg.getByLabel('Username or email').fill(U.user); await dlg.getByLabel('Password').fill(pass)
    await dlg.getByRole('button', { name: 'Sign in' }).click(); await page.waitForTimeout(3500)
  }
  await rec.step('Sign out first (Create account signs in automatically)', async () => { if (!(await acct.getByRole('button', { name: 'Sign out…' }).isVisible().catch(() => false))) return; await acct.getByRole('button', { name: 'Sign out…' }).click(); await page.waitForTimeout(500); await page.getByRole('alertdialog').getByRole('button', { name: 'Sign out', exact: true }).click(); await page.waitForTimeout(3000) })
  await rec.step('Sign in with a wrong password', () => signIn('wrong-password'))
  const wrong = `${await alerts(page)} | ${await dlg.isVisible().catch(() => false) ? await dialogText(page) : ''}`
  rec.note(`wrong password: ${wrong.slice(0, 400)}`)
  rec.check(!/Signed in as/.test(await card()), 'a wrong password does not sign in')
  rec.check(!RAW.test(wrong) && /password|incorrect|wrong|match|sign/i.test(wrong), 'the wrong-password text is plain', wrong.slice(0, 300))
  if (await dlg.isVisible().catch(() => false)) await dlg.getByRole('button', { name: 'Cancel' }).click().catch(() => {})
  await page.getByRole('button', { name: 'Dismiss' }).first().click().catch(() => {})
  await rec.step('Sign in with the right password', () => signIn(U.pass))
  rec.note(`card after sign-in: ${await card()} | alerts ${await alerts(page)}`)
  rec.check(new RegExp(`Signed in as\\W+${U.user}`).test(await card()), 'the card shows the signed-in user', await card())
  await rec.shot('signed-in').catch(() => {})
  const key = () => { try { return execFileSync('cmdkey', ['/list'], { encoding: 'utf8' }).split('\n').filter((l) => l.includes(String(port))).map((l) => l.trim()).join(' ; ') } catch { return 'cmdkey failed' } }
  rec.note(`Windows Credential Manager entries for this server after sign-in: ${key() || '(none)'}`)

  await rec.step('Change password…: wrong current password', async () => {
    await acct.getByRole('button', { name: 'Change password…' }).click(); await page.waitForTimeout(500)
    await dlg.getByLabel('Current password').fill('not-my-password'); await dlg.getByLabel('New password', { exact: true }).fill(U.pass2); await dlg.getByLabel('Repeat new password').fill(U.pass2); await page.waitForTimeout(300)
    await dlg.getByRole('button', { name: 'Change password', exact: true }).click(); await page.waitForTimeout(3000)
  })
  const wc = `${await alerts(page)} | dialog open: ${await dlg.isVisible().catch(() => false)}`
  rec.note(`wrong current password: ${wc}`)
  rec.check(await canLogin(U.pass) && !(await canLogin(U.pass2)), 'a wrong current password changes nothing')
  rec.check(!RAW.test(wc), 'the text is plain', wc)
  await page.getByRole('button', { name: 'Dismiss' }).first().click().catch(() => {})
  await rec.step('Change password: right current password', async () => {
    if (!(await dlg.isVisible().catch(() => false))) { await acct.getByRole('button', { name: 'Change password…' }).click(); await page.waitForTimeout(500) }
    await dlg.getByLabel('Current password').fill(U.pass); await dlg.getByLabel('New password', { exact: true }).fill(U.pass2); await dlg.getByLabel('Repeat new password').fill(U.pass2); await page.waitForTimeout(300)
    await dlg.getByRole('button', { name: 'Change password', exact: true }).click(); await page.waitForTimeout(3500)
  })
  rec.note(`after Change password: alerts ${await alerts(page)} | card ${await card()}`)
  rec.check(await canLogin(U.pass2), 'the new password works on the server')
  rec.check(!(await canLogin(U.pass)), 'the old password stops working at once')
  const who = await rec.cli(repo, ['auth', 'status'], { quietFrame: true })
  rec.note(`CLI auth status with the keychain (same machine): exit ${who.code} ${flat(who.stdout + who.stderr).slice(0, 200)}`)

  await rec.step('Sign out…, cancel', async () => { if (!(await acct.getByRole('button', { name: 'Sign out…' }).isVisible().catch(() => false))) return; await acct.getByRole('button', { name: 'Sign out…' }).click(); await page.waitForTimeout(500); rec.note(`sign-out dialog: ${flat(await page.getByRole('alertdialog').innerText().catch(() => ''))}`); await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(500) })
  rec.check(/Signed in as/.test(await card()), 'cancel keeps you signed in', await card())
  await rec.step('Sign out…, confirm', async () => { await acct.getByRole('button', { name: 'Sign out…' }).click(); await page.waitForTimeout(500); await page.getByRole('alertdialog').getByRole('button', { name: 'Sign out', exact: true }).click(); await page.waitForTimeout(3000) })
  rec.check(/not signed in/i.test(await card()), 'after Sign out the card says not signed in', await card())
  rec.check(key() === '', 'the token is removed from Windows Credential Manager', key())
  await toWelcome(page)
  if (sp.pid) stopTracked(sp.pid)
  await rec.finish(); await closeTerm()
})
