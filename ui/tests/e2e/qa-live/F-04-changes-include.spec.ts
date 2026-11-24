import { rmSync } from 'node:fs'
import { test } from '@playwright/test'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo } from './gui'
import { copyCorpus, intCheck } from './integrity'

// F-04: Changes list. Include and Exclude one file, a folder, all; Space key; Tree and Flat;
// "Preview only". After every action the CLI must report the same staged set.
const MIX: Record<string, string> = {
  '12690118_5053480.ai': 'design/poster.ai',
  '12690118_5053481.eps': 'design/poster.eps',
  '1965_ac_shelby_427_cobra_sc.glb': 'models/cobra.glb',
  '1965_AC_Shelby_427_Cobra_SC.usdz': 'models/cobra.usdz',
  'pdf/1710.02185v1.pdf': 'docs/paper.pdf',
  'freepik__talk__71826.jpeg': 'refs/talk.jpeg',
  '3D_model_of_Shucaris_ankylosskelos_appendage.png': 'refs/appendage.png',
  'sketch': 'sketch',
}

test('F-04 Changes: Include and Exclude', async () => {
  const rec = await Recorder.start('F-04', 'changes-include', { gui: true })
  const page = rec.page!
  const repo = workDir('F-04')
  rmSync(repo, { recursive: true, force: true })
  workDir('F-04')
  await rec.cli(repo, ['init'])
  const want = await copyCorpus(repo, MIX)
  const N = Object.keys(want).length
  rec.note(`${N} files copied`)

  const staged = async () => {
    const r = await rec.cli(repo, ['status', '--porcelain'], { quietFrame: true })
    const lines = r.stdout.split('\n').filter(Boolean)
    return { staged: lines.filter((l) => /^[AMD]/.test(l)).map((l) => l.slice(3).trim()), all: lines.length }
  }
  const tree = page.getByRole('tree', { name: 'Changes' })
  const count = () => page.getByTestId('include-count').innerText()
  const settle = async (re: RegExp) => {
    await page.waitForFunction((src) => new RegExp(src).test((document.querySelector('[data-testid="include-count"]') as HTMLElement | null)?.innerText ?? ''), re.source, { timeout: 120_000 })
    await page.waitForTimeout(400)
  }

  await rec.step('Open the repository, Files > Changes', async () => {
    await openRepo(page, repo)
    await go(page, 'files/changes')
    await tree.getByRole('treeitem').first().waitFor({ timeout: 30_000 })
  })
  const s0 = await staged()
  rec.check(s0.all === N, 'CLI status lists every copied file', `${s0.all} of ${N}`)
  rec.check(new RegExp(`0 of ${N}\\b`).test(await count()), `the dock reads "0 of ${N} changes included"`, await count())
  rec.check((await page.getByTestId('changes-summary').innerText()).includes(String(N)), 'the summary shows the change count', await page.getByTestId('changes-summary').innerText())
  rec.check(await page.getByRole('button', { name: 'Save version', exact: true }).isDisabled(), 'Save version is disabled with nothing included')

  // --- one file
  await rec.step('Expand design/ and include poster.ai', async () => {
    const dir = tree.getByRole('treeitem', { name: /design\// })
    if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.locator('.chev').click()
    await tree.getByRole('checkbox', { name: 'Include poster.ai' }).click()
    await settle(new RegExp(`1 of ${N}\\b`))
  })
  let s = await staged()
  rec.check(s.staged.length === 1 && s.staged[0] === 'design/poster.ai', 'CLI: exactly design/poster.ai is staged', s.staged.join(','))
  rec.check((await tree.getByRole('checkbox', { name: 'Include design' }).getAttribute('aria-checked')) === 'mixed', 'the design/ folder checkbox shows a partial state')

  await rec.step('Exclude poster.ai again', async () => {
    await tree.getByRole('checkbox', { name: 'Include poster.ai' }).click()
    await settle(new RegExp(`0 of ${N}\\b`))
  })
  s = await staged()
  rec.check(s.staged.length === 0, 'CLI: nothing staged after Exclude', s.staged.join(','))

  // --- a folder
  const inSketch = Object.keys(want).filter((f) => f.startsWith('sketch/')).length
  await rec.step(`Include the sketch/ folder (${inSketch} files)`, async () => {
    await tree.getByRole('checkbox', { name: 'Include sketch', exact: true }).click()
    await settle(new RegExp(`${inSketch} of ${N}\\b`))
  })
  s = await staged()
  rec.check(s.staged.length === inSketch && s.staged.every((f) => f.startsWith('sketch/')), `CLI: the ${inSketch} sketch files and only those are staged`, `${s.staged.length} staged`)

  // --- Space on the focused row
  await rec.step('Keyboard: focus models/, expand, arrow to cobra.glb, press Space', async () => {
    const dir = tree.getByRole('treeitem', { name: /models\// })
    await dir.click()
    if ((await dir.getAttribute('aria-expanded')) !== 'true') await page.keyboard.press('ArrowRight')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Space')
    await settle(new RegExp(`${inSketch + 1} of ${N}\\b`))
  })
  s = await staged()
  rec.check(s.staged.filter((f) => f.startsWith('models/')).length === 1, 'CLI: Space staged one models/ file', s.staged.filter((f) => f.startsWith('models/')).join(','))

  // --- Flat and Tree
  await rec.step('Switch to Flat view', async () => {
    await page.getByRole('group', { name: 'Folder view' }).getByText('Flat', { exact: true }).click()
    await page.waitForTimeout(500)
  })
  rec.check((await tree.locator('div.dir').count()) === 0, 'Flat view has no folder rows')
  rec.check(new RegExp(`${inSketch + 1} of ${N}\\b`).test(await count()), 'the included count is the same in Flat view', await count())
  await rec.step('Back to Tree view', async () => {
    await page.getByRole('group', { name: 'Folder view' }).getByText('Tree', { exact: true }).click()
    await page.waitForTimeout(500)
  })

  // --- all
  await rec.step('Include all', async () => {
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    await settle(new RegExp(`${N} of ${N}\\b`))
  })
  s = await staged()
  rec.check(s.staged.length === N, `CLI: all ${N} files staged`, `${s.staged.length}`)
  rec.check(!(await page.getByRole('button', { name: 'Save version', exact: true }).isDisabled()) || (await page.getByLabel('Message').inputValue()) === '', 'Save version waits only for a message now')

  await rec.step('Exclude all', async () => {
    await page.getByRole('button', { name: 'Exclude all', exact: true }).click()
    await settle(new RegExp(`0 of ${N}\\b`))
  })
  s = await staged()
  rec.check(s.staged.length === 0, 'CLI: nothing staged after Exclude all', `${s.staged.length}`)

  // --- Preview only (--dry-run)
  await rec.step('Staging options: Preview only, then Include all', async () => {
    await page.getByTestId('stage-more').locator('summary').click()
    await page.getByTestId('stage-more').getByText('Preview only').click()
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    await page.waitForTimeout(2500)
  })
  s = await staged()
  rec.check(s.staged.length === 0, 'CLI: Preview only staged nothing', `${s.staged.length}`)
  rec.note(`dock after preview: ${(await page.getByTestId('save-dock').innerText()).replace(/\n+/g, ' / ').slice(0, 400)}`)
  rec.check(new RegExp(`0 of ${N}\\b`).test(await count()), 'the dock still reads 0 included after a preview', await count())
  await rec.step('Turn Preview only off again', async () => {
    await page.getByTestId('stage-more').getByText('Preview only').click()
  })

  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})
