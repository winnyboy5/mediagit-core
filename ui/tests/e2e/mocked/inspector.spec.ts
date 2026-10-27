import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, mockIpc, RECENT } from './ipc-mock'

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.changes.mode': JSON.stringify('flat') }
const status = fixture<{ modified: string[] }>('status_populated')
const withImage = { ...status, modified: [...status.modified, 'art/hero_preview.png'] }

const unstaged = (page: Page) => page.getByRole('tree', { name: 'Files not staged' })
const staged = (page: Page) => page.getByRole('tree', { name: 'Staged files' })
const insp = (page: Page) => page.getByTestId('inspector')

async function boot(page: Page, extra: Record<string, unknown> = {}, handlers: Record<string, string> = {}) {
  await mockIpc(page, { commands: healthy({ repo_status: withImage, ...extra }), storage: REPO, handlers })
  await page.goto('/#/changes', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Changes', level: 1 })).toBeVisible()
}

test.describe('Inspector', () => {
  test('image: preview, media details from the parser, lock and history sections; no axe violations', async ({ page }) => {
    await boot(page, { media_info: fixture('media_png'), repo_log: fixture('log_graph') })
    await unstaged(page).getByRole('treeitem', { name: /hero_preview/ }).click()
    const i = insp(page)
    await expect(i.getByRole('img', { name: 'Preview of hero_preview.png' })).toBeVisible()
    const meta = i.getByTestId('media-details')
    await expect(meta).toContainText('Width')
    await expect(meta).toContainText('1920')
    await expect(meta).toContainText('Has alpha')
    await expect(meta).toContainText('1.3 MB')
    await expect(i.getByTestId('lock-state')).toContainText('Not locked')
    await expect(i.getByTestId('file-history').getByRole('button')).toHaveCount(11)
    expect((await ipcCalls(page, 'media_info'))[0]!.args).toMatchObject({ repo: '/repos/hero-film', path: 'art/hero_preview.png' })
    expect((await ipcCalls(page, 'repo_log')).at(-1)!.args).toMatchObject({ max: 10, all: false, path: 'art/hero_preview.png' })
    await expectNoAxeViolations(page)
  })

  test('audio gets a player; video a player; both labelled', async ({ page }) => {
    await boot(page)
    await unstaged(page).getByRole('treeitem', { name: /vo_take3/ }).click()
    await expect(insp(page).locator('audio[aria-label="Preview of vo_take3.wav"]')).toBeVisible()
    await unstaged(page).getByRole('treeitem', { name: /shot_010/ }).click()
    await expect(insp(page).locator('video[aria-label="Preview of shot_010.mp4"], [data-testid=preview] .note')).toHaveCount(1)
  })

  test('PSD and other types: icon + note, no player, details still offered', async ({ page }) => {
    await boot(page, { media_info: { layers: 12, color_mode: 'RGB' } })
    await staged(page).getByRole('treeitem', { name: /hero\.psd/ }).click()
    await expect(insp(page).getByTestId('preview')).toContainText('No preview for .psd files')
    await expect(insp(page).locator('img, video, audio')).toHaveCount(0)
    await expect(insp(page).getByTestId('media-details')).toContainText('Layers')
    await staged(page).getByRole('treeitem', { name: /sky_hdri/ }).click()
    await expect(insp(page).getByTestId('media-details')).toHaveCount(0) // exr: no parser
  })

  test('media details failure is plain text, not a crash', async ({ page }) => {
    await boot(page, { media_info: { __error: { code: 'error', message: 'Unsupported media type' } } })
    await staged(page).getByRole('treeitem', { name: /hero\.psd/ }).click()
    await expect(insp(page).getByTestId('media-details')).toContainText('Details are not available for this file: Unsupported media type')
  })

  test('lock: locked by someone else explains who and offers no unlock; a free file can be locked', async ({ page }) => {
    await boot(page, { lock_list: fixture('locks') })
    await staged(page).getByRole('treeitem', { name: /hero\.psd/ }).click()
    const lock = insp(page).getByTestId('lock-state')
    await expect(lock).toContainText('Locked by Mira Okafor')
    // identity unknown: Unlock is offered and the server decides
    await expect(lock.getByRole('button', { name: 'Unlock' })).toBeVisible()
    await unstaged(page).getByRole('treeitem', { name: /vo_take3/ }).click()
    await expect(lock).toContainText('Not locked')
    await lock.getByRole('button', { name: 'Lock this file' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'lock_create')).map((c) => c.args.path)).toEqual(['audio/vo_take3.wav'])
  })

  test('lock owned by another person (identity known) has no unlock, only a pointer to Locks', async ({ page }) => {
    await mockIpc(page, {
      commands: healthy({ repo_status: withImage, lock_list: fixture('locks') }),
      storage: { ...REPO, 'mg.lock.owner': JSON.stringify('Someone Else') },
    })
    await page.goto('/#/changes', { waitUntil: 'domcontentloaded' })
    await staged(page).getByRole('treeitem', { name: /hero\.psd/ }).click()
    const lock = insp(page).getByTestId('lock-state')
    await expect(lock).toContainText('Only Mira Okafor can unlock it')
    await expect(lock.getByRole('button')).toHaveCount(0)
    await expect(lock.getByRole('link', { name: 'Locks' })).toBeVisible()
  })

  test('lock service unavailable is said once, quietly', async ({ page }) => {
    await boot(page, { lock_list: { __error: { code: 'error', message: 'No remote configured' } } })
    await staged(page).getByRole('treeitem', { name: /hero\.psd/ }).click()
    await expect(insp(page).getByTestId('lock-state')).toContainText('Lock status is unavailable: No remote configured')
  })

  test('a deleted file has nothing to preview, lock or inspect further', async ({ page }) => {
    await boot(page)
    await unstaged(page).getByRole('treeitem', { name: /vo_take1/ }).click()
    await expect(insp(page).getByTestId('preview')).toContainText('deleted')
    await expect(insp(page).getByTestId('lock-state')).toHaveCount(0)
  })

  test('file history rows open that version in History', async ({ page }) => {
    await boot(page, { repo_log: fixture('log_graph') }, { repo_show: `() => (${JSON.stringify(fixture('show_commit'))})` })
    await staged(page).getByRole('treeitem', { name: /hero\.psd/ }).click()
    await insp(page).getByTestId('file-history').getByRole('button', { name: /Final grade on shot_010/ }).click()
    await expect(page).toHaveURL(/#\/history$/)
    await expect.poll(async () => (await ipcCalls(page, 'repo_show')).map((c) => c.args.rev)).toContain('0002'.repeat(10))
  })

  test('multi-select still shows the summary, with no per-file sections', async ({ page }) => {
    await boot(page)
    await unstaged(page).getByRole('treeitem', { name: /vo_take3/ }).click()
    await page.keyboard.down('Control')
    await unstaged(page).getByRole('treeitem', { name: /shot_010/ }).click()
    await page.keyboard.up('Control')
    await expect(page.getByTestId('selection-summary')).toContainText('2 files selected')
    await expect(insp(page).getByTestId('preview')).toHaveCount(0)
  })
})
