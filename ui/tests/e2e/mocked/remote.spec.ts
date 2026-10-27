import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, ipcRelease, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const OUT = '/Users/Mira/Desktop'
const entry = (page: Page, path: string) => page.locator(`[data-entry="${path}"]`)
const pane = (page: Page) => page.getByTestId('remote-pane')

// Root, `video`, and an empty folder, picked by the requested directory.
const lsHandler = `(a) => a.dir === null ? ${JSON.stringify(fixture('ls_root'))} : a.dir === 'video' ? ${JSON.stringify(fixture('ls_video'))} : { ...${JSON.stringify(fixture('ls_empty'))}, path: a.dir }`
const previewHandler = `(a) => ({ path: '/preview/' + a.remotePath.split('/').pop(), size: 1363148 })`
const TWO = { remotes: [...fixture<{ remotes: unknown[] }>('remotes').remotes, { name: 'backup', url: 'https://backup.example/hero-film' }] }

interface Boot {
  commands?: Record<string, unknown>
  handlers?: Record<string, string>
  scripts?: Record<string, unknown[]>
  dialog?: string | null
}

async function boot(page: Page, b: Boot = {}) {
  await mockIpc(page, {
    commands: healthy({ remote_list: fixture('remotes'), branch_list: fixture('branches'), tag_list: fixture('tags'), stash_list: fixture('stash_empty'), ...b.commands }),
    handlers: { remote_ls_tree: lsHandler, repo_download_preview: previewHandler, ...b.handlers },
    scripts: b.scripts as never,
    dialog: b.dialog === undefined ? OUT : b.dialog,
    storage: REPO,
  })
  await page.goto('/#/remote', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Remote Browser', level: 1 })).toBeVisible()
}

test.describe('Remote Browser: states', () => {
  test('populated: folders first, then files; breadcrumb at the root; no axe violations', async ({ page }) => {
    await boot(page)
    const rows = page.getByRole('list', { name: 'Remote folders and files' }).getByRole('listitem')
    await expect(rows).toHaveCount(4)
    await expect(rows).toHaveText([/art/, /video/, /hero\.png/, /README\.md/])
    await expect(page.getByRole('navigation', { name: 'Folder path' }).getByRole('button', { name: 'origin' })).toHaveAttribute('aria-current', 'location')
    await expect(entry(page, 'hero.png')).toContainText('1.3 MB')
    await expect(pane(page)).toContainText('Pick a file')
    await expect(page.getByText('Download saves one file.')).toBeVisible()
    expect((await ipcCalls(page, 'remote_ls_tree'))[0]!.args).toEqual({ repo: '/repos/hero-film', remote: 'origin', dir: null, gitRef: null })
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton until the first listing arrives', async ({ page }) => {
    await boot(page, { handlers: { remote_ls_tree: '' }, scripts: { remote_ls_tree: [{ hold: true }, { resolve: fixture('ls_root') }] } })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(entry(page, 'hero.png')).toBeVisible()
  })

  test('empty: no remote configured, with a way to add one', async ({ page }) => {
    await boot(page, { commands: { remote_list: { remotes: [] } } })
    await expect(page.getByText('No remote configured')).toBeVisible()
    await expect(page.getByRole('link', { name: 'Go to Sync' })).toBeVisible()
    await expect(page.getByLabel('Remote', { exact: true })).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('empty: a folder with nothing in it', async ({ page }) => {
    await boot(page)
    await entry(page, 'video').click()
    await entry(page, 'video/shots').click()
    await expect(page.getByText('This folder is empty.')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('error: the listing fails, Retry asks again', async ({ page }) => {
    await boot(page, { handlers: { remote_ls_tree: `() => { throw { code: 'error', message: 'The server did not return a folder listing.' } }` } })
    await expect(page.getByRole('alert')).toContainText('The server did not return a folder listing')
    await expectNoAxeViolations(page)
    const before = (await ipcCalls(page, 'remote_ls_tree')).length
    await page.getByRole('button', { name: 'Retry' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_ls_tree')).length).toBeGreaterThan(before)
  })
})

test.describe('Remote Browser: navigation', () => {
  test('opening folders adds breadcrumbs, one level per request; crumbs go back', async ({ page }) => {
    await boot(page)
    await entry(page, 'video').click()
    await expect(entry(page, 'video/shot_010.mp4')).toBeVisible()
    const crumbs = page.getByRole('navigation', { name: 'Folder path' })
    await expect(crumbs.getByRole('button', { name: 'video' })).toHaveAttribute('aria-current', 'location')
    await entry(page, 'video/shots').click()
    await expect(crumbs.getByRole('button', { name: 'shots' })).toBeVisible()
    expect((await ipcCalls(page, 'remote_ls_tree')).map((c) => c.args.dir).slice(-2)).toEqual(['video', 'video/shots'])
    await crumbs.getByRole('button', { name: 'origin' }).click()
    await expect(entry(page, 'hero.png')).toBeVisible()
    expect((await ipcCalls(page, 'remote_ls_tree')).at(-1)!.args.dir).toBeNull()
    await expectNoAxeViolations(page)
  })

  test('choosing a version (branch or tag) lists that ref from the root', async ({ page }) => {
    await boot(page)
    await entry(page, 'video').click()
    await page.getByLabel('Version').click()
    await expect(page.getByRole('option')).toHaveText(['Default branch', 'main', 'feature/grade', 'review/cut-2', 'v0.3 (tag)', 'v0.2 (tag)', 'v0.1 (tag)'])
    await page.getByRole('option', { name: 'v0.3 (tag)' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_ls_tree')).at(-1)?.args).toEqual({ repo: '/repos/hero-film', remote: 'origin', dir: null, gitRef: 'v0.3' })
  })

  test('another remote is listed by its own name', async ({ page }) => {
    await boot(page, { commands: { remote_list: TWO } })
    await page.getByLabel('Remote', { exact: true }).click()
    await page.getByRole('option', { name: 'backup' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_ls_tree')).at(-1)?.args.remote).toBe('backup')
  })

  test('folder rows are keyboard reachable with a visible focus ring', async ({ page }) => {
    await boot(page)
    await entry(page, 'art').focus()
    await expectFocusRing(page)
    await page.keyboard.press('Tab')
    await expect(entry(page, 'video')).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(entry(page, 'video/shot_010.mp4')).toBeVisible()
  })
})

test.describe('Remote Browser: preview and download', () => {
  test('an image is downloaded to the preview folder and shown', async ({ page }) => {
    await boot(page)
    await entry(page, 'hero.png').click()
    await expect(pane(page).getByRole('img', { name: 'Preview of hero.png' })).toBeVisible()
    await expect(pane(page)).toContainText('hero.png')
    expect((await ipcCalls(page, 'repo_download_preview'))[0]!.args).toMatchObject({ repo: '/repos/hero-film', remotePath: 'hero.png', gitRef: null })
    await expectNoAxeViolations(page)
  })

  test('audio gets a player', async ({ page }) => {
    await boot(page)
    await entry(page, 'video').click()
    await entry(page, 'video/vo.wav').click()
    await expect(pane(page).locator('audio')).toBeAttached()
  })

  test('a type with no preview says so and fetches nothing', async ({ page }) => {
    await boot(page)
    await entry(page, 'README.md').click()
    await expect(pane(page)).toContainText('No preview for this kind of file')
    expect(await ipcCalls(page, 'repo_download_preview')).toHaveLength(0)
    await expect(pane(page).getByRole('button', { name: 'Download to…' })).toBeEnabled()
  })

  test('busy: the preview shows a bytes bar while it loads', async ({ page }) => {
    const slow = [{ progress: { id: 0, msg: 'Downloading', pos: 40, len: 100, unit: 'bytes', done: false } }, { hold: true }, { resolve: { path: '/preview/hero.png', size: 100 } }]
    await boot(page, { handlers: { repo_download_preview: '' }, scripts: { repo_download_preview: slow } })
    await entry(page, 'hero.png').click()
    const bar = page.getByTestId('preview-progress')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '40')
    await expectNoAxeViolations(page)
    await ipcRelease(page)
    await expect(bar).toHaveCount(0)
    await expect(pane(page).getByRole('img', { name: 'Preview of hero.png' })).toBeVisible()
  })

  test('a failed preview is explained; downloading is still possible', async ({ page }) => {
    await boot(page, { handlers: { repo_download_preview: `() => { throw { code: 'error', message: 'Not found on the server.' } }` } })
    await entry(page, 'hero.png').click()
    await expect(pane(page).getByRole('alert')).toContainText('Could not load the preview: Not found on the server.')
    await expectNoAxeViolations(page)
    await expect(pane(page).getByRole('button', { name: 'Download to…' })).toBeEnabled()
  })

  test('Download to… sends the chosen folder and file name', async ({ page }) => {
    await boot(page, { commands: { repo_download: fixture('download_ok') } })
    await entry(page, 'video').click()
    await entry(page, 'video/shot_010.mp4').click()
    await pane(page).getByRole('button', { name: 'Download to…' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_download')).map((c) => c.args)).toMatchObject([
      { repo: '/repos/hero-film', remotePath: 'video/shot_010.mp4', gitRef: null, output: `${OUT}/shot_010.mp4` },
    ])
    await expect(page.getByText('Download shot_010.mp4').first()).toBeVisible()
  })

  test('busy: a download shows bytes progress and can be cancelled', async ({ page }) => {
    await boot(page, { scripts: { repo_download: fixture('download_progress') } })
    await entry(page, 'video').click()
    await entry(page, 'video/shot_010.mp4').click()
    await pane(page).getByRole('button', { name: 'Download to…' }).click()
    const bar = page.getByTestId('download-progress')
    await expect(bar).toContainText('Download shot_010.mp4')
    await expect(bar).toContainText('500 MB of 2.1 GB')
    await expect(bar.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '2254857830')
    await expectNoAxeViolations(page)
    await bar.getByRole('button', { name: 'Cancel' }).click()
    await expect(bar).toHaveCount(0)
    expect(await ipcCalls(page, 'cancel_op')).toHaveLength(1)
    await expect(pane(page).getByRole('alert')).toHaveCount(0)
    await expect(pane(page).getByRole('button', { name: 'Download to…' })).toBeEnabled()
  })

  test('a failed download is shown plainly', async ({ page }) => {
    await boot(page, { commands: { repo_download: { __error: { code: 'error', message: 'Disk is full.' } } } })
    await entry(page, 'README.md').click()
    await pane(page).getByRole('button', { name: 'Download to…' }).click()
    await expect(pane(page).getByRole('alert')).toContainText('Disk is full.')
    await expectNoAxeViolations(page)
  })

  test('cancelling the folder picker downloads nothing', async ({ page }) => {
    await boot(page, { dialog: null })
    await entry(page, 'README.md').click()
    await pane(page).getByRole('button', { name: 'Download to…' }).click()
    await page.waitForTimeout(200)
    expect(await ipcCalls(page, 'repo_download')).toHaveLength(0)
  })

  test('a remote other than origin is addressed by its full URL', async ({ page }) => {
    await boot(page, { commands: { remote_list: TWO, repo_download: fixture('download_ok') } })
    await page.getByLabel('Remote', { exact: true }).click()
    await page.getByRole('option', { name: 'backup' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_ls_tree')).at(-1)?.args.remote).toBe('backup')
    await entry(page, 'README.md').click()
    await pane(page).getByRole('button', { name: 'Download to…' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_download')).map((c) => c.args.remotePath)).toEqual(['https://backup.example/hero-film/README.md'])
  })
})
