import { test } from '@playwright/test'
import './demo-env'
import { startChapter } from './demo'
import { copyMedia, folder, seedRepo } from './seed'
import { go, openRepo, toWelcome } from '../qa-live/gui'

// Chapter 11: the server's storage form for local S3 (Silo), AWS, Google Cloud and Azure.
// Credentials come only from the environment (campaign_env.ps1 for the clouds; MG_DEMO_SILO_KEY / MG_DEMO_SILO_SECRET
// for the local Silo). Nothing is typed from a literal. Objects, if any, land under demo prefixes in the allowlisted
// buckets that clean_after_run_cloud.ps1 empties; Silo's prefix is listed by the caller.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
// Local-only Silo dev pair: env first, else read from the S-13 spec (the same way run_demo.ps1 does for its wipe); never written here.
const s13 = (() => { try { return readFileSync(join(process.cwd(), 'tests', 'e2e', 'qa-live', 'S-13-silo-storage.spec.ts'), 'utf8').match(/key: '([^']+)', secret: '([^']+)'/) } catch { return null } })()
const env = (n: string) => process.env[n] ?? (n === 'MG_DEMO_SILO_KEY' ? s13?.[1] : n === 'MG_DEMO_SILO_SECRET' ? s13?.[2] : '') ?? ''
const SILO_PREFIX = 'demo-video-silo'

const need = ['AWS_ACCESS_KEY_ID', 'AWS_SECRET_ACCESS_KEY', 'MG_QA_AWS_BUCKET', 'GCS_PROJECT_ID', 'GCS_BUCKET_NAME', 'GOOGLE_APPLICATION_CREDENTIALS',
  'AZURE_STORAGE_ACCOUNT', 'AZURE_STORAGE_KEY', 'MG_QA_AZURE_CONTAINER', 'MG_DEMO_SILO_KEY', 'MG_DEMO_SILO_SECRET']

test('11 Store on S3 or any cloud', async () => {
  const missing = need.filter((n) => !env(n))
  if (missing.length) throw new Error(`chapter 11 needs these environment variables: ${missing.join(', ')}`)
  const ch = await startChapter('11', 'Store on S3 or any cloud')
  const { page } = ch
  const dlg = page.getByRole('dialog')
  const field = (label: string) => dlg.getByLabel(label, { exact: true })
  const prefix = (id: string) => `qa-live/demo/${id}`

  /** Open the setup form on a given storage kind with the admin details filled in. */
  const open = async (kind: string) => {
    await page.getByRole('button', { name: 'Set up local server…' }).click()
    await dlg.waitFor()
    await dlg.getByLabel('Admin username').fill('studio-admin')
    await dlg.getByLabel('Admin email').fill('admin@studio.test')
    await dlg.getByLabel('Password', { exact: true }).fill('Studio!Demo-2026')
    await dlg.getByLabel('Repeat password').fill('Studio!Demo-2026')
    await dlg.getByText(kind, { exact: true }).click()
    await ch.pace(700)
  }
  /** Blur identifier fields, then fill the storage fields. */
  const fill = async (hide: string[], values: Record<string, string>) => {
    for (const l of hide) await ch.blur(field(l))
    for (const [l, v] of Object.entries(values)) await field(l).fill(v)
    await ch.pace(900)
  }
  const probe = async () => {
    await dlg.getByRole('button', { name: 'Test connection' }).click()
    await page.getByTestId('storage-ok').waitFor({ timeout: 120_000 })
    await ch.pace(1200)
  }

  try {
    const dir = folder('hero-film')
    copyMedia(dir, '3D_model_of_Shucaris_ankylosskelos_appendage.png')
    seedRepo(dir, 'First look')
    await toWelcome(page, true)
    await openRepo(page, dir)
    await go(page, 'team/server')
    await ch.pace(2000)
    await ch.caption('Keep your files wherever suits your studio: on a cloud, or on your own storage', 3500)

    await open('Amazon S3 or MinIO')
    await fill(['Access key ID', 'Bucket', 'Folder inside the bucket'], {
      Bucket: env('MG_QA_AWS_BUCKET'), Region: env('AWS_DEFAULT_REGION') || 'us-east-1', 'Folder inside the bucket': prefix('aws'),
      'Access key ID': env('AWS_ACCESS_KEY_ID'), 'Secret key': env('AWS_SECRET_ACCESS_KEY'),
    })
    await ch.caption('Amazon S3: enter your bucket and keys. Secrets are always hidden', 3500)
    await probe()
    await ch.caption('Test connection checks the details before anything is saved', 3000)
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await ch.pace(800)

    await open('Google Cloud')
    await fill(['Project', 'Service account key file', 'Bucket', 'Folder inside the bucket'], {
      Bucket: env('GCS_BUCKET_NAME'), Project: env('GCS_PROJECT_ID'), 'Folder inside the bucket': prefix('gcs'),
      'Service account key file': env('GOOGLE_APPLICATION_CREDENTIALS'),
    })
    await ch.caption('Google Cloud works the same way', 3000)
    await probe()
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await ch.pace(800)

    await open('Azure Blob')
    await fill(['Storage account', 'Container', 'Folder inside the container'], {
      'Storage account': env('AZURE_STORAGE_ACCOUNT'), Container: env('MG_QA_AZURE_CONTAINER'), 'Folder inside the container': prefix('azure'),
      'Account key': env('AZURE_STORAGE_KEY'),
    })
    await ch.caption('So does Microsoft Azure', 3000)
    await probe()
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await ch.pace(800)

    await open('Amazon S3 or MinIO')
    await fill(['Access key ID'], {
      Bucket: 'mediagit-qa-suite', Region: 'us-east-1', Endpoint: 'http://127.0.0.1:9000', 'Folder inside the bucket': SILO_PREFIX,
      'Access key ID': env('MG_DEMO_SILO_KEY'), 'Secret key': env('MG_DEMO_SILO_SECRET'),
    })
    await ch.caption('Prefer your own hardware? Any S3-compatible storage in your building works too', 3500)
    await probe()
    await dlg.getByRole('button', { name: 'Set up and start' }).click()
    await page.waitForFunction(() => /running/i.test((document.querySelector('[data-testid="local-state"]') as HTMLElement | null)?.innerText ?? ''), null, { timeout: 90_000 })
    await ch.pace(1200)
    await ch.caption('One click and your server stores everything on that storage', 3500)
  } finally {
    try {
      await page.getByRole('button', { name: 'Stop server…' }).click()
      await page.getByRole('button', { name: 'Stop server', exact: true }).click()
      await ch.pace(3000)
    } catch { /* server not running: nothing to stop */ }
    console.log('clip:', await ch.stop())
  }
})
