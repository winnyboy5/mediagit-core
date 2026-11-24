import { execSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { OUT, WORK, launchApp, quitApp } from './env'
import { Recorder, closeTerm } from './rec'
import { flat, go, store, toWelcome, visibleErrors } from './gui'
import { copyCorpus, hashCompare, intCheck } from './integrity'

// C-cloud: the app's Team > Server storage form and a cloud-backed server, on AWS S3, GCS and Azure.
// Small payload only (about 6 MB). Objects land under qa-live/<RunId>/<backend> and nowhere else.
// Credentials come only from the process environment (dev-tests/qa-suite/scripts/campaign_env.ps1,
// dot-sourced by run_qalive.ps1). A backend with a missing variable records SKIP, never PASS.
// Covered: storage form, Test connection (good + bad), Save and start, restart, use through the server
// (push, clone, second client, tag, lock, download, hash parity, fsck), broken credential while in use.
// Not covered: repos whose storage.backend is set directly in the repo (plan Part 3 group 6).
const SRV = join(WORK, 'app-server')
const ADMIN = { user: 'qaadmin', email: 'qaadmin@example.test', pass: 'Qa-Live!2026-admin' }
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
const RAW = /os error|panicked|exit code|stderr|\bErr\(|dispatch failure|SdkError|status code|HTTP \d{3}|\b40[0-9]\b|\b50[0-9]\b/i
const env = (n: string) => process.env[n] ?? ''
const GOOGLE_KEY_FILE = env('GOOGLE_APPLICATION_CREDENTIALS')
// The form gets a copy inside work\, so the wrong-credential case can wreck the copy and never the real key file.
const KEY_COPY = join(WORK, 'gcs-key-copy.json')

// Secret values that must never reach a log line, a note or the page.
const gcsParts = (): string[] => {
  try {
    const j = JSON.parse(readFileSync(GOOGLE_KEY_FILE, 'utf8')) as { private_key_id?: string; private_key?: string }
    const line = (j.private_key ?? '').split('\n').find((x) => x.length === 64) ?? ''
    return [j.private_key_id ?? '', j.private_key ?? '', line]
  } catch { return [] }
}
const SECRETS = () => [env('AWS_SECRET_ACCESS_KEY'), env('AZURE_STORAGE_KEY'), ...gcsParts()].filter((s) => s.length >= 8)
// The app and the server it starts must not inherit cloud credentials: the form is the only source.
const CLOUD_ENV = ['AWS_ACCESS_KEY_ID', 'AWS_SECRET_ACCESS_KEY', 'AWS_SESSION_TOKEN', 'AWS_PROFILE', 'AWS_DEFAULT_REGION', 'AWS_REGION', 'AZURE_STORAGE_ACCOUNT', 'AZURE_STORAGE_KEY', 'AZURE_STORAGE_CONNECTION_STRING', 'AZURE_STORAGE_SAS_TOKEN', 'GOOGLE_APPLICATION_CREDENTIALS', 'GOOGLE_CLOUD_PROJECT', 'GCS_PROJECT_ID', 'GCS_BUCKET_NAME', 'CLOUDSDK_CONFIG']
let appClean = false
const scrub = (s: string) => SECRETS().reduce((t, v) => t.split(v).join('<hidden>'), s)

const sh = (cmd: string, extra: Record<string, string> = {}): { ok: boolean; out: string } => {
  try { return { ok: true, out: execSync(cmd, { env: { ...process.env, ...extra }, encoding: 'utf8', windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], timeout: 120_000 }) } }
  catch (e) { const x = e as { stdout?: string; stderr?: string }; return { ok: false, out: `${x.stdout ?? ''}${x.stderr ?? ''}` } }
}

// gcloud behind a TLS-inspecting AV needs the Windows roots (same trick as clean_after_run_cloud.ps1).
let gcloudEnv: Record<string, string> | null = null
function gcloud(): Record<string, string> {
  if (gcloudEnv) return gcloudEnv
  const dir = join(WORK, 'gcloud'); mkdirSync(dir, { recursive: true })
  const pem = join(dir, 'roots.pem')
  const ps = `$sb=New-Object Text.StringBuilder; foreach($s in 'Cert:\\LocalMachine\\Root','Cert:\\CurrentUser\\Root'){foreach($c in Get-ChildItem $s -EA SilentlyContinue){[void]$sb.AppendLine('-----BEGIN CERTIFICATE-----');[void]$sb.AppendLine([Convert]::ToBase64String($c.RawData,'InsertLineBreaks'));[void]$sb.AppendLine('-----END CERTIFICATE-----')}}; [IO.File]::WriteAllText('${pem}',$sb.ToString())`
  writeFileSync(join(dir, 'mkpem.ps1'), ps)
  sh(`powershell -NoProfile -File "${join(dir, 'mkpem.ps1')}"`)
  gcloudEnv = { CLOUDSDK_CONFIG: dir, CLOUDSDK_CORE_CUSTOM_CA_CERTS_FILE: pem, CLOUDSDK_CORE_DISABLE_PROMPTS: '1' }
  sh(`gcloud auth activate-service-account --key-file="${GOOGLE_KEY_FILE}"`, gcloudEnv)
  return gcloudEnv
}

interface Backend {
  id: 'aws' | 'gcs' | 'azure'
  kind: string
  need: string[]
  folder: string
  /** Fill the storage form. `o` overrides single fields by label. */
  fill: (dlg: ReturnType<Page['getByRole']>, prefix: string, o?: Record<string, string>) => Promise<void>
  /** Object names under the per-run prefix (read-only cloud CLI list). null when the listing itself failed. */
  list: (prefix: string) => string[] | null
  /** Bad variants for Test connection: [name, overrides, hint]. */
  bad: [string, Record<string, string>, RegExp][]
  /** Variants that may legitimately pass: [name, overrides]. Only the wording is checked when they fail. */
  soft: [string, Record<string, string>][]
  /** Does a bucket of this name exist? (read-only) */
  hasBucket?: (name: string) => boolean
  dropBucket?: (name: string) => void
  /** Rewrite a server config file so the cloud credential is wrong. */
  breakCfg: (text: string) => string
}

const MISSING = `mediagit-qa-nonexistent-${randomUUID().slice(0, 12)}`
const aws: Backend = {
  id: 'aws', kind: 'Amazon S3 or MinIO', folder: 'Folder inside the bucket',
  need: ['AWS_ACCESS_KEY_ID', 'AWS_SECRET_ACCESS_KEY', 'MG_QA_AWS_BUCKET'],
  fill: async (d, prefix, o = {}) => {
    const v = { Bucket: env('MG_QA_AWS_BUCKET'), Region: env('AWS_DEFAULT_REGION') || 'us-east-1', Endpoint: '', 'Folder inside the bucket': prefix, 'Access key ID': env('AWS_ACCESS_KEY_ID'), 'Secret key': env('AWS_SECRET_ACCESS_KEY'), ...o }
    for (const [k, val] of Object.entries(v)) await d.getByLabel(k, { exact: true }).fill(val)
  },
  list: (p) => { const r = sh(`aws s3 ls "s3://${env('MG_QA_AWS_BUCKET')}/${p}/" --recursive`); return r.ok ? r.out.split('\n').filter((l) => l.trim()) : r.out.trim() === '' ? [] : null },
  bad: [
    ['wrong secret', { 'Secret key': 'wrong-secret-value-for-qa' }, /key|secret|credential|sign|access|denied|permission|auth/i],
    // A random name nobody owns: must be refused in plain words (and must not be created: checked below).
    ['bucket that does not exist', { Bucket: MISSING }, /bucket|exist|found|access|denied|permission/i],
    ['dead endpoint', { Endpoint: 'http://127.0.0.1:9' }, /reach|connect|endpoint|address|server/i],
  ],
  soft: [['wrong region', { Region: 'eu-west-1' }]],
  hasBucket: (n) => sh(`aws s3api head-bucket --bucket ${n}`).ok,
  dropBucket: (n) => { sh(`aws s3 rb s3://${n} --force`) },
  breakCfg: (t) => t.split(env('AWS_SECRET_ACCESS_KEY')).join('wrong-secret-value-for-qa'),
}
const gcs: Backend = {
  id: 'gcs', kind: 'Google Cloud', folder: 'Folder inside the bucket',
  need: ['GCS_PROJECT_ID', 'GCS_BUCKET_NAME', 'GOOGLE_APPLICATION_CREDENTIALS'],
  fill: async (d, prefix, o = {}) => {
    const v = { Bucket: env('GCS_BUCKET_NAME'), Project: env('GCS_PROJECT_ID'), 'Folder inside the bucket': prefix, 'Service account key file': KEY_COPY, ...o }
    for (const [k, val] of Object.entries(v)) await d.getByLabel(k, { exact: true }).fill(val)
  },
  list: (p) => { const r = sh(`gcloud storage ls -r "gs://${env('GCS_BUCKET_NAME')}/${p}/**"`, gcloud()); return r.ok ? r.out.split('\n').filter((l) => /^gs:\/\//.test(l) && !l.endsWith('/')) : /matched no objects/.test(r.out) ? [] : null },
  bad: [
    ['missing key file', { 'Service account key file': 'D:\\qa-no-such-dir\\no-key.json' }, /key|file|credential|find|read|exist/i],
    ['bucket that does not exist', { Bucket: MISSING }, /bucket|exist|found|access|denied|permission/i],
  ],
  soft: [['wrong project', { Project: 'qa-no-such-project-0' }]],
  hasBucket: (n) => sh(`gcloud storage buckets describe gs://${n}`, gcloud()).ok,
  dropBucket: (n) => { sh(`gcloud storage rm -r gs://${n} --quiet`, gcloud()) },
  breakCfg: (t) => t,
}
const azure: Backend = {
  id: 'azure', kind: 'Azure Blob', folder: 'Folder inside the container',
  need: ['AZURE_STORAGE_ACCOUNT', 'AZURE_STORAGE_KEY', 'MG_QA_AZURE_CONTAINER'],
  fill: async (d, prefix, o = {}) => {
    const v = { 'Storage account': env('AZURE_STORAGE_ACCOUNT'), Container: env('MG_QA_AZURE_CONTAINER'), 'Folder inside the container': prefix, 'Account key': env('AZURE_STORAGE_KEY'), 'Or a connection string': '', ...o }
    for (const [k, val] of Object.entries(v)) await d.getByLabel(k, { exact: true }).fill(val)
  },
  list: (p) => { const r = sh(`az storage blob list --container-name "${env('MG_QA_AZURE_CONTAINER')}" --prefix "${p}/" --num-results "*" --query "[].name" -o tsv --only-show-errors`); return r.ok ? r.out.split('\n').filter((l) => l.trim()) : null },
  // A wrong container is not tried: the server may create a missing container in the real account.
  bad: [
    ['wrong account key', { 'Account key': 'd3Jvbmcta2V5LWZvci1xYQ==' }, /key|credential|sign|access|denied|auth/i],
    ['garbage connection string', { 'Account key': '', 'Storage account': '', 'Or a connection string': 'not-a-connection-string' }, /connection|string|account|invalid|credential/i],
  ],
  soft: [],
  breakCfg: (t) => t.split(env('AZURE_STORAGE_KEY')).join('d3Jvbmcta2V5LWZvci1xYQ=='),
}

const tree = (dir: string, out: string[] = []): string[] => { for (const n of existsSync(dir) ? readdirSync(dir) : []) { const p = join(dir, n); if (statSync(p).isDirectory()) tree(p, out); else out.push(p) } return out }
const local = (page: Page) => store<{ state: string; url: string | null } | null>(page, 'server', 's.local')
const running = (page: Page) => page.waitForFunction(() => /running/i.test((document.querySelector('[data-testid="local-state"]') as HTMLElement | null)?.innerText ?? ''), null, { timeout: 90_000 }).then(() => true, () => false)

async function login(b: string): Promise<string | undefined> {
  for (const body of [{ username: ADMIN.user, password: ADMIN.pass }, { identifier: ADMIN.user, password: ADMIN.pass }, { email: ADMIN.email, password: ADMIN.pass }]) {
    const r = await fetch(`${b}/auth/login`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
    if (!r.ok) continue
    try { const j = (await r.json()) as Record<string, unknown>; return ((j.tokens as Record<string, unknown> | undefined)?.access_token ?? j.access_token ?? j.token) as string | undefined } catch { /* next shape */ }
  }
  return undefined
}

for (const B of [aws, gcs, azure]) {
  test(`C-cloud-${B.id} server storage on ${B.id} from the app`, async () => {
    const missing = B.need.filter((n) => !env(n))
    // Restart the app without cloud credentials in its environment (once per run), before attaching.
    if (!missing.length && !appClean) { mkdirSync(join(WORK, 'empty-appdata'), { recursive: true }); await quitApp(); await launchApp({ APPDATA: join(WORK, 'empty-appdata') }, CLOUD_ENV); appClean = true }
    const rec = await Recorder.start(`C-cloud-${B.id}`, `${B.id}-storage`, { gui: true })
    if (missing.length) { rec.skip(`${B.id}: environment variable(s) not set: ${missing.join(', ')} (run through run_qalive.ps1, which dot-sources campaign_env.ps1)`); await rec.finish(); await closeTerm(); return }
    const page = rec.page!
    const prefix = `qa-live/${process.env.MG_QA_DAY ?? 'adhoc'}/${B.id}`
    const dlg = page.getByRole('dialog')
    const saved = new Map<string, string>()
    let keyGood: Buffer | null = null
    const cfgFiles = () => tree(SRV).filter((f) => statSync(f).size < 2e6 && /\.(toml|json|ya?ml|env)$/i.test(f))
    const cli = (cwd: string, args: string[], e: Record<string, string> = {}) => rec.cli(cwd, args, { env: e, timeoutMs: 300_000 })
    const until = async (pred: () => Promise<boolean>, ms: number) => { const t = Date.now(); while (Date.now() - t < ms) { if (await pred().catch(() => false)) return true; await page.waitForTimeout(500) } return false }
    const stop = async () => {
      await page.getByRole('button', { name: 'Stop server…' }).click(); await page.waitForTimeout(500)
      await page.getByRole('button', { name: 'Stop server', exact: true }).click()
      await until(async () => (await local(page))?.state === 'stopped', 30_000)
    }
    try {
      if (B.id === 'gcs') copyFileSync(GOOGLE_KEY_FILE, KEY_COPY)
      rec.note(`prefix ${prefix}; payload about 6 MB; the app and its server run without cloud credentials or the user's APPDATA (no ambient Google sign-in); each case wipes the app-server folder and sets up fresh`)
      const fill = async (o: Record<string, string> = {}) => { await B.fill(dlg, prefix, o); await page.waitForTimeout(250) }
      const probe = async (name: string) => {
        const t = Date.now()
        await dlg.getByRole('button', { name: 'Test connection' }).click()
        const res = page.locator('[data-testid="storage-ok"], [data-testid="storage-bad"]').first()
        await res.waitFor({ timeout: 120_000 }).catch(() => {})
        const ok = await page.getByTestId('storage-ok').isVisible().catch(() => false)
        const text = scrub(flat(await res.innerText().catch(() => '(no result shown)')))
        rec.note(`Test connection, ${name}: ${ok ? 'OK' : 'FAILED'} in ${Date.now() - t} ms: ${text}`)
        return { ok, text, ms: Date.now() - t }
      }

      await toWelcome(page)
      await go(page, 'team/server'); await page.waitForTimeout(2500)
      if ((await local(page))?.state === 'running') await rec.step('Stop the server left by an earlier case', stop)
      if ((await local(page))?.state !== 'running') for (const n of existsSync(SRV) ? readdirSync(SRV) : []) rmSync(join(SRV, n), { recursive: true, force: true })
      await page.reload(); await page.waitForTimeout(2500)

      const chose = await rec.step(`Set up local server, choose "${B.kind}"`, async () => {
        await page.getByRole('button', { name: 'Set up local server…' }).click(); await page.waitForTimeout(600)
        await dlg.getByLabel('Admin username').fill(ADMIN.user); await dlg.getByLabel('Admin email').fill(ADMIN.email)
        await dlg.getByLabel('Password', { exact: true }).fill(ADMIN.pass); await dlg.getByLabel('Repeat password').fill(ADMIN.pass)
        await dlg.getByText(B.kind, { exact: true }).click(); await page.waitForTimeout(500)
        return true
      })
      if (!chose) return

      // 1. Form: empty values are refused with a plain message; secrets are password inputs.
      const secretLabels = B.id === 'aws' ? ['Secret key'] : B.id === 'azure' ? ['Account key', 'Or a connection string'] : []
      for (const l of secretLabels) rec.check((await dlg.getByLabel(l, { exact: true }).getAttribute('type')) === 'password', `form: "${l}" is a password field`)
      const e0 = await probe('empty form')
      rec.check(!e0.ok && !RAW.test(e0.text) && e0.text.length > 0, 'form: Test on an empty form fails in plain words', e0.text)

      // 2. Test connection: bad variants, soft variants, then good.
      for (const [name, o, hint] of B.bad) {
        await fill(o)
        const r = await probe(name)
        rec.check(!r.ok, `${name}: Test connection says it failed`, r.text)
        rec.check(!RAW.test(r.text), `${name}: the reason is in plain words`, r.text)
        rec.check(hint.test(r.text), `${name}: the reason points at the right thing`, r.text)
        rec.check(r.ms < 60_000, `${name}: answers within 60 s`, `${r.ms} ms`)
        if (o.Bucket === MISSING && B.hasBucket) {
          const made = B.hasBucket(MISSING)
          rec.check(!made, 'Test connection did not create the missing bucket in the real account', made ? 'it exists now' : undefined)
          if (made) B.dropBucket?.(MISSING)
        }
      }
      for (const [name, o] of B.soft) {
        await fill(o)
        const r = await probe(name)
        if (!r.ok) rec.check(!RAW.test(r.text), `${name}: when it fails, the reason is in plain words`, r.text)
        else rec.note(`${name}: Test connection said OK (accepted: the cloud may not care)`)
      }
      await fill()
      const good = await probe('good values')
      rec.check(good.ok, 'good values (credential only from the form): Test connection says OK', good.text)
      const probeLeft = B.list(prefix)
      rec.check(probeLeft !== null, 'the cloud listing works (read-only check of the prefix)')
      const leftN = probeLeft?.length ?? 0
      rec.note(`objects under the prefix after Test connection: ${probeLeft?.length ?? 'unknown'}`)

      // 3. Save and start.
      await rec.step('Set up and start', async () => {
        await dlg.getByRole('button', { name: 'Set up and start' }).click()
        await running(page)
      })
      const l = await local(page)
      rec.note(`after set up: ${JSON.stringify(l)} | alerts ${scrub(JSON.stringify(await visibleErrors(page))).slice(0, 300)}`)
      rec.check(l?.state === 'running', `the server is running on ${B.id} storage`, JSON.stringify(l))
      if (l?.state !== 'running' || !l.url) return
      const b = l.url.replace(/\/$/, '')

      const bucket = env(B.id === 'aws' ? 'MG_QA_AWS_BUCKET' : B.id === 'gcs' ? 'GCS_BUCKET_NAME' : 'MG_QA_AZURE_CONTAINER')
      const cfgHits = cfgFiles().filter((f) => readFileSync(f, 'utf8').includes(bucket))
      rec.check(cfgHits.length > 0, `the saved server config names the ${B.id} bucket/container`, cfgHits.map((f) => f.replace(SRV, '')).join(', '))
      rec.check(cfgHits.some((f) => readFileSync(f, 'utf8').includes(prefix)), 'the saved server config names the per-run folder', prefix)
      if (B.id === 'gcs') {
        const norm = (x: string) => x.replaceAll('\\', '/').toLowerCase()
        rec.check(cfgFiles().some((f) => norm(readFileSync(f, 'utf8')).includes(norm(KEY_COPY))), 'the saved server config names the key file typed in the form (not ambient Google sign-in)', KEY_COPY.replace(WORK, ''))
      }
      // Secret leak scan: the app log, the server log and the page must not hold a secret value.
      const logs = [...tree(join(WORK, 'logs')), ...tree(SRV).filter((f) => /\.log$/i.test(f))].filter((f) => statSync(f).size < 50e6)
      const leaked = logs.filter((f) => SECRETS().some((s) => readFileSync(f, 'utf8').includes(s)))
      rec.check(leaked.length === 0, 'no secret value in the app log or the server log', leaked.map((f) => f.replace(WORK, '')).join(', '))
      const html = await page.content()
      rec.check(!SECRETS().some((s) => html.includes(s)), 'no secret value in the page after saving')

      // 4. Use the cloud-backed server (CLI against it, as S-13 does).
      const tok = await login(b)
      rec.check(!!tok, 'setup: admin can sign in over HTTP')
      const cenv = { ...NOKEY, MEDIAGIT_TOKEN: tok ?? '' }
      const root = join(WORK, `C-cloud-${B.id}`); rmSync(root, { recursive: true, force: true }); mkdirSync(root, { recursive: true })
      const a = join(root, 'a'); mkdirSync(a)
      const mk = await cli(root, ['remote', 'repos', 'create', 'film', '--server', b], cenv)
      rec.check(mk.code === 0, 'a hosted repository "film" is created', scrub(flat(mk.stdout + mk.stderr).slice(-300)))
      const want = await copyCorpus(a, { '101394-video-720.mp4': 'shot.mp4', 'freepik__talk__71826.jpeg': 'still.jpeg', 'creative-pairs/ai/banner_v1.ai': 'banner.ai' })
      for (const args of [['init'], ['add', '--all'], ['commit', '-m', `v1 on ${B.id}`], ['remote', 'add', 'origin', `${b}/film`]]) await cli(a, args, cenv)
      const push = await cli(a, ['push', '-u', 'origin', 'main'], cenv)
      rec.check(push.code === 0, `push to the ${B.id}-backed server (${push.ms} ms)`, scrub(flat(push.stdout + push.stderr).slice(-300)))
      const objs = B.list(prefix)
      rec.check(!!objs && objs.length > leftN, 'the pushed data is in the cloud under the per-run prefix (more than any probe leftover)', `${objs?.length ?? 'listing failed'} objects, ${leftN} before the push`)
      const onDisk = tree(join(SRV, 'repos')).reduce((n, f) => n + statSync(f).size, 0)
      rec.check(onDisk < 3e6, 'the server folder did not also keep the media on local disk', `${(onDisk / 1e6).toFixed(1)} MB`)
      const c = join(root, 'c')
      const clone = await cli(root, ['clone', `${b}/film`, c], cenv)
      rec.check(clone.code === 0, `clone back from the ${B.id}-backed server (${clone.ms} ms)`, scrub(flat(clone.stdout + clone.stderr).slice(-300)))
      await hashCompare(rec, c, want, `clone from ${B.id}`)
      await intCheck(rec, c)
      // Second client: a second commit from a pushes, c pulls it.
      writeFileSync(join(a, 'notes.txt'), `second version on ${B.id}\n`)
      await cli(a, ['add', 'notes.txt'], cenv); await cli(a, ['commit', '-m', 'v2 notes'], cenv)
      const p2 = await cli(a, ['push', 'origin', 'main'], cenv)
      rec.check(p2.code === 0, 'second push (new version)', scrub(flat(p2.stdout + p2.stderr).slice(-300)))
      const pull = await cli(c, ['pull'], cenv)
      rec.check(pull.code === 0 && existsSync(join(c, 'notes.txt')), 'a second client pulls the new version', scrub(flat(pull.stdout + pull.stderr).slice(-300)))
      await cli(a, ['tag', 'create', 'v1.0'], cenv)
      const tp = await cli(a, ['push', 'origin', '--tags'], cenv)
      rec.check(tp.code === 0, 'tag push', scrub(flat(tp.stdout + tp.stderr).slice(-300)))
      const lk = await cli(a, ['lock', 'create', 'banner.ai'], cenv)
      const ll = await cli(a, ['lock', 'list'], cenv)
      rec.check(lk.code === 0 && ll.code === 0 && /banner\.ai/.test(ll.stdout), 'lock a file and see it listed', scrub(flat(lk.stdout + lk.stderr + ll.stdout).slice(-300)))
      const ul = await cli(a, ['lock', 'unlock', 'banner.ai'], cenv)
      const ll2 = await cli(a, ['lock', 'list'], cenv)
      rec.check(ul.code === 0 && ll2.code === 0 && !/banner\.ai/.test(ll2.stdout), 'unlock the file', scrub(flat(ul.stdout + ul.stderr + ll2.stdout).slice(-300)))
      const dl = join(root, 'dl-still.jpeg')
      const dn = await cli(c, ['download', 'still.jpeg', '-o', dl], cenv)
      rec.check(dn.code === 0 && existsSync(dl), 'single-file download from the cloud-backed server', scrub(flat(dn.stdout + dn.stderr).slice(-300)))
      if (existsSync(dl)) await hashCompare(rec, root, { 'dl-still.jpeg': want['still.jpeg']! }, 'downloaded file')
      const lg = await cli(c, ['log', '--oneline'], cenv)
      const lg2 = await cli(a, ['log', '--oneline'], cenv)
      const n1 = lg.stdout.trim().split('\n').filter(Boolean).length
      const n2 = lg2.stdout.trim().split('\n').filter(Boolean).length
      rec.check(lg.code === 0 && lg2.code === 0 && n1 === 2 && n2 === 2, 'history: both clones show exactly 2 versions', `${n1} / ${n2}`)

      // Restart through the app keeps the backend and the data.
      await page.reload(); await page.waitForTimeout(2500)
      await rec.step('Stop server', stop)
      await rec.step('Start server again', async () => { await page.getByRole('button', { name: /^Start server/ }).click(); await running(page) })
      const l2 = await local(page)
      rec.check(l2?.state === 'running', 'restart: the server runs again on the same storage', JSON.stringify(l2))
      const b2 = (l2?.url ?? b).replace(/\/$/, '')
      const cenv2 = { ...NOKEY, MEDIAGIT_TOKEN: (await login(b2)) ?? '' }
      const c2 = join(root, 'c2')
      const clone2 = await cli(root, ['clone', `${b2}/film`, c2], cenv2)
      rec.check(clone2.code === 0, 'restart: clone after restart works', scrub(flat(clone2.stdout + clone2.stderr).slice(-300)))
      await hashCompare(rec, c2, want, `clone after restart from ${B.id}`)

      // 5. Broken credential while in use: commit locally, stop, wreck the saved credential, start.
      writeFileSync(join(a, 'more.txt'), 'edit made around the credential failure\n')
      await cli(a, ['add', 'more.txt'], cenv); await cli(a, ['commit', '-m', 'v3 around the failure'], cenv)
      await rec.step('Stop server before breaking the credential', stop)
      if (B.id === 'gcs') rec.note('GCS: the wrong credential is made by overwriting the key file copy (KEY_COPY); breakCfg does not edit the config')
      for (const f of cfgFiles()) { const t = readFileSync(f, 'utf8'); const w = B.breakCfg(t); if (w !== t) { saved.set(f, t); writeFileSync(f, w) } }
      if (B.id === 'gcs') { keyGood = readFileSync(KEY_COPY); writeFileSync(KEY_COPY, '{"type":"service_account"}') }
      rec.check(saved.size > 0 || !!keyGood, 'the saved credential was found and replaced with a wrong one', [...saved.keys()].map((f) => f.replace(SRV, '')).join(', ') || KEY_COPY.replace(WORK, ''))
      const cfgLines = [...saved.keys()].flatMap((f) => readFileSync(f, 'utf8').split('\n').filter((x) => /backend|bucket|container|prefix|account|credentials|key_file|region/i.test(x)).map((x) => `${f.replace(SRV, '')}: ${scrub(x.trim())}`))
      rec.note(`config lines (secrets scrubbed): ${cfgLines.join(' ; ').slice(0, 800)}`)
      const startAndWait = async () => {
        const t = Date.now()
        await page.getByRole('button', { name: /^Start server/ }).click()
        // Done when running, or when it went through a non-stopped state and came back to stopped (or stayed stopped 8 s).
        let seen = false
        while (Date.now() - t < 150_000) {
          const st = (await local(page))?.state
          if (st === 'running') break
          if (st && st !== 'stopped') seen = true
          if (st === 'stopped' && (seen || Date.now() - t > 8000)) break
          await page.waitForTimeout(500)
        }
        return { ms: Date.now() - t, state: await local(page), alerts: scrub(JSON.stringify(await visibleErrors(page))).slice(0, 400) }
      }
      const w = await rec.step('Start server with the wrong credential', startAndWait)
      rec.note(`with the wrong credential: ${JSON.stringify(w)}`)
      if (w?.state?.state === 'running' && w.state.url) {
        const b3 = w.state.url.replace(/\/$/, '')
        const cenv3 = { ...NOKEY, MEDIAGIT_TOKEN: (await login(b3)) ?? '' }
        const bp = await cli(a, ['push', 'origin', 'main'], cenv3)
        const msg = scrub(flat(bp.stdout + bp.stderr))
        rec.check(bp.code !== 0, 'push fails while the cloud credential is wrong', msg.slice(-300))
        rec.check(!/panicked|RUST_BACKTRACE/.test(msg) && msg.length > 0, 'the push failure is a message, not a crash', msg.slice(-300))
        rec.check(existsSync(join(a, 'more.txt')), 'local work is intact after the failed push')
        const bc = await cli(root, ['clone', `${b3}/film`, join(root, 'c3')], cenv3)
        rec.check(bc.code !== 0, 'clone fails while the cloud credential is wrong', scrub(flat(bc.stdout + bc.stderr)).slice(-300))
        await rec.step('Stop server to restore the credential', stop)
      } else {
        rec.check(w?.state?.state === 'stopped' && (w?.ms ?? 0) < 60_000, 'the server refuses to start on a wrong credential, and the app says so within 60 s', JSON.stringify(w))
        rec.check(!!w && w.alerts !== '[]' && !RAW.test(w.alerts) && /storage|credential|key|bucket|cloud|access|secret/i.test(w.alerts), 'the app shows an error for the refused start that names the cause (storage/credential)', w?.alerts)
      }
      for (const [f, t] of saved) writeFileSync(f, t)
      saved.clear()
      if (keyGood) { writeFileSync(KEY_COPY, keyGood); keyGood = null }
      const w2 = await rec.step('Start server with the credential restored', startAndWait)
      rec.check(w2?.state?.state === 'running', 'the server runs again once the credential is fixed', JSON.stringify(w2))
      if (w2?.state?.state === 'running' && w2.state.url) {
        const b4 = w2.state.url.replace(/\/$/, '')
        const cenv4 = { ...NOKEY, MEDIAGIT_TOKEN: (await login(b4)) ?? '' }
        const rp = await cli(a, ['push', 'origin', 'main'], cenv4)
        rec.check(rp.code === 0, 'retry after fixing the credential: push succeeds', scrub(flat(rp.stdout + rp.stderr)).slice(-300))
        const c5 = join(root, 'c5')
        const cl5 = await cli(root, ['clone', `${b4}/film`, c5], cenv4)
        rec.check(cl5.code === 0 && existsSync(join(c5, 'more.txt')), 'retry: a fresh clone has the version made around the failure', scrub(flat(cl5.stdout + cl5.stderr)).slice(-300))
        await hashCompare(rec, c5, want, `final clone from ${B.id}`)
        await intCheck(rec, c5)
      }
      rec.note(`left in ${B.id} until clean_after_run_cloud.ps1: ${prefix}/ (${B.list(prefix)?.length ?? '?'} objects)`)
    } catch (e) {
      rec.check(false, 'the case crashed before finishing', scrub(String(e)).slice(0, 600))
    } finally {
      // Always: put the credential back, stop the server, delete the key copy and gcloud state, then record.
      try { for (const [f, t] of saved) writeFileSync(f, t) } catch { /* best effort */ }
      try { if (keyGood) writeFileSync(KEY_COPY, keyGood) } catch { /* best effort */ }
      try { if ((await local(page))?.state === 'running') await stop() } catch { /* best effort */ }
      rmSync(KEY_COPY, { force: true })
      if (B.id === 'gcs') { rmSync(join(WORK, 'gcloud'), { recursive: true, force: true }); gcloudEnv = null }
      const leak2 = tree(join(WORK, 'logs')).filter((f) => SECRETS().some((s) => readFileSync(f, 'utf8').includes(s)))
      rec.check(leak2.length === 0, 'no secret value in the app log after the whole case', leak2.join(', '))
      const own = readFileSync(join(OUT, `C-cloud-${B.id}-${B.id}-storage.log`), 'utf8')
      rec.check(!SECRETS().some((s) => own.includes(s)), 'no secret value in this case\'s own log')
      await rec.finish(); await closeTerm()
    }
  })
}
