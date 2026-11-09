import { expect, test, type Page } from '@playwright/test'
import { expectFocusRing, expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, mockIpc, RECENT } from './ipc-mock'

// W4b: the options behind "More options" in Sync, Welcome, Locks and Changes. Each option must reach the
// mocked command with the right arguments; risky ones must ask first and default to the safe variant.

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT, 'mg.changes.mode': JSON.stringify('flat') }
const PUSHED = { remote: 'origin', result: 'pushed', ref_updates: [], skipped_up_to_date: 0, bytes_uploaded: 0, objects_sent: 0 }
const PULLED = { remote: 'origin', branch: 'main', result: 'up_to_date', old_head: null, new_head: null, tracking_refs_updated: 0, bytes_downloaded: 0, objects_received: 0 }
const TWO = { remotes: [{ name: 'origin', url: 'https://mediagit.studio.example/hero-film' }, { name: 'backup', url: 'https://backup.example/hero' }] }
const THEMES = ['light', 'dark'] as const

const check = (page: Page, name: string) => page.getByRole('checkbox', { name, exact: true })
const last = async (page: Page, cmd: string) => (await ipcCalls(page, cmd)).at(-1)!.args as Record<string, unknown>

async function sync(page: Page, theme: (typeof THEMES)[number] = 'light', commands: Record<string, unknown> = {}) {
  await mockIpc(page, {
    commands: healthy({ remote_list: fixture('remotes'), repo_fetch: fixture('fetch_ok'), repo_pull: PULLED, repo_push: PUSHED, ...commands }),
    storage: { ...REPO, 'mg.theme': JSON.stringify(theme) },
  })
  await page.goto('/#/sync', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Sync', level: 2, exact: true })).toBeVisible()
  await page.getByTestId('sync-more').getByText('More options').click()
}

test.describe('Sync: More options', () => {
  for (const theme of THEMES) {
    test(`disclosure opens by keyboard and has no axe violations (${theme})`, async ({ page }) => {
      await sync(page, theme)
      await expect(page.getByTestId('sync-more')).toHaveAttribute('open', '')
      await expectNoAxeViolations(page)
      await page.getByTestId('sync-more').locator('summary').focus()
      await page.keyboard.press('Shift+Tab')
      await page.keyboard.press('Tab')
      await expectFocusRing(page)
    })
  }

  test('fetch: branch, all branches, prune', async ({ page }) => {
    await sync(page)
    await page.getByTestId('sync-more').getByText('Fetch', { exact: true }).click()
    await page.getByLabel('Only this branch').fill('dev')
    await check(page, 'Forget branches deleted on the remote').click()
    await page.getByRole('button', { name: 'Fetch', exact: true }).first().click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_fetch')).length).toBe(1)
    expect((await last(page, 'repo_fetch')).opts).toEqual({ branch: 'dev', prune: true })
    await check(page, 'Every branch').click()
    await page.getByRole('button', { name: 'Fetch', exact: true }).first().click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_fetch')).length).toBe(2)
    expect((await last(page, 'repo_fetch')).opts).toEqual({ all: true, prune: true })
  })

  test('pull: branch, rebase, preview', async ({ page }) => {
    await sync(page)
    await page.getByTestId('sync-more').getByText('Get latest', { exact: true }).click()
    await page.getByLabel('Only this branch').fill('dev')
    await check(page, 'Replay my versions on top').click()
    await check(page, 'Preview only').click()
    await page.getByRole('button', { name: /^Get latest( \d+)?$/ }).first().click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_pull')).length).toBe(1)
    expect((await last(page, 'repo_pull')).opts).toEqual({ branch: 'dev', rebase: true, dryRun: true })
  })

  test('push: safe options go straight through with no confirmation', async ({ page }) => {
    await sync(page)
    await page.getByLabel('Branches to send').fill('main dev:release')
    for (const n of ['Send tags too', 'Send tags that point at what is sent', 'Preview only', 'Make the remote branch this branch’s upstream', 'Check the remote and re-send damaged data']) await check(page, n).click()
    await page.getByRole('button', { name: /^Send( \d+)?$/ }).first().click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_push')).length).toBe(1)
    expect((await last(page, 'repo_push')).opts).toEqual({
      refspecs: ['main', 'dev:release'],
      tags: true,
      followTags: true,
      dryRun: true,
      setUpstream: true,
      repair: true,
    })
    await expect(page.getByRole('alertdialog')).toHaveCount(0)
  })

  test('push: every branch and no-track', async ({ page }) => {
    await sync(page)
    await check(page, 'Every branch').click()
    await check(page, 'Do not track the remote branch').click()
    await page.getByRole('button', { name: /^Send( \d+)?$/ }).first().click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_push')).length).toBe(1)
    expect((await last(page, 'repo_push')).opts).toEqual({ all: true, noTrack: true })
  })

  test('push: the safe default is "stop if the remote moved"; no overwrite is sent', async ({ page }) => {
    await sync(page)
    await expect(page.getByTestId('push-risk').getByRole('button', { name: 'Stop if the remote moved' })).toHaveAttribute('aria-pressed', 'true')
    await page.getByRole('button', { name: /^Send( \d+)?$/ }).first().click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_push')).length).toBe(1)
    expect((await last(page, 'repo_push')).opts).toEqual({})
  })

  for (const [label, mode, flag] of [
    ['Overwrite safely', 'lease', '--force-with-lease'],
    ['Overwrite anyway', 'force', '--force'],
  ] as const) {
    test(`push: "${label}" asks first, says what is lost, and only then sends confirmed (${flag})`, async ({ page }) => {
      await sync(page)
      await page.getByTestId('push-risk').getByRole('button', { name: label, exact: true }).click()
      await expect(page.getByTestId('push-risk')).toContainText(flag)
      await page.getByRole('button', { name: /^Send( \d+)?$/ }).first().click()
      const dlg = page.getByRole('alertdialog')
      await expect(dlg).toContainText('Overwrite on the remote?')
      await expect(dlg).toContainText(mode === 'force' ? 'are lost from the remote' : 'stops if someone sent new work')
      await expectNoAxeViolations(page)
      await dlg.getByRole('button', { name: 'Cancel' }).click()
      expect(await ipcCalls(page, 'repo_push')).toHaveLength(0)
      await page.getByRole('button', { name: /^Send( \d+)?$/ }).first().click()
      await page.getByRole('alertdialog').getByRole('button', { name: 'Overwrite', exact: true }).click()
      await expect.poll(async () => (await ipcCalls(page, 'repo_push')).length).toBe(1)
      expect((await last(page, 'repo_push')).opts).toEqual({ overwrite: mode, confirmed: true })
    })
  }

  test('push: deleting a remote branch needs its name, then a confirmation', async ({ page }) => {
    await sync(page)
    await check(page, 'Delete these branches on the remote').click()
    await expect(page.getByTestId('push-delete').getByRole('alert')).toContainText('Name the branches to delete')
    await expect(page.getByRole('button', { name: /^Send( \d+)?$/ }).first()).toBeDisabled()
    await page.getByLabel('Branches to send').fill('old-cut')
    await page.getByRole('button', { name: /^Send( \d+)?$/ }).first().click()
    const dlg = page.getByRole('alertdialog')
    await expect(dlg).toContainText('Delete on the remote?')
    await expect(dlg).toContainText('old-cut')
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'repo_push')).toHaveLength(0)
    await page.getByRole('button', { name: /^Send( \d+)?$/ }).first().click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Delete branches' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_push')).length).toBe(1)
    expect((await last(page, 'repo_push')).opts).toEqual({ refspecs: ['old-cut'], delete: true, confirmed: true })
  })
})

test.describe('Sync: remote actions', () => {
  const act = async (page: Page, item: string) => {
    await page.getByRole('button', { name: 'Actions for origin' }).click()
    await page.getByRole('menuitem', { name: item }).click()
  }

  test('add remote: fetch right after and an access token', async ({ page }) => {
    await sync(page)
    await page.getByRole('button', { name: 'Add remote' }).first().click()
    const dlg = page.getByRole('dialog', { name: 'Add remote' })
    await dlg.getByLabel('Name').fill('backup')
    await dlg.getByLabel('Address').fill('https://backup.example/hero')
    await dlg.getByText('More options').click()
    await expectNoAxeViolations(page)
    await dlg.getByRole('checkbox', { name: 'Fetch right after adding' }).click()
    await dlg.getByLabel('Access token').fill('s3cret')
    await expect(dlg.getByLabel('Access token')).toHaveAttribute('type', 'password')
    await dlg.getByRole('button', { name: 'Add remote' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_add')).length).toBe(1)
    expect(await last(page, 'remote_add')).toEqual({ repo: '/repos/hero-film', name: 'backup', url: 'https://backup.example/hero', fetch: true, token: 's3cret' })
  })

  test('details asks the server and shows branches', async ({ page }) => {
    await sync(page, 'light', { remote_show: { name: 'origin', fetch_url: 'https://a.example/r', push_url: 'https://a.example/r', reachable: true, error: null, head_branch: 'main', branches: ['main', 'dev'] } })
    await act(page, 'Details…')
    const d = page.getByTestId('remote-detail')
    await expect(d).toContainText('main, dev')
    await expect(d).toContainText('Yes')
    await expectNoAxeViolations(page)
    expect(await last(page, 'remote_show')).toEqual({ repo: '/repos/hero-film', name: 'origin' })
  })

  test('rename validates and sends old and new', async ({ page }) => {
    await sync(page, 'light', { remote_rename: null })
    await act(page, 'Rename…')
    const dlg = page.getByRole('dialog', { name: 'Rename origin' })
    await dlg.getByLabel('New name').fill('-bad')
    await expect(dlg.getByRole('button', { name: 'Rename', exact: true })).toBeDisabled()
    await dlg.getByLabel('New name').fill('studio')
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_rename')).length).toBe(1)
    expect(await last(page, 'remote_rename')).toEqual({ repo: '/repos/hero-film', old: 'origin', new: 'studio' })
  })

  test('change address, optionally only where versions are sent', async ({ page }) => {
    await sync(page, 'light', { remote_set_url: null })
    await act(page, 'Change address…')
    const dlg = page.getByRole('dialog', { name: 'Change address of origin' })
    await dlg.getByLabel('Address').fill('https://new.example/hero')
    await dlg.getByText('More options').click()
    await dlg.getByRole('checkbox', { name: 'Only where versions are sent' }).click()
    await dlg.getByRole('button', { name: 'Change address' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'remote_set_url')).length).toBe(1)
    expect(await last(page, 'remote_set_url')).toEqual({ repo: '/repos/hero-film', name: 'origin', url: 'https://new.example/hero', push: true })
  })
})

test.describe('Welcome: init and clone options', () => {
  test('new repository: first branch name', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ repo_init: { ok: true } }), dialog: '/repos/new', storage: { 'mg.recent': RECENT } })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /New repository/ }).click()
    const dlg = page.getByRole('dialog', { name: 'Create repository' })
    await dlg.getByText('More options').click()
    await dlg.getByLabel('First branch name').fill('--bare')
    await expect(dlg.getByRole('button', { name: 'Create', exact: true })).toBeDisabled()
    await dlg.getByLabel('First branch name').fill('trunk')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Create', exact: true }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_init')).length).toBe(1)
    expect(await last(page, 'repo_init')).toEqual({ path: '/repos/new', initialBranch: 'trunk' })
  })

  test('clone: branch to check out', async ({ page }) => {
    await mockIpc(page, { commands: healthy(), scripts: { repo_clone: fixture('clone_progress') }, storage: { 'mg.recent': RECENT } })
    await page.goto('/#/welcome')
    await page.getByRole('button', { name: /Clone/ }).click()
    const dlg = page.getByRole('dialog')
    await dlg.getByLabel('Repository URL').fill('https://studio.example/hero-film')
    await dlg.getByLabel('Save into').fill('/repos/hero-film')
    await dlg.getByText('More options').click()
    await dlg.getByLabel('Branch to check out').fill('dev')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Clone', exact: true }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_clone')).length).toBe(1)
    expect(await last(page, 'repo_clone')).toMatchObject({ url: 'https://studio.example/hero-film', dest: '/repos/hero-film', branch: 'dev' })
  })
})

test.describe('Locks: remote and owner', () => {
  for (const theme of THEMES) {
    test(`remote picker and "lock as" reach the commands (${theme})`, async ({ page }) => {
      await mockIpc(page, {
        commands: healthy({ lock_list: fixture('locks'), remote_list: TWO, lock_create: { lock_id: 'l-9', path: 'a.wav', owner: 'ana', created_at: 1793000000 } }),
        storage: { ...REPO, 'mg.theme': JSON.stringify(theme) },
      })
      await page.goto('/#/locks')
      await expect(page.getByRole('heading', { name: 'Locks', level: 2 })).toBeVisible()
      await page.getByRole('combobox', { name: 'Remote' }).click()
      await page.getByRole('option', { name: 'backup' }).click()
      await expect.poll(async () => (await last(page, 'lock_list')).remote).toBe('backup')
      await page.getByRole('button', { name: 'Lock a file' }).click()
      const dlg = page.getByRole('dialog', { name: 'Lock a file' })
      await dlg.getByLabel('File path').fill('audio/a.wav')
      await dlg.getByText('More options').click()
      await dlg.getByLabel('Lock as').fill('ana')
      await expectNoAxeViolations(page)
      await dlg.getByRole('button', { name: 'Lock file' }).click()
      await expect.poll(async () => (await ipcCalls(page, 'lock_create')).length).toBe(1)
      expect(await last(page, 'lock_create')).toMatchObject({ path: 'audio/a.wav', owner: 'ana', remote: 'backup' })
    })
  }

  test('unlock uses the chosen remote', async ({ page }) => {
    await mockIpc(page, { commands: healthy({ lock_list: fixture('locks'), remote_list: TWO, lock_unlock: {} }), storage: { ...REPO, 'mg.lock.owner': JSON.stringify('Mira Okafor') } })
    await page.goto('/#/locks')
    await page.getByRole('combobox', { name: 'Remote' }).click()
    await page.getByRole('option', { name: 'backup' }).click()
    await page.getByRole('button', { name: 'Mine' }).click()
    await page.locator('[data-lock="art/hero.psd"]').getByRole('button', { name: 'Unlock' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'lock_unlock')).length).toBe(1)
    expect((await last(page, 'lock_unlock')).remote).toBe('backup')
  })
})

test.describe('Changes: staging, saving and unstaging options', () => {
  async function changes(page: Page, theme: (typeof THEMES)[number] = 'light', extra: Record<string, unknown> = {}) {
    await mockIpc(page, { commands: healthy({ repo_add: { added: 2, modified: 0, deleted: 1, bytes: 0 }, repo_commit: { oid: 'abc', branch: 'main', summary: 'x' }, repo_unstage: {}, ...extra }), storage: { ...REPO, 'mg.theme': JSON.stringify(theme) } })
    await page.goto('/#/changes')
    await expect(page.getByRole('heading', { name: 'Changes', level: 2 })).toBeVisible()
  }

  for (const theme of THEMES) {
    test(`staging options reach add (${theme})`, async ({ page }) => {
      await changes(page, theme)
      await page.getByTestId('stage-more').getByText('Staging options').click()
      for (const n of ['Include removed files', 'Only files already tracked', 'Leave removed files out', 'Include files the ignore list skips']) await check(page, n).click()
      await expectNoAxeViolations(page)
      await page.getByRole('button', { name: 'Include all', exact: true }).click()
      await expect.poll(async () => (await ipcCalls(page, 'repo_add')).length).toBe(1)
      expect((await last(page, 'repo_add')).opts).toEqual({ all: true, dryRun: false, force: true, update: true, ignoreRemoval: true })
    })
  }

  test('a staging preview says what would happen and changes nothing', async ({ page }) => {
    await changes(page)
    await page.getByTestId('stage-more').getByText('Staging options').click()
    await check(page, 'Preview only').click()
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    await expect(page.getByTestId('preview-note')).toContainText('3 files would be staged')
    expect(((await last(page, 'repo_add')).opts as { dryRun: boolean }).dryRun).toBe(true)
  })

  for (const theme of THEMES) {
    test(`save options reach commit (${theme})`, async ({ page }) => {
      await changes(page, theme)
      await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Grade pass')
      await page.getByTestId('commit-more').getByText('More options').click()
      await check(page, 'Stage every changed tracked file first').click()
      await page.getByLabel('Also stage these files').fill('art/a.psd\nart/b.psd')
      await page.getByLabel('Author').fill('Ana Ruiz <ana@studio.example>')
      await page.getByLabel('Date').fill('2026-01-15T10:30:00Z')
      for (const n of ['Allow a version with no changes', 'Add a Signed-off-by line']) await check(page, n).click()
      await expectNoAxeViolations(page)
      await page.getByRole('button', { name: 'Save version' }).click()
      await expect.poll(async () => (await ipcCalls(page, 'repo_commit')).length).toBe(1)
      expect(await last(page, 'repo_commit')).toMatchObject({
        message: 'Grade pass',
        opts: { all: true, include: ['art/a.psd', 'art/b.psd'], author: 'Ana Ruiz <ana@studio.example>', date: '2026-01-15T10:30:00Z', allowEmpty: true, signoff: true },
      })
    })
  }

  test('a message file replaces the message box; preview keeps your text', async ({ page }) => {
    await changes(page, 'light', { repo_commit: { would_commit: true, staged: 3, deleted: 0, staged_paths: [], summary: 'x' } })
    await page.getByTestId('commit-more').getByText('More options').click()
    await expect(page.getByRole('button', { name: 'Save version' })).toBeDisabled()
    await page.getByLabel('Take the description from a file').fill('C:\\msgs\\release.txt')
    await check(page, 'Preview only').last().click()
    await page.getByRole('button', { name: 'Save version' }).click()
    await expect(page.getByTestId('preview-note')).toContainText('would save 3 files')
    expect(((await last(page, 'repo_commit')).opts as Record<string, unknown>).messageFile).toBe('C:\\msgs\\release.txt')
  })

  test('unstage all and unstage selected', async ({ page }) => {
    await changes(page)
    await page.getByRole('button', { name: 'Exclude all' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_unstage')).length).toBe(1)
    expect(await last(page, 'repo_unstage')).toEqual({ repo: '/repos/hero-film', paths: ['.'] })
    await page.getByRole('checkbox', { name: /Include .*hero.psd/ }).click()
    await expect.poll(async () => (await ipcCalls(page, 'repo_unstage')).length).toBe(2)
    expect(((await last(page, 'repo_unstage')).paths as string[]).length).toBe(1)
  })
})

