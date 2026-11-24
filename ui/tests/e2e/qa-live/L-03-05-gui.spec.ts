import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { launchApp, quitApp } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { answerPicker, openRepo, toWelcome, visibleErrors } from './gui'
import { copyCorpus, intCheck } from './integrity'

const MIX: Record<string, string> = {
  '12690118_5053480.ai': 'design/poster.ai',
  '1965_ac_shelby_427_cobra_sc.glb': 'models/cobra.glb',
  'pdf/1710.02185v1.pdf': 'docs/paper.pdf',
  'freepik__talk__71826.jpeg': 'refs/talk.jpeg',
}
const PLAIN = /os error|panick|Error:\s*Error|exit code|stack backtrace|undefined|\[object/i

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' })
const onWelcome = (page: Page) => page.getByRole('heading', { name: 'Welcome' }).isVisible()
const switcher = (page: Page) => page.getByTestId('repo-switcher')
const appText = async (page: Page) => (await page.locator('#app').innerText()).replace(/\n+/g, ' / ')

async function seed(rec: Recorder, repo: string, files = false) {
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  await rec.cli(repo, ['init'], { quietFrame: true })
  if (files) {
    await copyCorpus(repo, MIX)
    await rec.cli(repo, ['add', '--all'], { quietFrame: true })
    await rec.cli(repo, ['commit', '-m', 'first plates'], { quietFrame: true })
  }
}

async function pickOpen(page: Page, dir: string | null, wait = 1500) {
  await answerPicker(page, dir)
  await page.getByRole('button', { name: 'Open folder…' }).click()
  await page.waitForTimeout(wait)
}

test('L-03 GUI open an existing repository, and folders that are not one', async () => {
  const rec = await Recorder.start('L-03', 'gui-open', { gui: true })
  const page = rec.page!
  const root = workDir('L-03')
  const repo = join(root, 'shots')
  await seed(rec, repo, true)
  const plain = join(root, 'plain-folder')
  rmSync(plain, { recursive: true, force: true })
  mkdirSync(plain, { recursive: true })
  writeFileSync(join(plain, 'notes.txt'), 'not a repository\n')
  const broken = join(root, 'broken')
  rmSync(broken, { recursive: true, force: true })
  mkdirSync(join(broken, '.mediagit'), { recursive: true })

  await rec.step('Welcome, empty recents', () => toWelcome(page, true))

  await rec.step('Open folder, picker cancelled', () => pickOpen(page, null, 700))
  rec.check(await onWelcome(page), 'cancelling the picker stays on Welcome')
  rec.check((await visibleErrors(page)).length === 0, 'cancelling the picker shows no error', JSON.stringify(await visibleErrors(page)))

  await rec.step('Open folder: a real repository', async () => {
    await pickOpen(page, repo, 0)
    await nav(page).waitFor({ timeout: 20_000 })
    await page.waitForTimeout(1000)
  })
  rec.check(/#\/files/.test(page.url()), 'lands on the Files hub', page.url())
  rec.check((await switcher(page).innerText()).includes('shots'), 'rail shows the repository name')
  rec.check((await page.getByTestId('branch-indicator').innerText()).includes('main'), 'rail shows branch main')
  const body = await appText(page)
  rec.note(`Files hub after open: ${body.slice(0, 400)}`)
  rec.check(/design|models|docs|refs/.test(body), 'the tracked folders are listed')

  await rec.step('Welcome again, Open folder: a folder that is not a repository', async () => {
    await toWelcome(page)
    await pickOpen(page, plain)
  })
  let errs = await visibleErrors(page)
  rec.note(`not-a-repository message: ${JSON.stringify(errs)}`)
  rec.check(await onWelcome(page), 'a non-repository folder stays on Welcome')
  rec.check(errs.join(' ').length > 0, 'a non-repository folder gives a message')
  rec.check(!PLAIN.test(errs.join(' ')), 'that message is plain words', errs.join(' | ').slice(0, 300))
  rec.check(!existsSync(join(plain, '.mediagit')), 'opening a plain folder creates nothing in it')
  rec.note(`message offers to create a repository there: ${/create|new repository|init/i.test(await appText(page))}`)

  await rec.step('Open folder: a folder with an empty .mediagit (damaged repository)', async () => {
    await toWelcome(page)
    await pickOpen(page, broken)
  })
  errs = await visibleErrors(page)
  rec.note(`damaged-repository message: ${JSON.stringify(errs)} url ${page.url()}`)
  rec.check(errs.join(' ').length > 0 || (await nav(page).isVisible()), 'a damaged repository says something or opens (no silent nothing)')
  rec.check(!PLAIN.test(errs.join(' ')), 'the damaged-repository message is plain words', errs.join(' | ').slice(0, 300))

  await rec.step('Open folder: a subfolder inside a repository', async () => {
    await toWelcome(page)
    await pickOpen(page, join(repo, 'design'), 2000)
  })
  errs = await visibleErrors(page)
  const subOpened = await nav(page).isVisible()
  rec.note(`subfolder open: opened=${subOpened} name=${subOpened ? await switcher(page).innerText() : ''} errors=${JSON.stringify(errs)}`)
  rec.check(subOpened && (await switcher(page).innerText()).includes('shots'), 'BUG-004: picking a subfolder opens the repository root (rail shows "shots", not "design")', await switcher(page).innerText().catch(() => ''))
  await page.waitForTimeout(2500)
  rec.check(!existsSync(join(repo, 'design', '.mediagit')) && !existsSync(join(repo, 'design', 'LAYOUT')), 'BUG-004: nothing is written into the picked subfolder (no nested .mediagit)')

  await rec.step('Open folder: a path that does not exist', async () => {
    await toWelcome(page)
    await pickOpen(page, join(root, 'gone-away'))
  })
  errs = await visibleErrors(page)
  rec.note(`missing-folder message: ${JSON.stringify(errs)}`)
  rec.check(errs.join(' ').length > 0 && !PLAIN.test(errs.join(' ')), 'a missing folder gives a plain-word message', errs.join(' | ').slice(0, 300))

  await rec.step('Keyboard: Ctrl+O opens the picker', async () => {
    await toWelcome(page)
    await answerPicker(page, repo)
    await page.keyboard.press('Control+o')
    await nav(page).waitFor({ timeout: 20_000 }).catch(() => {})
  })
  rec.check(await nav(page).isVisible(), 'Ctrl+O on Welcome opens the chosen repository')

  await rec.finish()
  await closeTerm()
})

test('L-04 Sidebar repository switcher: create, open, clone', async () => {
  const rec = await Recorder.start('L-04', 'gui-switcher', { gui: true })
  const page = rec.page!
  const root = workDir('L-04')
  const a = join(root, 'alpha')
  const b = join(root, 'beta')
  await seed(rec, a, true)
  await seed(rec, b)
  const made = join(root, 'made-from-rail')
  rmSync(made, { recursive: true, force: true })
  mkdirSync(made, { recursive: true })
  const item = (name: string | RegExp) => page.getByRole('menuitem', { name })
  const createDlg = page.getByRole('dialog', { name: 'Create repository' })
  const cloneDlg = page.getByRole('dialog', { name: 'Clone repository' })

  await rec.step('Open beta then alpha so both are in recents', async () => {
    await toWelcome(page, true)
    await openRepo(page, b)
    await openRepo(page, a)
  })
  await rec.step('Click the repository switcher', async () => {
    await switcher(page).click()
    await item(/Create a new repository/).waitFor()
  })
  const labels = await page.getByRole('menuitem').allInnerTexts()
  rec.note(`switcher menu: ${JSON.stringify(labels)}`)
  rec.check(['alpha', 'beta'].every((n) => labels.some((l) => l.includes(n))), 'menu lists both recent repositories')
  rec.check(labels.length >= 5, 'menu has Open, Create and Clone entries')

  await rec.step('Pick beta in the menu', async () => {
    await item('beta').click()
    await page.waitForTimeout(1500)
  })
  rec.check((await switcher(page).innerText()).includes('beta'), 'switches to beta in place')
  rec.check(await nav(page).isVisible(), 'the main rail stays (no bounce to Welcome)')
  rec.note(`route after switch: ${page.url()}`)

  // --- Create, cancelled: is the user still in their repository?
  await rec.step('Switcher > Create a new repository, then Cancel the dialog', async () => {
    await answerPicker(page, made)
    await switcher(page).click()
    await item(/Create a new repository/).click()
    await createDlg.waitFor()
    await rec.shot('create dialog from the rail: what is behind it')
    rec.note(`behind the create dialog: url ${page.url()} rail visible ${await nav(page).isVisible()}`)
    await createDlg.getByRole('button', { name: 'Cancel' }).click()
    await page.waitForTimeout(800)
  })
  const strandedCreate = !(await nav(page).isVisible())
  rec.check(!strandedCreate, 'cancelling Create from the rail returns to the open repository (beta)', `url ${page.url()} welcome=${await onWelcome(page)}`)
  rec.check(!existsSync(join(made, '.mediagit')), 'cancelled Create made nothing on disk')
  if (strandedCreate) await openRepo(page, b)

  await rec.step('Switcher > Create a new repository, Create', async () => {
    await answerPicker(page, made)
    await switcher(page).click()
    await item(/Create a new repository/).click()
    await createDlg.waitFor()
    await createDlg.getByRole('button', { name: 'Create' }).click()
    await nav(page).waitFor({ timeout: 20_000 })
    await page.waitForTimeout(1000)
  })
  rec.check((await switcher(page).innerText()).includes('made-from-rail'), 'the new repository opens straight away')
  rec.check(existsSync(join(made, '.mediagit')), '.mediagit exists on disk')

  // --- Create with the picker cancelled
  await rec.step('Switcher > Create, folder picker cancelled', async () => {
    await answerPicker(page, null)
    await switcher(page).click()
    await item(/Create a new repository/).click()
    await page.waitForTimeout(1200)
  })
  const strandedPick = !(await nav(page).isVisible())
  rec.check(!strandedPick, 'cancelling the folder picker keeps the open repository', `url ${page.url()} welcome=${await onWelcome(page)}`)
  if (strandedPick) await openRepo(page, made)

  await rec.step('Switcher > Open a repository: alpha', async () => {
    await answerPicker(page, a)
    await switcher(page).click()
    await item(/Open a repository/).click()
    await page.waitForTimeout(2000)
  })
  rec.check((await switcher(page).innerText()).includes('alpha'), 'Open from the rail switches to alpha')

  await rec.step('Switcher > Open a repository: a plain folder', async () => {
    await answerPicker(page, root)
    await switcher(page).click()
    await item(/Open a repository/).click()
    await page.waitForTimeout(2000)
  })
  const errs = await visibleErrors(page)
  rec.note(`open plain folder from the rail: errors ${JSON.stringify(errs)} url ${page.url()}`)
  rec.check(errs.join(' ').length > 0, 'opening a non-repository from the rail gives a message (not silence)')
  rec.check((await nav(page).isVisible()) && (await switcher(page).innerText()).includes('alpha'), 'alpha stays open after the failed open')

  await rec.step('Switcher > Clone a repository, then Cancel', async () => {
    if (!(await nav(page).isVisible())) await openRepo(page, a)
    await switcher(page).click()
    await item(/Clone a repository/).click()
    await cloneDlg.waitFor()
    await rec.shot('clone dialog from the rail')
    rec.check(await cloneDlg.getByRole('button', { name: 'Clone', exact: true }).isDisabled(), 'Clone is disabled while the form is empty')
    await cloneDlg.getByLabel('Repository URL').fill('ftp://nope')
    await page.waitForTimeout(300)
    rec.note(`bad URL hint: ${(await cloneDlg.innerText()).replace(/\n+/g, ' / ').slice(0, 300)}`)
    rec.check(/http\(s\)|address/i.test(await cloneDlg.innerText()), 'a bad URL is explained in the form')
    await cloneDlg.getByRole('button', { name: 'Cancel' }).click()
    await page.waitForTimeout(800)
  })
  rec.check(await nav(page).isVisible(), 'cancelling Clone from the rail returns to the open repository (alpha)', `url ${page.url()} welcome=${await onWelcome(page)}`)

  await intCheck(rec, made)
  await intCheck(rec, a)
  await rec.finish()
  await closeTerm()
})

test('L-05 Recent repositories: order, restart, a deleted repository', async () => {
  const rec = await Recorder.start('L-05', 'gui-recents', { gui: true })
  let page = rec.page!
  const root = workDir('L-05')
  const names = ['r1-concept', 'r2-layout', 'r3-anim', 'r4-comp']
  for (const n of names) await seed(rec, join(root, n))
  const list = () => page.getByTestId('recent-list')
  const order = async () => (await list().locator('li').allInnerTexts()).map((t) => names.find((n) => t.includes(n)) ?? `?${t.slice(0, 30)}`)

  // A fresh app process: WebView2 limits how often it writes localStorage to disk, and the
  // earlier cases in this run have used that budget up.
  await quitApp()
  await launchApp()
  page = await rec.attach()
  await rec.step('Fresh app start. Welcome, empty recents', () => toWelcome(page, true))
  for (const n of names) {
    await rec.step(`Open folder: ${n}`, async () => {
      await toWelcome(page)
      await pickOpen(page, join(root, n), 0)
      await nav(page).waitFor({ timeout: 20_000 })
    })
  }
  await rec.step('Back to Welcome: the recent list', () => toWelcome(page))
  let got = await order()
  rec.check(got.join() === [...names].reverse().join(), 'recents are most recent first', got.join(' > '))
  rec.note(`recent row text: ${JSON.stringify((await list().locator('li').first().innerText()).replace(/\n+/g, ' / '))}`)

  await rec.step('Open r2 from recents, back to Welcome', async () => {
    await list().locator('button.open', { hasText: 'r2-layout' }).click()
    await nav(page).waitFor({ timeout: 20_000 })
    await toWelcome(page)
  })
  got = await order()
  rec.check(got[0] === 'r2-layout' && got.length === 4, 'reopening moves a repository to the top, no duplicate', got.join(' > '))

  await rec.step('Quit the app and start it again', async () => {
    await page.waitForTimeout(8000)
    await quitApp()
    await new Promise((r) => setTimeout(r, 1000))
    await launchApp()
    page = await rec.attach()
    await page.waitForTimeout(2500)
  })
  await rec.shot('after restart: where the app opens')
  rec.note(`after restart: url ${page.url()} rail ${await nav(page).isVisible()} name ${(await nav(page).isVisible()) ? await switcher(page).innerText() : ''}`)
  await rec.step('Welcome after restart', () => toWelcome(page))
  const after = await order()
  rec.check(after.join() === got.join(), 'recents survive a restart in the same order', after.join(' > '))

  await rec.step('Open r1, close the app 1 s later, start it again', async () => {
    await list().locator('button.open', { hasText: 'r1-concept' }).click()
    await nav(page).waitFor({ timeout: 20_000 })
    await page.waitForTimeout(1000)
    await quitApp()
    await launchApp()
    page = await rec.attach()
    await page.waitForTimeout(2500)
  })
  await rec.shot('after the quick close: where the app opens')
  const reopened = (await nav(page).isVisible()) ? await switcher(page).innerText() : 'Welcome'
  rec.check(reopened.includes('r1-concept'), 'a repository opened 1 s before closing the app is reopened on the next start', reopened.replace(/\s+/g, ' '))
  await toWelcome(page)
  const quick = await order()
  rec.check(quick[0] === 'r1-concept', 'and it is first in recents', quick.join(' > '))

  await rec.step('Delete r3 on disk, then click it in recents', async () => {
    rmSync(join(root, 'r3-anim'), { recursive: true, force: true })
    await list().locator('button.open', { hasText: 'r3-anim' }).click()
    await page.waitForTimeout(2000)
  })
  const errs = await visibleErrors(page)
  rec.note(`deleted-repository message: ${JSON.stringify(errs)}`)
  rec.check(await onWelcome(page), 'a deleted repository stays on Welcome')
  rec.check(errs.join(' ').length > 0 && !PLAIN.test(errs.join(' ')), 'a deleted repository gives a plain-word message', errs.join(' | ').slice(0, 300))
  rec.check(/r3-anim/.test(errs.join(' ')) || /no longer|missing|not found|moved|deleted/i.test(errs.join(' ')), 'the message says the folder is gone', errs.join(' | ').slice(0, 300))
  rec.check(!existsSync(join(root, 'r3-anim')), 'clicking a deleted repository does not recreate its folder')

  await rec.step('Remove r3 from recents', async () => {
    await page.getByRole('button', { name: 'Remove r3-anim from recent' }).click()
    await page.waitForTimeout(500)
  })
  got = await order()
  rec.check(!got.includes('r3-anim') && got.length === 3, 'the missing repository can be removed from recents', got.join(' > '))

  await rec.step('Open r1: the rail menu no longer lists r3', async () => {
    await list().locator('button.open', { hasText: 'r1-concept' }).click()
    await nav(page).waitFor({ timeout: 20_000 })
    await switcher(page).click()
    await page.getByRole('menuitem', { name: /Open a repository/ }).waitFor()
  })
  const labels = await page.getByRole('menuitem').allInnerTexts()
  rec.check(!labels.some((l) => l.includes('r3-anim')), 'rail menu matches the recent list', JSON.stringify(labels))
  await page.keyboard.press('Escape')

  await rec.finish()
  await closeTerm()
})
