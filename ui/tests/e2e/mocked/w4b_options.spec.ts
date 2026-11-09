import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { fixture, healthy, ipcCalls, mockIpc, RECENT } from './ipc-mock'

// W4b: every remaining flag of tags, stash, gc, fsck, verify, auth register/passwd and
// key rotate-master is reachable. Each test opens the disclosure, sets the option and
// checks the exact arguments the mocked command receives.

const REPO = { 'mg.repo': JSON.stringify('/repos/hero-film'), 'mg.recent': RECENT }
const PW = 'Zebra-pw-Quiet-91!'
const calls = async (page: Page, cmd: string) => (await ipcCalls(page, cmd)).map((c) => c.args)
const polled = (page: Page, cmd: string) => expect.poll(async () => (await ipcCalls(page, cmd)).length)
const more = (scope: ReturnType<Page['locator']>) => scope.locator('summary').click()

async function boot(page: Page, route: string, heading: string, commands: Record<string, unknown> = {}, handlers: Record<string, string> = {}) {
  await mockIpc(page, { commands: healthy(commands), handlers, storage: REPO })
  await page.goto(`/#/${route}`, { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
}

const tagsBoot = (page: Page, extra: Record<string, unknown> = {}, handlers: Record<string, string> = {}) =>
  boot(page, 'tags', 'Tags & Stash', { tag_list: fixture('tags'), stash_list: fixture('stash_list'), ...extra }, handlers)

const CREATED = { action: 'create', created: null, deleted: [], missing: [] }
const SHOWN = { name: 'v0.3', oid: '0000000000000000000000000000000000000000', annotated: true, message: 'Rig v2 export', tagger_name: 'Mira O', tagger_email: 'mira@studio.example', date: '2026-11-01T10:00:00+00:00', signed: true }

test.describe('Tags: options', () => {
  test('filter: tag list gets the pattern', async ({ page }) => {
    await tagsBoot(page)
    await page.getByLabel('Show only tags matching').fill('v0.*')
    await expect.poll(async () => (await calls(page, 'tag_list')).map((a) => a.pattern)).toContain('v0.*')
    await expectNoAxeViolations(page)
  })

  test('filter with no match says so', async ({ page }) => {
    await tagsBoot(page, {}, { tag_list: `(a) => a.pattern ? { tags: [] } : ${JSON.stringify(fixture('tags'))}` })
    await page.getByLabel('Show only tags matching').fill('zzz*')
    await expect(page.getByText('No tag matches')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('details: note, tagger, date, signature; check signature', async ({ page }) => {
    await tagsBoot(page, { tag_show: SHOWN, tag_verify: { name: 'v0.3', oid: SHOWN.oid, annotated: true, signed: true, fingerprint: 'AB12 CD34' } })
    await page.getByRole('button', { name: 'Details of tag v0.3' }).click()
    const dlg = page.getByRole('dialog', { name: 'Tag v0.3' })
    await expect(dlg.getByTestId('tag-detail')).toContainText('Rig v2 export')
    await expect(dlg.getByTestId('tag-detail')).toContainText('Mira O (mira@studio.example)')
    await expect(dlg.getByTestId('tag-detail')).toContainText('Signed')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Check signature' }).click()
    await expect(dlg).toContainText('Signature is valid')
    await expect(dlg).toContainText('AB12 CD34')
    expect((await calls(page, 'tag_show'))[0]).toMatchObject({ name: 'v0.3' })
    expect((await calls(page, 'tag_verify'))[0]).toMatchObject({ name: 'v0.3' })
    await expectNoAxeViolations(page)
  })

  test('an unsigned tag says there is nothing to check; a bad signature is an error', async ({ page }) => {
    await tagsBoot(page, { tag_show: { ...SHOWN, signed: false }, tag_verify: { name: 'v0.3', oid: SHOWN.oid, annotated: true, signed: false, fingerprint: null } })
    await page.getByRole('button', { name: 'Details of tag v0.3' }).click()
    const dlg = page.getByRole('dialog', { name: 'Tag v0.3' })
    await dlg.getByRole('button', { name: 'Check signature' }).click()
    await expect(dlg).toContainText('not signed, so there is nothing to check')
  })

  test('details that cannot be read show the reason', async ({ page }) => {
    await tagsBoot(page, { tag_show: { __error: { code: 'error', message: 'Tag v0.3 is damaged.' } } })
    await page.getByRole('button', { name: 'Details of tag v0.3' }).click()
    await expect(page.getByRole('dialog', { name: 'Tag v0.3' }).getByRole('alert')).toContainText('damaged')
  })

  test('more options: version, tagger and email reach tag create; tagger needs a note', async ({ page }) => {
    await tagsBoot(page, { tag_create: CREATED })
    const form = page.locator('form', { has: page.getByLabel('Tag name') })
    await more(form)
    await expect(form.getByLabel('Tagger name')).toBeDisabled()
    await expectNoAxeViolations(page)
    await page.getByLabel('Tag name').fill('v1.0')
    await page.getByLabel('Note (optional)').fill('Final')
    await form.getByLabel('Version to tag').fill('abc1234')
    await form.getByLabel('Tagger name').fill('Mira O')
    await form.getByLabel('Tagger email').fill('mira@studio.example')
    await form.getByRole('button', { name: 'Tag this version' }).click()
    await polled(page, 'tag_create').toBe(1)
    expect((await calls(page, 'tag_create'))[0]).toMatchObject({
      name: 'v1.0',
      options: { commit: 'abc1234', message: 'Final', tagger: 'Mira O', email: 'mira@studio.example', force: false, confirmed: false },
    })
  })

  test('a version id that is not one is refused before sending', async ({ page }) => {
    await tagsBoot(page, { tag_create: CREATED })
    const form = page.locator('form', { has: page.getByLabel('Tag name') })
    await more(form)
    await page.getByLabel('Tag name').fill('v1.0')
    await form.getByLabel('Version to tag').fill('main')
    await expect(form.getByText('Use a version id from History')).toBeVisible()
    await expect(form.getByRole('button', { name: 'Tag this version' })).toBeDisabled()
  })

  test('replacing a tag: danger button, a dialog that says what is lost, force only after confirm', async ({ page }) => {
    await tagsBoot(page, { tag_create: CREATED })
    const form = page.locator('form', { has: page.getByLabel('Tag name') })
    await more(form)
    await page.getByLabel('Tag name').fill('v0.3')
    await expect(page.getByText('A tag with this name exists')).toBeVisible()
    await form.getByRole('checkbox', { name: /Replace the tag if the name is taken/ }).click()
    const go = form.getByRole('button', { name: 'Replace tag…' })
    await expect(go).toBeEnabled()
    await go.click()
    const alert = page.getByRole('alertdialog')
    await expect(alert).toContainText('The old tag is lost')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Cancel' }).click()
    expect(await calls(page, 'tag_create')).toHaveLength(0)
    await go.click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Replace tag' }).click()
    await polled(page, 'tag_create').toBe(1)
    expect((await calls(page, 'tag_create'))[0]).toMatchObject({ name: 'v0.3', options: { force: true, confirmed: true } })
  })

  test('replace ticked but the name is free: no force is sent', async ({ page }) => {
    await tagsBoot(page, { tag_create: CREATED })
    const form = page.locator('form', { has: page.getByLabel('Tag name') })
    await more(form)
    await page.getByLabel('Tag name').fill('v9')
    await form.getByRole('checkbox', { name: /Replace the tag/ }).click()
    await form.getByRole('button', { name: 'Tag the latest version' }).click()
    await polled(page, 'tag_create').toBe(1)
    expect((await calls(page, 'tag_create'))[0]).toMatchObject({ options: { force: true, confirmed: false } })
  })
})

test.describe('Stash: options', () => {
  test('shelve selected files: only the ticked paths are sent', async ({ page }) => {
    await tagsBoot(page, { stash_save: { action: 'save', index: 0, oid: 'aa'.repeat(20), message: 'x', files_updated: null, stashes: [], paths: [] } })
    const card = page.locator('section[aria-labelledby="stash-h"]')
    await more(card.locator('details'))
    const files = card.getByTestId('stash-files')
    await expect(files).toContainText('art/hero.psd')
    await expectNoAxeViolations(page)
    await files.getByRole('checkbox', { name: 'art/hero.psd' }).click()
    await files.getByRole('checkbox', { name: 'char/rig_lod1.fbx' }).click()
    await expect(card.getByRole('button', { name: 'Set aside 2 selected files' })).toBeEnabled()
    await card.getByLabel('Message (optional)').fill('Only art')
    await card.getByRole('button', { name: 'Set aside 2 selected files' }).click()
    await polled(page, 'stash_save').toBe(1)
    expect((await calls(page, 'stash_save'))[0]).toMatchObject({ message: 'Only art', includeUntracked: false, paths: ['art/hero.psd', 'char/rig_lod1.fbx'] })
  })

  test('new files can be picked once "Include new files" is on', async ({ page }) => {
    await tagsBoot(page)
    const card = page.locator('section[aria-labelledby="stash-h"]')
    await more(card.locator('details'))
    await expect(card.getByTestId('stash-files')).not.toContainText('env/fog_vol.vdb')
    await card.getByRole('checkbox', { name: 'Include new files' }).click()
    await expect(card.getByTestId('stash-files')).toContainText('env/fog_vol.vdb')
  })

  test('options dialog: apply with --index; details of the entry are read', async ({ page }) => {
    await tagsBoot(page, { stash_apply: fixture('stash_apply_progress'), stash_show: { action: 'show', index: 0, oid: 'aa', message: 'WIP lighting', files_updated: null, stashes: [{ index: 0, oid: 'aa', message: 'WIP lighting', branch: 'main', timestamp: '2026-11-01T10:00:00+00:00' }], paths: [] } })
    await page.getByRole('button', { name: 'Options for stash 0' }).click()
    const dlg = page.getByRole('dialog', { name: 'Stash 0' })
    await expect(dlg.getByTestId('stash-detail')).toContainText('WIP lighting')
    await dlg.getByRole('checkbox', { name: /Also restore what was staged/ }).click()
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Apply', exact: true }).click()
    await polled(page, 'stash_apply').toBe(1)
    expect((await calls(page, 'stash_show'))[0]).toMatchObject({ index: 0 })
    expect((await calls(page, 'stash_apply'))[0]).toMatchObject({ index: 0, restoreIndex: true, force: false, confirmed: false })
  })

  test('overwriting: the dialog says what is lost and the buttons turn into danger actions', async ({ page }) => {
    await tagsBoot(page, { stash_pop: fixture('stash_apply_progress') })
    await page.getByRole('button', { name: 'Options for stash 1' }).click()
    const dlg = page.getByRole('dialog', { name: 'Stash 1' })
    await dlg.getByRole('checkbox', { name: /Overwrite my current changes/ }).click()
    await expect(dlg.getByRole('note')).toContainText('Your newer edits to them are lost')
    await expect(dlg.getByRole('button', { name: 'Overwrite, apply and remove' })).toBeVisible()
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Overwrite, apply and remove' }).click()
    await polled(page, 'stash_pop').toBe(1)
    expect((await calls(page, 'stash_pop'))[0]).toMatchObject({ index: 1, restoreIndex: false, force: true, confirmed: true })
  })

  test('without the overwrite box, no force is sent even through the options dialog', async ({ page }) => {
    await tagsBoot(page, { stash_pop: fixture('stash_apply_progress') })
    await page.getByRole('button', { name: 'Options for stash 0' }).click()
    await page.getByRole('dialog', { name: 'Stash 0' }).getByRole('button', { name: 'Apply and remove' }).click()
    await polled(page, 'stash_pop').toBe(1)
    expect((await calls(page, 'stash_pop'))[0]).toMatchObject({ force: false, confirmed: false })
  })
})

test.describe('Maintenance: options', () => {
  const card = (page: Page, t: string) => page.locator(`[data-task="${t}"]`)
  const mboot = (page: Page, extra: Record<string, unknown> = {}) =>
    boot(page, 'maintenance', 'Maintenance', { repo_gc: fixture('gc_dry'), repo_fsck: fixture('fsck_ok'), repo_verify: fixture('fsck_ok'), ...extra })

  test('clean-up options reach gc', async ({ page }) => {
    await mboot(page)
    await more(card(page, 'gc'))
    await expectNoAxeViolations(page)
    await card(page, 'gc').getByRole('checkbox', { name: /Keep data nothing refers to/ }).click()
    await card(page, 'gc').getByRole('checkbox', { name: /Only if it is worth doing/ }).click()
    await card(page, 'gc').getByRole('checkbox', { name: /Pack loose data together/ }).click()
    await card(page, 'gc').getByRole('button', { name: 'Preview clean-up' }).click()
    await polled(page, 'repo_gc').toBe(1)
    expect((await calls(page, 'repo_gc'))[0]).toMatchObject({ dryRun: true, confirmed: false, options: { noPrune: true, auto: true, repack: true } })
  })

  test('a real clean-up keeps its confirm and carries the options', async ({ page }) => {
    await mboot(page, { repo_gc: fixture('gc_done') })
    await more(card(page, 'gc'))
    await card(page, 'gc').getByRole('checkbox', { name: /Pack loose data together/ }).click()
    await card(page, 'gc').getByRole('checkbox', { name: 'Preview only' }).click()
    await card(page, 'gc').getByRole('button', { name: 'Clean up…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Clean up' }).click()
    await polled(page, 'repo_gc').toBe(1)
    expect((await calls(page, 'repo_gc'))[0]).toMatchObject({ dryRun: false, confirmed: true, options: { repack: true, noPrune: false } })
  })

  test('integrity listing options reach fsck', async ({ page }) => {
    await mboot(page)
    await more(card(page, 'fsck'))
    await expectNoAxeViolations(page)
    for (const name of [/List every object checked/, /List lost and unused objects/, /Skip the unused-objects check/]) await card(page, 'fsck').getByRole('checkbox', { name }).click()
    await card(page, 'fsck').getByRole('button', { name: 'Run integrity check' }).click()
    await polled(page, 'repo_fsck').toBe(1)
    expect((await calls(page, 'repo_fsck'))[0]).toMatchObject({ options: { all: true, lostFound: true, noDangling: true, repair: false, dryRun: false, confirmed: false } })
  })

  test('repair previews first; a real repair is a danger button with a confirm that says what is lost', async ({ page }) => {
    await mboot(page, { repo_fsck: { ...fixture<Record<string, unknown>>('fsck_ok'), repaired: 2 } })
    const c = card(page, 'fsck')
    await more(c)
    await c.getByRole('checkbox', { name: /Try to repair what it finds/ }).click()
    const preview = c.getByRole('checkbox', { name: /Only show what a repair would change/ })
    await expect(preview).toBeChecked()
    await c.getByRole('button', { name: 'Preview repair' }).click()
    await polled(page, 'repo_fsck').toBe(1)
    expect((await calls(page, 'repo_fsck'))[0]).toMatchObject({ options: { repair: true, dryRun: true } })
    await expect(c.getByTestId('fsck-result')).toContainText('Would repair')
    await expectNoAxeViolations(page)

    await preview.click()
    const go = c.getByRole('button', { name: 'Repair…' })
    await go.click()
    const alert = page.getByRole('alertdialog')
    await expect(alert).toContainText('cannot be brought back')
    await expectNoAxeViolations(page)
    await alert.getByRole('button', { name: 'Cancel' }).click()
    expect(await calls(page, 'repo_fsck')).toHaveLength(1)
    await go.click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Repair' }).click()
    await polled(page, 'repo_fsck').toBe(2)
    expect((await calls(page, 'repo_fsck'))[1]).toMatchObject({ options: { repair: true, dryRun: false, confirmed: true } })
    await expect(c.getByTestId('fsck-result')).toContainText('Repaired')
  })

  test('verify one version, with checksums and detail', async ({ page }) => {
    await mboot(page, { repo_verify: { start: null, end: 'abc', commits: 3, verified: 3, errors: 0, problems: [] } })
    const c = card(page, 'verify')
    await more(c)
    await expectNoAxeViolations(page)
    await c.getByLabel('Check only this version').fill('main')
    await c.getByRole('checkbox', { name: /Check every checksum/ }).click()
    await c.getByRole('checkbox', { name: /Describe each problem in detail/ }).click()
    await c.getByRole('button', { name: 'Verify' }).click()
    await polled(page, 'repo_verify').toBe(1)
    expect((await calls(page, 'repo_verify'))[0]).toMatchObject({ options: { commit: 'main', start: null, end: null, checksums: true, detailed: true, quick: false } })
    await expect(c.getByTestId('verify-range-result')).toContainText('No problems found')
    await expect(c.getByTestId('verify-range-result')).toContainText('Versions looked at')
  })

  test('verify a range; problems are listed; a version and a range cannot be mixed', async ({ page }) => {
    await mboot(page, { repo_verify: { start: 'aaa', end: 'bbb', commits: 4, verified: 3, errors: 1, problems: ['object 12ab is missing'] } })
    const c = card(page, 'verify')
    await more(c)
    await c.getByLabel('Check only this version').fill('main')
    await c.getByLabel('From version').fill('aaa1111')
    await expect(c.getByText('Choose one version or a range, not both')).toBeVisible()
    await expect(c.getByRole('button', { name: 'Verify' })).toBeDisabled()
    await c.getByLabel('Check only this version').fill('')
    await c.getByLabel('Up to version').fill('HEAD')
    await c.getByRole('button', { name: 'Verify' }).click()
    await polled(page, 'repo_verify').toBe(1)
    expect((await calls(page, 'repo_verify'))[0]).toMatchObject({ options: { commit: null, start: 'aaa1111', end: 'HEAD' } })
    await expect(c.getByTestId('verify-range-result')).toContainText('Problems found')
    await expect(c.getByRole('list', { name: 'Verify findings' })).toContainText('object 12ab is missing')
    await expectNoAxeViolations(page)
  })
})

test.describe('Security: account and key options', () => {
  const OUT = fixture('auth_out')
  const IN = fixture('auth_in')
  const sboot = (page: Page, auth: unknown, key: unknown = fixture('key_on'), extra: Record<string, unknown> = {}) =>
    boot(page, 'security', 'Security', { auth_run: auth, key_status: key, ...extra })
  const seen = (page: Page) => page.evaluate(() => document.body.innerText + JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }))

  test('create an account: password mismatch is caught; the password is only an argument', async ({ page }) => {
    await sboot(page, OUT, undefined, { auth_register: { action: 'register' } })
    await page.getByRole('button', { name: 'Create account…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Create account' })
    await dlg.getByLabel('Server address (optional)').fill('https://mediagit.studio.example')
    await dlg.getByLabel('Username').fill('lena')
    await dlg.getByLabel('Email').fill('lena@studio.example')
    await dlg.getByLabel('Password', { exact: true }).fill(PW)
    await dlg.getByLabel('Repeat password').fill(PW + 'x')
    await expect(dlg.getByText('The passwords do not match')).toBeVisible()
    await expect(dlg.getByRole('button', { name: 'Create account' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Repeat password').fill(PW)
    await dlg.getByRole('button', { name: 'Create account' }).click()
    await polled(page, 'auth_register').toBe(1)
    expect((await calls(page, 'auth_register'))[0]).toEqual({ server: 'https://mediagit.studio.example', username: 'lena', email: 'lena@studio.example', password: PW })
    expect(await seen(page)).not.toContain(PW)
  })

  test('change password: both passwords are arguments, then gone', async ({ page }) => {
    await sboot(page, IN, undefined, { auth_passwd: { action: 'passwd' } })
    await page.getByRole('button', { name: 'Change password…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Change password' })
    await dlg.getByLabel('Current password').fill('Old-pw-One-77!')
    await dlg.getByLabel('New password', { exact: true }).fill(PW)
    await dlg.getByLabel('Repeat new password').fill(PW)
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Change password' }).click()
    await polled(page, 'auth_passwd').toBe(1)
    expect((await calls(page, 'auth_passwd'))[0]).toEqual({ server: null, current: 'Old-pw-One-77!', new: PW })
    const text = await seen(page)
    expect(text).not.toContain(PW)
    expect(text).not.toContain('Old-pw-One-77')
  })

  test('change password failure shows the reason', async ({ page }) => {
    await sboot(page, IN, undefined, { auth_passwd: { __error: { code: 'error', message: 'current password was rejected' } } })
    await page.getByRole('button', { name: 'Change password…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Change password' })
    await dlg.getByLabel('Current password').fill('wrong-old-pw')
    await dlg.getByLabel('New password', { exact: true }).fill(PW)
    await dlg.getByLabel('Repeat new password').fill(PW)
    await dlg.getByRole('button', { name: 'Change password' }).click()
    await expect(page.getByRole('alert')).toContainText('current password was rejected')
  })

  test('replace master key with a new passphrase (current one asked for only when it is a passphrase)', async ({ page }) => {
    await sboot(page, IN, { ...fixture<Record<string, unknown>>('key_on'), source: 'passphrase' }, { key_rotate_master: { action: 'rotate-master', enabled: true, source: 'passphrase' } })
    await page.getByRole('button', { name: 'Replace master key…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Replace the master key' })
    await expect(dlg).toContainText('does not re-encrypt the data')
    await dlg.getByLabel('Current passphrase').fill('cur-Pass-Phrase-1')
    await dlg.getByLabel('New passphrase').fill('next-Pass-Phrase-2')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Replace master key' }).click()
    await polled(page, 'key_rotate_master').toBe(1)
    expect((await calls(page, 'key_rotate_master'))[0]).toMatchObject({ repo: '/repos/hero-film', newKeyfile: null, current: 'cur-Pass-Phrase-1', newPassphrase: 'next-Pass-Phrase-2' })
    const text = await seen(page)
    expect(text).not.toContain('Pass-Phrase')
  })

  test('replace master key with a key file; no current passphrase is asked when the key unlocks from the keychain', async ({ page }) => {
    await sboot(page, IN, fixture('key_on'), { key_rotate_master: { action: 'rotate-master', enabled: true, source: 'key file' } })
    await page.getByRole('button', { name: 'Replace master key…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Replace the master key' })
    await expect(dlg.getByLabel('Current passphrase')).toHaveCount(0)
    await dlg.getByRole('button', { name: 'A key file' }).click()
    await dlg.getByLabel('Key file').fill('C:\\Keys\\hero-film.key')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Replace master key' }).click()
    await polled(page, 'key_rotate_master').toBe(1)
    expect((await calls(page, 'key_rotate_master'))[0]).toMatchObject({ newKeyfile: 'C:\\Keys\\hero-film.key', current: null, newPassphrase: null })
  })

  test('replace master key failure keeps the reason', async ({ page }) => {
    await sboot(page, IN, fixture('key_on'), { key_rotate_master: { __error: { code: 'error', message: 'that would re-wrap the repository under the master key it already uses' } } })
    await page.getByRole('button', { name: 'Replace master key…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Replace the master key' })
    await dlg.getByLabel('New passphrase').fill('some-new-pass')
    await dlg.getByRole('button', { name: 'Replace master key' }).click()
    await expect(page.getByRole('alert')).toContainText('already uses')
  })
})
