import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { MCLI, WORK, pidsOf } from './env'
import { Recorder, closeTerm } from './rec'
import { dialogText, flat, go, store, toWelcome, visibleErrors } from './gui'
import { copyCorpus, hashCompare, intCheck } from './integrity'

// S-13: server storage on S3-compatible Silo at :9000, set from the app's "Set up local server" form.
// Uses an existing bucket and a fresh folder (prefix) inside it. Creates no bucket.
const SRV = join(WORK, 'app-server')
const S3 = { endpoint: 'http://127.0.0.1:9000', bucket: 'mediagit-qa-suite', region: 'us-east-1', key: 'minioadmin', secret: 'minioadmin' }
const PREFIX = `qa-live-s13-${Date.now()}`
const ADMIN = { user: 'qaadmin', email: 'qaadmin@example.test', pass: 'Qa-Live!2026-admin' }
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
const BIG = 'video-implementation/DW0001C004_251020_113048_p1I7H.mxf'
const RAW = /os error|panicked|exit code|stderr|\bErr\(|dispatch failure|SdkError|status code|HTTP \d{3}|\b40[0-9]\b|\b50[0-9]\b/i

const mc = (...args: string[]) => { try { return execFileSync(MCLI!, args, { env: { ...process.env, MC_HOST_s: `http://${S3.key}:${S3.secret}@127.0.0.1:9000` }, encoding: 'utf8', windowsHide: true }) } catch (e) { return `mcli failed: ${String(e).slice(0, 200)}` } }
const buckets = () => mc('ls', 's').split('\n').map((l) => l.trim().split(/\s+/).pop() ?? '').filter(Boolean).sort().join(',')
const local = (page: Page) => store<{ state: string; url: string | null } | null>(page, 'server', 's.local')
const tree = (dir: string, out: string[] = []): string[] => { for (const n of existsSync(dir) ? readdirSync(dir) : []) { const p = join(dir, n); if (statSync(p).isDirectory()) tree(p, out); else out.push(p) } return out }

test('S-13 Server storage on Silo (S3) from the app', async () => {
  const rec = await Recorder.start('S-13', 'silo-storage', { gui: true })
  const page = rec.page!
  if (!MCLI || !existsSync(MCLI)) {
    rec.skip(`MG_QA_MCLI not set or missing (${MCLI ?? 'unset'}): the Silo mcli client is needed to list buckets`)
    await rec.finish(); await closeTerm(); return
  }
  const health = await fetch(`${S3.endpoint}/minio/health/live`).then((r) => r.status).catch(() => 0)
  rec.check(health === 200, 'setup: Silo answers on :9000', String(health))
  const before = buckets()
  rec.note(`buckets before: ${before}`)
  if (pidsOf('mediagit-server.exe').length === 0) for (const n of existsSync(SRV) ? readdirSync(SRV) : []) rmSync(join(SRV, n), { recursive: true, force: true })
  else rec.note('a mediagit-server process is already running; the server folder was not cleared')

  await toWelcome(page)
  await rec.step('Open Team > Server', async () => { await go(page, 'team/server'); await page.waitForTimeout(2500) })
  rec.note(`on arrival: ${JSON.stringify(await local(page))}`)
  const dlg = page.getByRole('dialog')
  await rec.step('Set up local server…, choose "Amazon S3 or MinIO"', async () => {
    await page.getByRole('button', { name: 'Set up local server…' }).click(); await page.waitForTimeout(600)
    await dlg.getByLabel('Admin username').fill(ADMIN.user); await dlg.getByLabel('Admin email').fill(ADMIN.email)
    await dlg.getByLabel('Password', { exact: true }).fill(ADMIN.pass); await dlg.getByLabel('Repeat password').fill(ADMIN.pass)
    await dlg.getByText('Amazon S3 or MinIO', { exact: true }).click(); await page.waitForTimeout(500)
  })
  rec.note(`form: ${flat(await dlg.innerText()).slice(0, 600)}`)

  const fill = async (o: Partial<typeof S3> & { prefix?: string }) => {
    const v = { ...S3, prefix: PREFIX, ...o }
    await dlg.getByLabel('Bucket', { exact: true }).fill(v.bucket); await dlg.getByLabel('Region').fill(v.region); await dlg.getByLabel('Endpoint').fill(v.endpoint)
    await dlg.getByLabel('Folder inside the bucket').fill(v.prefix); await dlg.getByLabel('Access key ID').fill(v.key); await dlg.getByLabel('Secret key').fill(v.secret)
    await page.waitForTimeout(300)
  }
  const probe = async (name: string): Promise<{ ok: boolean; text: string; ms: number }> => {
    const t = Date.now()
    await dlg.getByRole('button', { name: 'Test connection' }).click()
    await page.locator('[data-testid="storage-ok"], [data-testid="storage-bad"]').first().waitFor({ timeout: 120_000 }).catch(() => {})
    const ok = await page.getByTestId('storage-ok').isVisible().catch(() => false)
    const text = flat(await page.locator('[data-testid="storage-ok"], [data-testid="storage-bad"]').first().innerText().catch(() => '(no result shown)'))
    const ms = Date.now() - t
    rec.note(`Test connection, ${name}: ${ok ? 'OK' : 'FAILED'} in ${ms} ms: ${text}`)
    await rec.shot(`test-${name.replace(/\W+/g, '-')}`)
    return { ok, text, ms }
  }
  const bad = (r: { ok: boolean; text: string; ms: number }, name: string, hint: RegExp) => {
    rec.check(!r.ok, `${name}: Test connection says it failed`, r.text)
    rec.check(!RAW.test(r.text), `${name}: the reason is in plain words`, r.text)
    rec.check(hint.test(r.text), `${name}: the reason points at the right field`, r.text)
    rec.check(r.ms < 30_000, `${name}: answers within 30 s`, `${r.ms} ms`)
  }

  await rec.step('Test: empty form', async () => { await fill({ bucket: '', endpoint: '', key: '', secret: '', prefix: '' }) })
  const e0 = await probe('empty form'); rec.check(!e0.ok, 'an empty S3 form does not test as OK', e0.text)
  await rec.step('Test: wrong secret', () => fill({ secret: 'not-the-secret' }))
  bad(await probe('wrong secret'), 'wrong secret', /key|secret|credential|sign|access|denied/i)
  await rec.step('Test: wrong access key', () => fill({ key: 'nobody' }))
  bad(await probe('wrong access key'), 'wrong access key', /key|credential|access|denied/i)
  await rec.step('Test: endpoint nobody listens on', () => fill({ endpoint: 'http://127.0.0.1:9' }))
  bad(await probe('dead endpoint'), 'dead endpoint', /reach|connect|endpoint|address|server/i)
  rec.note('missing-bucket test is not repeated here: run 1 showed it CREATES the bucket (fail/S-13-run1-test-connection.log)')
  await rec.step('Test: good values', () => fill({}))
  const good = await probe('good values')
  rec.check(good.ok, 'good values: Test connection says OK', good.text)
  rec.note(`objects under the prefix after a successful test: ${mc('ls', '--recursive', `s/${S3.bucket}/${PREFIX}/`).trim().split('\n').filter(Boolean).length} (a probe object left behind would show here)`)

  await rec.step('Set up and start', async () => {
    // Typing in the storage form must not have cleared the account fields.
    rec.note(`account fields before submit: user "${await dlg.getByLabel('Admin username').inputValue()}", password ${(await dlg.getByLabel('Password', { exact: true }).inputValue()).length} chars`)
    await dlg.getByRole('button', { name: 'Set up and start' }).click()
    await page.waitForFunction(() => /running/i.test((document.querySelector('[data-testid="local-state"]') as HTMLElement | null)?.innerText ?? ''), null, { timeout: 90_000 }).catch(() => {})
  })
  const l = await local(page)
  rec.note(`after set up: ${JSON.stringify(l)} | alerts ${JSON.stringify(await visibleErrors(page)).slice(0, 400)} | dialog ${await dialogText(page)}`)
  rec.check(l?.state === 'running', 'the server is running on S3 storage', JSON.stringify(l))
  await rec.shot('running-on-s3').catch(() => {})
  if (l?.state !== 'running' || !l.url) { await rec.finish(); await closeTerm(); return }
  const b = l.url.replace(/\/$/, '')

  // Where did the settings and the secret go?
  const cfgFiles = tree(SRV).filter((f) => statSync(f).size < 2e6 && /\.(toml|json|ya?ml|env|txt|log)$/i.test(f))
  const withBucket = cfgFiles.filter((f) => readFileSync(f, 'utf8').includes(S3.bucket))
  rec.check(withBucket.length > 0, 'the server config names the bucket', cfgFiles.map((f) => f.replace(SRV, '')).join(', '))
  for (const f of withBucket) {
    const lines = readFileSync(f, 'utf8').split('\n').filter((x) => /bucket|endpoint|prefix|region|access_key|secret|backend/i.test(x)).map((x) => x.replace(/(secret[^=:]*[=:]\s*).*/i, '$1<hidden>'))
    rec.note(`${f.replace(SRV, '')}: ${lines.join(' ; ')}`)
    rec.note(`secret key stored in clear text in ${f.replace(SRV, '')}: ${/secret[^=:\n]*[=:]\s*"?minioadmin/i.test(readFileSync(f, 'utf8')) ? 'YES' : 'no'}`)
  }
  const appLog = join(WORK, 'logs')
  const leaked = tree(appLog).filter((f) => statSync(f).size < 20e6 && /secret[^\n]{0,40}minioadmin/i.test(readFileSync(f, 'utf8')))
  rec.check(leaked.length === 0, 'the secret key is not in the app log', leaked.join(', '))

  // Push and clone through the S3-backed server.
  let tok: string | undefined
  for (const body of [{ username: ADMIN.user, password: ADMIN.pass }, { identifier: ADMIN.user, password: ADMIN.pass }, { email: ADMIN.email, password: ADMIN.pass }, { username_or_email: ADMIN.user, password: ADMIN.pass }]) {
    const r = await fetch(`${b}/auth/login`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
    const text = await r.text()
    if (!r.ok) { rec.note(`login with ${Object.keys(body)[0]}: ${r.status} ${text.slice(0, 120)}`); continue }
    try { const j = JSON.parse(text) as Record<string, unknown>; tok = ((j.tokens as Record<string, unknown> | undefined)?.access_token ?? j.access_token ?? j.token) as string | undefined } catch { /* not JSON */ }
    if (tok) break
    rec.note(`login ok but no token found; keys: ${text.slice(0, 200).replace(/"[A-Za-z0-9._-]{40,}"/g, '"<hidden>"')}`)
  }
  rec.check(!!tok, 'setup: admin can sign in over HTTP')
  const env = { ...NOKEY, MEDIAGIT_TOKEN: tok ?? '' }
  const root = join(WORK, 'S-13'); rmSync(root, { recursive: true, force: true }); mkdirSync(root, { recursive: true })
  const a = join(root, 'a'); mkdirSync(a)
  const mk = await rec.cli(root, ['remote', 'repos', 'create', 'film', '--server', b], { env })
  let made = mk.code === 0
  if (!made) {
    const r = await fetch(`${b}/admin/repos`, { method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${tok}` }, body: JSON.stringify({ name: 'film' }) })
    rec.note(`CLI repos create: exit ${mk.code} ${flat(mk.stdout + mk.stderr).slice(-200)} | POST /admin/repos: ${r.status} ${(await r.text()).slice(0, 200)}`)
    made = r.ok
  }
  rec.check(made, 'a repository "film" is created on the S3-backed server')
  const want = await copyCorpus(a, { [BIG]: 'shot.mxf', 'freepik__talk__71826.jpeg': 'still.jpeg', 'creative-pairs/ai/banner_v1.ai': 'banner.ai' })
  await rec.cli(a, ['init'], { env }); await rec.cli(a, ['add', '--all'], { env }); await rec.cli(a, ['commit', '-m', 'v1 on s3'], { env })
  await rec.cli(a, ['remote', 'add', 'origin', `${b}/film`], { env })
  const push = await rec.cli(a, ['push', '-u', 'origin', 'main'], { env })
  rec.check(push.code === 0, `push 1.3 GB to the S3-backed server (${push.ms} ms, peak ${push.peakMb.toFixed(0)} MB)`, flat(push.stdout + push.stderr).slice(-300))
  const objs = mc('ls', '--recursive', `s/${S3.bucket}/${PREFIX}/`).trim().split('\n').filter(Boolean)
  rec.note(`objects under s3://${S3.bucket}/${PREFIX}/ after the push: ${objs.length}; first: ${objs[0]?.trim().slice(0, 160) ?? '(none)'}`)
  rec.check(objs.length > 0 && !/mcli failed/.test(objs[0] ?? ''), 'the pushed data is in the bucket under the chosen folder', `${objs.length} objects`)
  const onDisk = tree(join(SRV, 'repos')).reduce((n, f) => n + statSync(f).size, 0)
  rec.note(`bytes under the server folder's repos/ after the push: ${onDisk}`)
  rec.check(onDisk < 50e6, 'the server folder did not also store the media on local disk', `${(onDisk / 1e6).toFixed(1)} MB`)
  const c = join(root, 'c')
  const clone = await rec.cli(root, ['clone', `${b}/film`, c], { env })
  rec.check(clone.code === 0, `clone back from the S3-backed server (${clone.ms} ms)`, flat(clone.stdout + clone.stderr).slice(-300))
  await hashCompare(rec, c, want, 'clone from S3')
  await intCheck(rec, c)
  rec.check(buckets() === before, 'still no new bucket', buckets())

  await rec.step('Storage shown in the app', async () => { await page.reload(); await page.waitForTimeout(3000) })
  rec.note(`Server page text: ${flat(await page.locator('#app').innerText()).slice(0, 900)}`)
  await rec.shot('server-page').catch(() => {})
  await rec.step('Stop server', async () => { await page.getByRole('button', { name: 'Stop server…' }).click(); await page.waitForTimeout(500); await page.getByRole('button', { name: 'Stop server', exact: true }).click(); await page.waitForTimeout(4000) })
  rec.check(!(await fetch(`${b}/health`).then((r) => r.ok).catch(() => false)), 'the server stops')
  rec.note(`left in Silo for you to clear: s3://${S3.bucket}/${PREFIX}/ (${objs.length} objects)`)
  await rec.finish(); await closeTerm()
})
