import { mkdirSync } from 'node:fs'
import { test, type Page } from '@playwright/test'
import { fixture, healthy, mockIpc, RECENT } from './ipc-mock'

// W4b review screenshots: `SHOTS=1 pnpm e2e w4bshots`. Skipped in normal runs.
// Output: ui/.impeccable/review/w4b-<view>-<state>-<w>-<theme>.png. Every state is its own test.
test.skip(!process.env.SHOTS, 'screenshots only on demand')

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const SIZES = [
  { w: 1440, h: 900 },
  { w: 1024, h: 700 },
]
const SHOWN = { name: 'v0.3', oid: '0000000000000000000000000000000000000000', annotated: true, message: 'Rig v2 export', tagger_name: 'Mira O', tagger_email: 'mira@studio.example', date: '2026-11-01T10:00:00+00:00', signed: true }
const RANGE = { start: 'aaa1111', end: 'bbb2222', commits: 4, verified: 3, errors: 1, problems: ['Object 12ab34 is missing from storage'] }

async function shot(page: Page, name: string, theme: string, w: number) {
  mkdirSync('.impeccable/review', { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(350)
  await page.screenshot({ path: `.impeccable/review/w4b-${name}-${w}-${theme}.png` })
}

async function open(page: Page, route: string, heading: string, commands: Record<string, unknown>) {
  await mockIpc(page, { commands: healthy(commands), storage: REPO })
  await page.goto(`/#/${route}`, { waitUntil: 'domcontentloaded' })
  await page.getByRole('heading', { name: heading, level: 1 }).waitFor()
}

type Step = (p: Page) => Promise<void>
const open1 = (loc: (p: Page) => ReturnType<Page['locator']>): Step => async (p) => void (await loc(p).locator('summary').first().click())

const STASH_ENTRY = { index: 0, oid: 'aa', message: 'WIP lighting', branch: 'main', timestamp: '2026-11-01T10:00:00+00:00' }
const tags = {
  tag_list: fixture('tags'),
  stash_list: fixture('stash_list'),
  tag_show: SHOWN,
  stash_show: { action: 'show', index: 0, oid: 'aa', message: 'WIP lighting', files_updated: null, stashes: [STASH_ENTRY], paths: [] },
}
const maint = { repo_gc: fixture('gc_dry'), repo_fsck: { ...fixture<Record<string, unknown>>('fsck_ok'), repaired: 2 }, repo_verify: RANGE }
const sec = (auth: string) => ({ auth_run: fixture(auth), key_status: fixture('key_on') })

const settings = {
  config_list: {
    action: 'list',
    entries: [
      { key: 'author.name', value: 'Mira Okafor', secret: false },
      { key: 'author.email', value: 'mira@studio.example', secret: false },
      { key: 'performance.upload_concurrency', value: '8', secret: false },
      { key: 'performance.download_concurrency', value: null, secret: false },
      { key: 'storage.create_dirs', value: null, secret: false },
      { key: 'storage.secret_access_key', value: 'set', secret: true },
      { key: 'remotes.origin.url', value: 'https://mediagit.studio.example/hero-film', secret: false },
      { key: 'remotes.origin.token', value: null, secret: true },
    ],
  },
  config_schema: fixture('config_schema'),
  sparse_list: fixture('sparse_off'),
}

const cases: [string, string, string, Record<string, unknown>, Step?][] = [
  ['settings', 'settings-form', 'Settings', settings, async (p) => void (await p.getByTestId('config').scrollIntoViewIfNeeded())],
  ['settings', 'settings-invalid', 'Settings', settings, async (p) => {
    const row = p.locator('[data-key="performance.download_concurrency"]')
    await row.getByLabel('Files received at once').fill('0')
    await row.scrollIntoViewIfNeeded()
  }],
  ['tags', 'tags-more', 'Tags & Stash', tags, async (p) => {
    const f = p.locator('form', { has: p.getByLabel('Tag name') })
    await f.locator('summary').click()
    await p.getByLabel('Tag name').fill('v0.3')
    await f.getByRole('checkbox', { name: /Replace the tag/ }).click()
  }],
  ['tags', 'tags-detail', 'Tags & Stash', tags, async (p) => void (await p.getByRole('button', { name: 'Details of tag v0.3' }).click())],
  ['tags', 'stash-files', 'Tags & Stash', tags, open1((p) => p.locator('section[aria-labelledby="stash-h"] details'))],
  ['tags', 'stash-options', 'Tags & Stash', tags, async (p) => {
    await p.getByRole('button', { name: 'Options for stash 0' }).click()
    await p.getByRole('checkbox', { name: /Overwrite my current changes/ }).click()
  }],
  ['maintenance', 'maintenance-gc', 'Maintenance', maint, open1((p) => p.locator('[data-task="gc"]'))],
  ['maintenance', 'maintenance-fsck-repair', 'Maintenance', maint, async (p) => {
    const c = p.locator('[data-task="fsck"]')
    await c.locator('summary').click()
    await c.getByRole('checkbox', { name: /Try to repair/ }).click()
    await c.getByRole('checkbox', { name: /Only show what a repair/ }).click()
  }],
  ['maintenance', 'maintenance-fsck-confirm', 'Maintenance', maint, async (p) => {
    const c = p.locator('[data-task="fsck"]')
    await c.locator('summary').click()
    await c.getByRole('checkbox', { name: /Try to repair/ }).click()
    await c.getByRole('checkbox', { name: /Only show what a repair/ }).click()
    await c.getByRole('button', { name: 'Repair…' }).click()
  }],
  ['maintenance', 'maintenance-verify-range', 'Maintenance', maint, async (p) => {
    const c = p.locator('[data-task="verify"]')
    await c.locator('summary').click()
    await c.getByLabel('From version').fill('aaa1111')
    await c.getByLabel('Up to version').fill('HEAD')
    await c.getByRole('button', { name: 'Verify' }).click()
    await p.getByTestId('verify-range-result').waitFor()
  }],
  ['security', 'security-register', 'Security', sec('auth_out'), async (p) => void (await p.getByRole('button', { name: 'Create account…' }).click())],
  ['security', 'security-password', 'Security', sec('auth_in'), async (p) => void (await p.getByRole('button', { name: 'Change password…' }).click())],
  ['security', 'security-rotate', 'Security', sec('auth_in'), async (p) => void (await p.getByRole('button', { name: 'Replace master key…' }).click())],
]

for (const { w, h } of SIZES) {
  test.describe(`${w}`, () => {
    test.use({ viewport: { width: w, height: h } })
    for (const [route, name, heading, commands, after] of cases) {
      test(name, async ({ page }, info) => {
        await open(page, route, heading, commands)
        await after?.(page)
        await shot(page, name, info.project.name, w)
      })
    }
  })
}
