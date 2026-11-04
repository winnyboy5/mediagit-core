import { mkdirSync } from 'node:fs'
import { test, type Page } from '@playwright/test'
import { healthy, mockIpc } from './ipc-mock'

// W5 review screenshots: `SHOTS=1 pnpm e2e w5shots`. Skipped in normal runs.
// Output: ui/.impeccable/review/w5-<state>-<w>-<theme>.png. Every state is its own test (fresh page).
test.skip(!process.env.SHOTS, 'screenshots only on demand')

const SIZES = [
  { w: 1440, h: 900 },
  { w: 1024, h: 700 },
]
const URL_ = 'http://127.0.0.1:4100'
const status = (state: string, extra: Record<string, unknown> = {}) => ({
  state,
  url: state === 'not_set_up' ? null : URL_,
  port: state === 'not_set_up' ? null : 4100,
  pid: state === 'running' ? 4242 : null,
  data_dir: 'C:\\Users\\mira\\AppData\\Roaming\\dev.mediagit.desktop\\server',
  log_path: 'server.log',
  lan: false,
  ...extra,
})
const me = (role: string | null) => ({
  action: 'status',
  server: URL_,
  auth_enabled: true,
  logged_in: role !== null,
  credential_tier: 'stored login',
  identity: role ? { username: 'mira', email: 'mira@studio.example', role, grants: [{ repo: 'hero-film', level: 'write' }] } : null,
  removed: null,
})
const op = (extra: Record<string, unknown> = {}) => ({ action: 'x', server: URL_, id: null, name: null, role: null, repo: null, level: null, key: null, permissions: [], keys: [], users: [], note: null, ...extra })
const LOG = `2026-11-04T09:12:01Z INFO mediagit_server: listening on 127.0.0.1:4100\n2026-11-04T09:12:01Z INFO mediagit_server: authentication enabled\n2026-11-04T09:14:22Z INFO request: GET /healthz 200`
const RUN = {
  server_local_status: status('running'),
  server_local_log: LOG,
  auth_run: me('admin'),
  server_users_list: op({ users: [{ id: 'u1', username: 'mira', role: 'admin' }, { id: 'u2', username: 'ana', role: 'write' }, { id: 'u3', username: 'omar', role: 'read' }] }),
  server_keys_list: op({ keys: [{ id: 'k1', name: 'build machine', created_at: 1793000000 }] }),
  server_repos_list: {
    server: URL_,
    repos: [
      { name: 'hero-film', size_bytes: 4_800_000_000, object_count: 1200, last_modified: Math.floor(Date.now() / 1000) - 7200 },
      { name: 'trailer-cut', size_bytes: 0, object_count: 0, last_modified: 0 },
    ],
  },
}

async function shot(page: Page, name: string, theme: string, w: number) {
  mkdirSync('.impeccable/review', { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(350)
  await page.screenshot({ path: `.impeccable/review/w5-${name}-${w}-${theme}.png` })
}

async function open(page: Page, commands: Record<string, unknown>, tab?: string, storage?: Record<string, string>) {
  await mockIpc(page, { commands: healthy(commands), storage })
  await page.goto('/#/server', { waitUntil: 'domcontentloaded' })
  await page.getByRole('heading', { name: 'Server', level: 1 }).waitFor()
  if (tab) await page.getByRole('tab', { name: tab }).click()
}

const DISABLED = {
  ...RUN,
  server_users_list: op({ users: [{ id: 'u1', username: 'mira', role: 'admin' }, { id: 'u2', username: 'ana', role: 'write' }, { id: 'u3', username: 'omar', role: 'read', disabled: true }] }),
}
const PICKED = {
  ...RUN,
  server_user_grants: op({ grants: [{ repo: 'trailer-cut', level: 'read' }, { repo: 'hero-film', level: 'write' }] }),
  server_repo_access: op({ access: [{ user_id: 'u2', username: 'ana', level: 'write' }, { user_id: 'u3', username: 'omar', level: 'read' }] }),
}
const REMOTE = { 'mg.servers': JSON.stringify(['https://mediagit.studio.example']), 'mg.server.target': JSON.stringify('https://mediagit.studio.example') }

for (const { w, h } of SIZES) {
  test.describe(`${w}`, () => {
    test.use({ viewport: { width: w, height: h } })
    const cases: [string, Record<string, unknown>, string | undefined, Record<string, string> | undefined, ((p: Page) => Promise<void>)?][] = [
      ['not-set-up', { server_local_status: status('not_set_up') }, undefined, undefined],
      ['setup-dialog', { server_local_status: status('not_set_up') }, undefined, undefined, async (p) => void (await p.getByRole('button', { name: 'Set up local server…' }).click())],
      ['stopped', { server_local_status: status('stopped') }, undefined, undefined],
      ['running', RUN, undefined, undefined],
      ['elsewhere', { ...RUN, server_local_status: status('elsewhere') }, undefined, undefined],
      ['remote-overview', { server_local_status: status('stopped'), auth_run: me('admin') }, undefined, REMOTE],
      ['people', RUN, 'People', undefined],
      ['people-member', { server_local_status: status('stopped'), auth_run: me('write') }, 'People', REMOTE],
      ['people-down', { server_local_status: status('stopped') }, 'People', undefined],
      ['access', RUN, 'Access', undefined],
      ['people-disabled', DISABLED, 'People', undefined],
      ['people-disable-dialog', DISABLED, 'People', undefined, async (p) => void (await p.getByRole('button', { name: 'Stop ana signing in' }).click())],
      [
        'access-picked',
        PICKED,
        'Access',
        undefined,
        async (p) => {
          await p.getByRole('combobox', { name: 'Person' }).click()
          await p.getByRole('option', { name: /^ana/ }).click()
          await p.getByRole('combobox', { name: 'Repository' }).click()
          await p.getByRole('option', { name: 'hero-film' }).click()
          await p.getByTestId('current-level').waitFor()
        },
      ],
      ['keys', RUN, 'API keys', undefined],
      ['repos', RUN, 'Repositories', undefined],
      ['repos-delete', RUN, 'Repositories', undefined, async (p) => void (await p.getByRole('button', { name: 'Delete hero-film' }).click())],
    ]
    for (const [name, commands, tab, storage, after] of cases) {
      test(name, async ({ page }, info) => {
        await open(page, commands, tab, storage)
        await after?.(page)
        await shot(page, name, info.project.name, w)
      })
    }
  })
}
