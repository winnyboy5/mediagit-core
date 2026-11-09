import { mkdirSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import { fixture, healthy, ipcRelease, mockIpc, RECENT } from './ipc-mock'
import { openSection } from './nav'

// W4c review screenshots: every hub section at 1440, both themes. `SHOTS=1 pnpm e2e w4cshots`.
// Output: ui/.impeccable/review/W4C-<hub>-<section>-1440-<theme>.png
test.skip(!process.env.SHOTS, 'screenshots only on demand')
test.use({ viewport: { width: 1440, height: 900 } })

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.changes.mode': JSON.stringify('tree') }
// ls-files leaves ignored files out unless asked.
const LS = `(a) => ({ files: ${JSON.stringify(fixture<{ files: unknown[] }>('lsfiles_populated').files)}.filter((f) => a.ignored || f.status !== 'ignored') })`
const SECTIONS = [
  'files/all',
  'files/changes',
  'history/versions',
  'history/compare',
  'history/tags',
  'branches/branches',
  'branches/merge',
  'branches/sync',
  'branches/stash',
  'team/locks',
  'team/remote',
  'team/server',
  'settings/general',
  'settings/storage',
  'settings/maintenance',
  'settings/security',
  'settings/advanced',
]

// A version with several media files, so the thumbnail grid has something to show.
const SHOW_MEDIA = {
  ...fixture<Record<string, unknown>>('show_commit'),
  files: [
    { path: 'art/hero_preview.png', status: 'modified', old_size: 1153433, new_size: 1363148 },
    { path: 'art/hero_sketch.jpg', status: 'added', old_size: null, new_size: 482000 },
    { path: 'art/logo.png', status: 'modified', old_size: 90000, new_size: 94000 },
    { path: 'scenes/keep_exterior.blend', status: 'modified', old_size: 9000000, new_size: 9100000 },
    { path: 'audio/vo_take1.wav', status: 'deleted', old_size: 12582912, new_size: null },
    { path: 'video/shot_010.mp4', status: 'modified', old_size: 398458880, new_size: 421527552 },
  ],
}
const SHOW = `(a) => a.tree ? ${JSON.stringify(fixture('show_tree'))} : ${JSON.stringify(SHOW_MEDIA)}`
const EXTRACT = `() => ({ path: '/tmp/mediagit-preview/p.png', size: 1000 })`
// A running local server with a signed-in admin, so the Overview has data rather than a skeleton.
const SERVER = {
  server_local_status: { state: 'running', url: 'http://127.0.0.1:4100', port: 4100, pid: 4242, data_dir: 'C:\\Users\\mira\\AppData\\Roaming\\dev.mediagit.desktop\\server', log_path: 'C:\\server.log', lan: false },
  server_local_log: 'listening on 127.0.0.1:4100',
  auth_run: { action: 'status', server: 'http://127.0.0.1:4100', auth_enabled: true, logged_in: true, credential_tier: 'stored login', identity: { username: 'mira', email: 'mira@studio.example', role: 'admin', grants: [] }, removed: null },
  server_repos_list: { server: 'http://127.0.0.1:4100', repos: [{ name: 'hero-film', size_bytes: 4_800_000_000, object_count: 1200, last_modified: 1793000000 }] },
}

for (const path of SECTIONS) {
  test(`hub section ${path}`, async ({ page }, info) => {
    await mockIpc(page, {
      commands: healthy({
        repo_log: fixture('log_graph'),
        tag_list: fixture('tags'),
        stash_list: fixture('stash_list'),
        config_list: fixture('config_list'),
        config_schema: fixture('config_schema'),
        branch_list: fixture('branches'),
        lock_list: fixture('locks'),
        remote_list: fixture('remotes'),
        ...(path === 'team/server' ? SERVER : {}),
      }),
      handlers: { repo_ls_files: LS, repo_show: SHOW, repo_extract: EXTRACT },
      storage: REPO,
    })
    await openSection(page, path)
    if (path === 'team/server') await expect(page.getByTestId('local-state')).toBeVisible()
    if (path === 'history/versions') await expect(page.getByRole('list', { name: 'Files this version changed' }).getByRole('listitem').first()).toBeVisible()
    await page.waitForTimeout(600)
    await page.evaluate(() => document.fonts.ready)
    mkdirSync('.impeccable/review', { recursive: true })
    await page.screenshot({ path: `.impeccable/review/W4C-${path.replace('/', '-')}-1440-${info.project.name}.png` })
  })
}

// All files with hero.psd selected: the preview and details panel is part of the layout.
test('files/all with a file selected', async ({ page }, info) => {
  await mockIpc(page, { commands: healthy(), handlers: { repo_ls_files: LS, repo_extract: EXTRACT }, storage: REPO })
  await openSection(page, 'files/all')
  await page.getByRole('treeitem', { name: /hero\.psd/ }).first().click()
  await expect(page.getByTestId('inspector')).toBeVisible()
  await page.waitForTimeout(600)
  await page.evaluate(() => document.fonts.ready)
  await page.screenshot({ path: `.impeccable/review/W4C-files-all-selected-1440-${info.project.name}.png` })
})

// Staging in progress: files done / total, bytes, rate and time left (real progress fixture, two bars).
test('files/changes while staging', async ({ page }, info) => {
  const script = [
    { progress: { id: 3, msg: 'env/sky_hdri.exr', pos: 0, len: 4_000_000_000, unit: 'bytes', done: false } },
    { progress: { id: 4, msg: 'files', pos: 0, len: 6, unit: 'items', done: false } },
    { delay: 1100, progress: { id: 3, msg: 'env/sky_hdri.exr', pos: 1_000_000_000, len: 4_000_000_000, unit: 'bytes', done: false } },
    { progress: { id: 4, msg: 'files', pos: 2, len: 6, unit: 'items', done: false } },
    { delay: 1200, progress: { id: 3, msg: 'env/fog_vol.vdb', pos: 2_000_000_000, len: 4_000_000_000, unit: 'bytes', done: false } },
    { progress: { id: 4, msg: 'files', pos: 3, len: 6, unit: 'items', done: false } },
    { hold: true },
    { resolve: null },
  ]
  await mockIpc(page, { commands: healthy(), storage: REPO, scripts: { repo_add: script } })
  await openSection(page, 'files/changes')
  await page.getByRole('button', { name: 'Include all', exact: true }).click()
  await expect(page.getByTestId('pill-files')).toBeVisible()
  await page.evaluate(() => document.fonts.ready)
  await page.screenshot({ path: `.impeccable/review/W4C-files-staging-1440-${info.project.name}.png` })
  await ipcRelease(page)
})
