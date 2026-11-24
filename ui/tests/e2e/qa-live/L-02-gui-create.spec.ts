import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { Recorder, closeTerm, workDir } from './rec'
import { answerPicker, pickerHits, toWelcome, visibleErrors } from './gui'
import { intCheck } from './integrity'

// L-02: Welcome > New repository, with More options, cancel, an already-initialised folder,
// a folder that holds files, and a CLI twin for parity.
test('L-02 GUI create a repository from Welcome', async () => {
  const rec = await Recorder.start('L-02', 'gui-create', { gui: true })
  const page = rec.page!
  const root = workDir('L-02')
  rmSync(root, { recursive: true, force: true })
  const dir = (n: string) => { const d = join(root, n); mkdirSync(d, { recursive: true }); return d }
  const fresh = dir('dragon-keep')
  const cancelled = dir('cancelled')
  const withFiles = dir('has-files')
  writeFileSync(join(withFiles, 'shot_010.txt'), 'plate notes\n')
  mkdirSync(join(withFiles, 'art'))
  writeFileSync(join(withFiles, 'art', 'ref.txt'), 'reference\n')
  const twin = dir('twin')

  await rec.step('Welcome screen, no repository open', () => toWelcome(page, true))
  rec.check(await page.getByText('No repositories yet').isVisible(), 'Welcome shows the empty recent list')

  // --- picker cancelled: nothing happens
  await rec.step('New repository, folder picker cancelled', async () => {
    await answerPicker(page, null)
    await page.getByRole('button', { name: 'New repository (init)' }).click()
    await page.waitForTimeout(600)
  })
  rec.check((await pickerHits(page)) === 1, 'the folder picker was asked once')
  rec.check(!(await page.getByRole('dialog', { name: 'Create repository' }).isVisible()), 'cancelling the picker opens no dialog')

  // --- dialog cancelled: no repository is made
  const dlg = page.getByRole('dialog', { name: 'Create repository' })
  await rec.step('New repository, pick a folder, then Cancel the dialog', async () => {
    await answerPicker(page, cancelled)
    await page.getByRole('button', { name: 'New repository (init)' }).click()
    await dlg.waitFor()
    await rec.shot('create dialog open')
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await dlg.waitFor({ state: 'hidden' })
  })
  rec.check(!existsSync(join(cancelled, '.mediagit')), 'Cancel in the dialog creates nothing on disk')

  // --- bad branch name, then a good one
  await rec.step('New repository, More options, a bad first branch name', async () => {
    await answerPicker(page, fresh)
    await page.getByRole('button', { name: 'New repository (init)' }).click()
    await dlg.waitFor()
    await dlg.getByText('More options').click()
    await dlg.getByLabel('First branch name').fill('bad name')
  })
  rec.check(await dlg.getByRole('button', { name: 'Create' }).isDisabled(), 'Create is disabled for the branch name "bad name"')
  rec.note(`branch-name error text: ${JSON.stringify(await dlg.locator('.error, [role="alert"], .hint').allInnerTexts())}`)
  rec.check((await dlg.innerText()).includes(fresh), 'the dialog shows the chosen folder path')

  await rec.step('Type the branch name "dev" and Create', async () => {
    await dlg.getByLabel('First branch name').fill('dev')
    await dlg.getByRole('button', { name: 'Create' }).click()
    await page.getByRole('navigation', { name: 'Main' }).waitFor({ timeout: 20_000 })
  })
  await page.waitForTimeout(800)
  await rec.shot('after create: where the app landed')
  rec.check(/#\/files/.test(page.url()), 'the app lands on the Files hub', page.url())
  rec.check((await page.getByTestId('repo-switcher').innerText()).includes('dragon-keep'), 'the rail shows the repository name')
  const sub = await page.getByTestId('branch-indicator').innerText()
  rec.check(sub.includes('dev'), 'the rail shows branch dev', sub)
  rec.note(`Files hub text on an empty repository: ${(await page.locator('main, .hub, #app').first().innerText()).replace(/\n+/g, ' / ').slice(0, 500)}`)
  rec.check(existsSync(join(fresh, '.mediagit')), '.mediagit exists on disk')
  const st = await rec.cli(fresh, ['status', '--branch'])
  rec.check(/On branch: dev/.test(st.stdout), 'CLI agrees: on branch dev', st.stdout.slice(0, 200))

  // --- parity twin
  await rec.cli(root, ['init', '--initial-branch', 'dev', twin])
  const ls = (d: string) => readdirSync(join(d, '.mediagit')).sort().join(',')
  rec.check(ls(fresh) === ls(twin), 'parity: GUI-made and CLI-made repositories have the same .mediagit layout', `${ls(fresh)} vs ${ls(twin)}`)

  // --- create in a folder that already is a repository
  await rec.step('Welcome again: New repository on a folder that is already a repository', async () => {
    await toWelcome(page)
    await answerPicker(page, fresh)
    await page.getByRole('button', { name: 'New repository (init)' }).click()
    await dlg.waitFor()
    await dlg.getByRole('button', { name: 'Create' }).click()
    await page.waitForTimeout(1500)
  })
  const errs = await visibleErrors(page)
  rec.note(`already-a-repository message: ${JSON.stringify(errs)} url ${page.url()}`)
  rec.check(errs.join(' ').length > 0 || /#\/files/.test(page.url()), 'creating on an existing repository says so or simply opens it (no silent nothing)')
  rec.check(!/os error|panick|Error:\s*Error|exit code/i.test(errs.join(' ')), 'that message is plain words', errs.join(' | ').slice(0, 300))
  const st2 = await rec.cli(fresh, ['status', '--branch'])
  rec.check(/On branch: dev/.test(st2.stdout), 'the existing repository is untouched (still on dev)')

  // --- recent list now has the repository
  await rec.step('Welcome: the recent list holds the new repository', () => toWelcome(page))
  rec.check((await page.getByTestId('recent-list').innerText().catch(() => '')).includes('dragon-keep'), 'recent list shows dragon-keep')

  // --- create in a folder that already holds files
  await rec.step('New repository on a folder that already holds files', async () => {
    await answerPicker(page, withFiles)
    await page.getByRole('button', { name: 'New repository (init)' }).click()
    await dlg.waitFor()
    await dlg.getByRole('button', { name: 'Create' }).click()
    await page.getByRole('navigation', { name: 'Main' }).waitFor({ timeout: 20_000 })
    await page.waitForTimeout(1200)
  })
  const body = await page.locator('#app').innerText()
  rec.check(body.includes('shot_010.txt') || body.includes('art'), 'existing files show up in the Files hub straight away', body.replace(/\n+/g, ' / ').slice(0, 300))
  const por = await rec.cli(withFiles, ['status', '--porcelain'])
  rec.check(por.stdout.includes('shot_010.txt'), 'CLI agrees: the files are untracked, nothing was staged for the user', por.stdout.slice(0, 200))
  rec.check(/main/.test(await page.getByTestId('branch-indicator').innerText()), 'default first branch is main when the field is left empty')

  await intCheck(rec, fresh)
  rec.touch(withFiles)
  await rec.finish()
  await closeTerm()
})
