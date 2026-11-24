import { appendFileSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { Recorder, closeTerm, workDir } from './rec'
import { go, openRepo, toWelcome, visibleErrors } from './gui'
import { copyCorpus, hashCompare, intCheck } from './integrity'

const U = 'unity/megacity-metro-13069724080c2aacc89b735206a7af1c9df81b51/megacity-metro-13069724080c2aacc89b735206a7af1c9df81b51/Assets/Art'
// One small file of each kind. Value = name in the test repository.
const TYPES: Record<string, string> = {
  '3D_model_of_Shucaris_ankylosskelos_appendage.png': 'img/still.png',
  'freepik__talk__71826.jpeg': 'img/photo.jpeg',
  'Workstation_cube_lid_off.webp': 'img/render.webp',
  '3D_Model_-_Sperry_F3619-00.gif': 'img/turntable.gif',
  '3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg': 'img/plan.svg',
  'blender/blender-3.6-splash/blender-3.6-splash/pro/lib/maps/patterns/reflection_02/reflection_02.tif': 'img/reflection.tif',
  'video-implementation/openexr-images-main/openexr-images-main/TestImages/stripes.exr': 'img/stripes.exr',
  [`${U}/UI/MegacityMetro_Atlas.psd`]: 'img/atlas.psd',
  '101394-video-720.mp4': 'video/clip.mp4',
  'video-variants/bbb-5s-h264.mov': 'video/bunny.mov',
  [`${U}/UI/menu/MegacityMetroMenu.webm`]: 'video/menu.webm',
  '3D-Bite-Modeling-and-Feeding-Mechanics-of-the-Largest-Living-Amphibian-the-Chinese-Giant-Salamander-pone.0121885.s004.ogv': 'video/bite.ogv',
  '_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav': 'audio/aria.wav',
  '_Amir_Tangsiri__Dokhtare_Koli.flac': 'audio/song.flac',
  '_Into_the_Oceans_and_the_Air_.ogg': 'audio/oceans.ogg',
  'pdf/1710.02185v1.pdf': 'docs/paper.pdf',
  '12690118_5053480.ai': 'docs/poster.ai',
  '12690118_5053481.eps': 'docs/poster.eps',
  'indesign/Guideline - drdesign/brand_guideline-v2.indd': 'docs/guideline.indd',
  'figma/UI-Kit-Italia (4).fig': 'docs/kit.fig',
  'after-effects/mixkit-wedding-invitation-opener-803/WeddingInvitation_Opener folder/WeddingInvitation_Opener.aep': 'docs/opener.aep',
  '1965_ac_shelby_427_cobra_sc.glb': '3d/cobra.glb',
  '1900s_telephone.stl': '3d/telephone.stl',
  'blender/blender-3.6-splash/blender-3.6-splash/pro/lib/cam/camera_rig.blend': '3d/camera_rig.blend',
  [`${U}/Models/Environment/Rooftops/Airducts/Airduct_straight_lod2.fbx`]: '3d/airduct.fbx',
  '93-ply/ply/Dragon 2.5_ply.ply': '3d/dragon.ply',
}
const PREVIEWABLE: Record<string, 'image' | 'video' | 'audio'> = { png: 'image', jpeg: 'image', webp: 'image', gif: 'image', svg: 'image', mp4: 'video', mov: 'video', webm: 'video', ogv: 'video', wav: 'audio', flac: 'audio', ogg: 'audio' }

const tree = (page: Page) => page.getByRole('tree', { name: 'All files' })
const find = (page: Page) => page.getByRole('searchbox', { name: 'Find a file' })

interface Shown { kind: string; ok: boolean; detail: string; text: string }
/** What the preview panel really shows right now. */
function readPreview(page: Page): Promise<Shown> {
  return page.evaluate(() => {
    const p = document.querySelector('[data-testid="preview"]') as HTMLElement | null
    if (!p) return { kind: 'no-panel', ok: false, detail: '', text: '' }
    const text = p.innerText.replace(/\s+/g, ' ').trim()
    const img = p.querySelector('img')
    if (img) return { kind: 'image', ok: img.complete && img.naturalWidth > 0, detail: `${img.naturalWidth}x${img.naturalHeight}, shown ${img.clientWidth}x${img.clientHeight}`, text }
    const v = p.querySelector('video')
    if (v) return { kind: 'video', ok: v.readyState >= 1 && v.videoWidth > 0, detail: `readyState ${v.readyState}, ${v.videoWidth}x${v.videoHeight}, ${v.duration?.toFixed?.(1)} s, error ${v.error?.code ?? 'none'}`, text }
    const a = p.querySelector('audio')
    if (a) return { kind: 'audio', ok: a.readyState >= 1 && a.duration > 0, detail: `readyState ${a.readyState}, ${a.duration?.toFixed?.(1)} s, error ${a.error?.code ?? 'none'}`, text }
    if (p.querySelector('[role="status"]')) return { kind: 'loading', ok: false, detail: '', text }
    if (p.querySelector('[role="alert"]')) return { kind: 'error', ok: false, detail: '', text }
    return { kind: 'note', ok: text.length > 0, detail: '', text }
  })
}

async function select(page: Page, name: string): Promise<{ shown: Shown; ms: number }> {
  await find(page).fill(name)
  await page.waitForTimeout(350)
  const t = Date.now()
  await tree(page).getByRole('treeitem', { name: new RegExp(name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click()
  let shown = await readPreview(page)
  for (let i = 0; i < 40 && (shown.kind === 'loading' || shown.kind === 'no-panel' || ((shown.kind === 'image' || shown.kind === 'video' || shown.kind === 'audio') && !shown.ok)); i++) {
    await page.waitForTimeout(250)
    shown = await readPreview(page)
  }
  return { shown, ms: Date.now() - t }
}

test('F-08 Preview of every media type', async () => {
  const rec = await Recorder.start('F-08', 'preview-types', { gui: true })
  const page = rec.page!
  const repo = workDir('F-08')
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  await rec.cli(repo, ['init'])
  const want = await copyCorpus(repo, TYPES)
  let r = await rec.cli(repo, ['add', '--all'])
  rec.note(`PERF add 26 mixed files: ${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB`)
  r = await rec.cli(repo, ['commit', '-m', 'One of every kind'])
  rec.check(r.code === 0, 'setup: every kind is committed', (r.stdout + r.stderr).slice(-200))

  await rec.step('Open the repository, All files, Flat view', async () => {
    await toWelcome(page, true)
    await openRepo(page, repo)
    await go(page, 'files/all')
    await tree(page).getByRole('treeitem').first().waitFor({ timeout: 30_000 })
    await page.getByRole('group', { name: 'Folder view' }).getByText('Flat', { exact: true }).click()
  })
  await tree(page).getByRole('treeitem').first().click()
  await page.waitForTimeout(800)
  rec.check(await page.getByTestId('inspector').isVisible().catch(() => false), 'the Details panel is on screen', `window ${await page.evaluate(() => `${innerWidth}x${innerHeight}`)}`)

  const table: string[] = []
  for (const rel of Object.values(TYPES)) {
    const name = rel.split('/').pop()!
    const ext = name.split('.').pop()!.toLowerCase()
    let got: { shown: Shown; ms: number } = { shown: { kind: 'none', ok: false, detail: '', text: '' }, ms: 0 }
    await rec.step(`Select ${name}`, async () => { got = await select(page, name) })
    const { shown, ms } = got
    const expect = PREVIEWABLE[ext]
    table.push(`| ${name} | ${expect ?? 'no preview'} | ${shown.kind}${shown.ok ? '' : ' (not ok)'} | ${shown.detail || shown.text.slice(0, 70)} | ${ms} ms |`)
    if (expect) {
      const honest = shown.kind === 'note' && /cannot display/i.test(shown.text)
      rec.check((shown.kind === expect && shown.ok) || honest, `${name}: a real ${expect} preview, or an honest note`, `${shown.kind} ${shown.detail} "${shown.text.slice(0, 120)}"`)
      if (honest) rec.note(`${name}: honest fallback "${shown.text}"`)
    } else {
      rec.check(shown.kind === 'note' && /No preview for/i.test(shown.text), `${name}: says there is no preview (no blank panel, no broken image)`, `${shown.kind} "${shown.text.slice(0, 120)}"`)
    }
    if (ext === 'ogv') rec.check(shown.kind === 'note' && /cannot display/i.test(shown.text), 'BUG-015: a sound-only .ogv shows the "cannot display" note, not an empty player', `${shown.kind} ${shown.detail} "${shown.text.slice(0, 120)}"`)
    rec.check(ms < 10_000, `${name}: the panel settles within 10 s`, `${ms} ms`)
  }
  rec.note(`PREVIEW TABLE\n| File | Expected | Shown | Detail | Time |\n|---|---|---|---|---|\n${table.join('\n')}`)
  const details = (await page.getByTestId('inspector').innerText()).replace(/\n+/g, ' / ')
  rec.note(`Details panel for the last file: ${details.slice(0, 400)}`)

  // --- media details for a video
  await rec.step('Select clip.mp4 and read the details rows', async () => { await select(page, 'clip.mp4') })
  const vid = (await page.getByTestId('inspector').innerText()).replace(/\n+/g, ' / ')
  rec.note(`Details for clip.mp4: ${vid.slice(0, 500)}`)
  rec.check(/5\.\d MB|4\.\d MB|5,128,888|5128888/.test(vid), 'the size shown matches the file (5.1 MB)', vid.slice(0, 200))
  const mi = await rec.cli(repo, ['media', 'info', 'video/clip.mp4'])
  rec.check(/width:?\s*1280\b/i.test(mi.stdout + mi.stderr) && /height:?\s*720\b/i.test(mi.stdout + mi.stderr) && !/83886080|47185920/.test(mi.stdout + mi.stderr), 'BUG-016: media info reports clip.mp4 as 1280 x 720 (16.16 fixed point is shifted)', (mi.stdout + mi.stderr).replace(/\n+/g, ' / ').slice(0, 300))
  rec.note(`CLI media info clip.mp4: ${(mi.stdout + mi.stderr).replace(/\n+/g, ' / ').slice(0, 300)}`)

  // --- edited and deleted working files
  await rec.step('Outside the app: delete still.png, replace photo.jpeg with the webp bytes', async () => {
    rmSync(join(repo, 'img', 'still.png'))
    appendFileSync(join(repo, 'docs', 'paper.pdf'), '\n%qa\n')
    await page.waitForTimeout(3200)
  })
  let got = { shown: { kind: '', ok: false, detail: '', text: '' }, ms: 0 }
  await rec.step('Select the deleted still.png', async () => { got = await select(page, 'still.png') })
  rec.check(got.shown.kind === 'note' && /deleted/i.test(got.shown.text), 'a deleted file says so, with no broken image', `${got.shown.kind} "${got.shown.text}"`)
  rec.note(`deleted file: is there a way to see or restore the saved copy from here? panel: ${(await page.getByTestId('inspector').innerText()).replace(/\n+/g, ' / ').slice(0, 300)}`)

  // --- keyboard: arrow through rows
  await rec.step('Clear the search, press Arrow Down 5 times in the list', async () => {
    await find(page).fill('')
    await page.waitForTimeout(400)
    await tree(page).getByRole('treeitem').first().click()
    for (let i = 0; i < 5; i++) { await page.keyboard.press('ArrowDown'); await page.waitForTimeout(250) }
    await page.waitForTimeout(1200)
  })
  const sel = await tree(page).locator('[aria-selected="true"]').innerText().catch(() => '')
  rec.note(`row selected after 5 Arrow Down: "${sel.replace(/\n+/g, ' / ')}"; preview: ${JSON.stringify(await readPreview(page)).slice(0, 200)}`)
  rec.check(sel.length > 0, 'arrow keys move the selection and the preview follows')
  rec.check((await visibleErrors(page)).filter((e) => /fail|error|could not/i.test(e)).length === 0, 'no error panels during the tour', JSON.stringify(await visibleErrors(page)).slice(0, 300))

  await rec.cli(repo, ['reset', '--hard', 'HEAD'], { quietFrame: true })
  await intCheck(rec, repo, want)
  await rec.finish()
  await closeTerm()
})

test('F-09 Changes: exclude, and Discard all unsaved changes', async () => {
  const rec = await Recorder.start('F-09', 'discard', { gui: true })
  const page = rec.page!
  const repo = workDir('F-09')
  rmSync(repo, { recursive: true, force: true })
  mkdirSync(repo, { recursive: true })
  await rec.cli(repo, ['init'])
  const want = await copyCorpus(repo, {
    '12690118_5053480.ai': 'design/poster.ai',
    '1965_ac_shelby_427_cobra_sc.glb': 'models/cobra.glb',
    'pdf/1710.02185v1.pdf': 'docs/paper.pdf',
    'freepik__talk__71826.jpeg': 'refs/talk.jpeg',
  })
  await rec.cli(repo, ['add', '--all'], { quietFrame: true })
  await rec.cli(repo, ['commit', '-m', 'first plates'], { quietFrame: true })
  const changes = page.getByRole('tree', { name: 'Changes' })
  const dialog = page.getByRole('alertdialog').or(page.getByRole('dialog')).first()
  const mess = () => {
    appendFileSync(join(repo, 'docs', 'paper.pdf'), '\n%edited\n') // edited, tracked
    rmSync(join(repo, 'refs', 'talk.jpeg'), { force: true }) // deleted, tracked
    writeFileSync(join(repo, 'new-untracked.txt'), 'never saved, never included\n') // new, not included
    writeFileSync(join(repo, 'new-included.txt'), 'never saved, but included\n') // new, included
  }
  const porcelain = async () => (await rec.cli(repo, ['status', '--porcelain'])).stdout.trim().split('\n').filter(Boolean)

  await rec.step('Open the repository. Outside the app: edit, delete, and add two files', async () => {
    await toWelcome(page, true)
    await openRepo(page, repo)
    mess()
    await page.waitForTimeout(3200)
    await go(page, 'files/changes')
    await changes.getByRole('treeitem').first().waitFor({ timeout: 20_000 })
    await page.getByRole('group', { name: 'Folder view' }).getByText('Tree', { exact: true }).click()
    await page.waitForTimeout(400)
  })
  rec.check((await page.getByTestId('changes-summary').innerText()).includes('4'), 'four changes are listed', await page.getByTestId('changes-summary').innerText())
  rec.note(`Changes rows: ${(await changes.innerText()).replace(/\n+/g, ' / ').slice(0, 300)}`)

  await rec.step('Include new-included.txt and the edited paper.pdf', async () => {
    await changes.getByRole('checkbox', { name: 'Include new-included.txt' }).click()
    await page.waitForTimeout(900)
    await changes.getByRole('checkbox', { name: 'Include paper.pdf' }).click()
    await page.waitForTimeout(900)
  })
  let st = await porcelain()
  rec.check(st.some((l) => /^A\s+new-included\.txt/.test(l)) && st.some((l) => /^M\s+docs\/paper\.pdf/.test(l)), 'CLI agrees: both are staged', st.join(' | '))
  await rec.step('Exclude paper.pdf again', async () => {
    await changes.getByRole('checkbox', { name: 'Include paper.pdf' }).click()
    await page.waitForTimeout(900)
  })
  st = await porcelain()
  rec.check(st.some((l) => /^ M docs\/paper\.pdf/.test(l)), 'Exclude unstages the file and keeps the edit on disk', st.join(' | '))

  await rec.step('Include the deleted talk.jpeg', async () => {
    await changes.getByRole('checkbox', { name: 'Include talk.jpeg' }).click()
    await page.waitForTimeout(900)
  })
  st = await porcelain()
  rec.check(st.some((l) => /^D\s+refs\/talk\.jpeg/.test(l)), 'a deletion can be included', st.join(' | '))

  // --- is there a per-file discard?
  await rec.step('Right-click the edited paper.pdf row', async () => {
    await changes.getByRole('treeitem', { name: /paper\.pdf/ }).click({ button: 'right' })
    await page.waitForTimeout(600)
  })
  const menu = await page.getByRole('menuitem').allInnerTexts()
  rec.note(`row context menu: ${JSON.stringify(menu)}; details panel buttons: ${JSON.stringify(await page.getByTestId('inspector').getByRole('button').allInnerTexts().catch(() => []))}`)
  // The app offers per-file discard as "Discard selected (N)" once rows are selected (a plain click selects one).
  await page.keyboard.press('Escape'); await changes.getByRole('treeitem', { name: /paper\.pdf/ }).click(); await page.waitForTimeout(400)
  const selBtn = await page.getByRole('button', { name: /^Discard selected \(1\)/ }).count()
  rec.note(`Discard selected (1) button after selecting one row: ${selBtn}`)
  rec.check(selBtn > 0 || menu.some((m) => /discard|undo|restore/i.test(m)) || (await page.getByTestId('inspector').getByRole('button', { name: /discard|undo|restore/i }).count()) > 0, 'one file can be discarded on its own (without losing the other changes)', JSON.stringify(menu))
  await page.keyboard.press('Escape')

  // --- Discard all: cancel first
  await rec.step('More actions > Discard all unsaved changes…', async () => {
    await page.getByRole('button', { name: 'More actions' }).click()
    await page.getByRole('menuitem', { name: /Discard all unsaved changes/ }).click()
    await dialog.waitFor()
  })
  const text = (await dialog.innerText()).replace(/\n+/g, ' / ')
  rec.note(`discard dialog: ${text}`)
  rec.check(/cannot be undone|permanent|lost|can't be undone/i.test(text), 'the dialog says the loss is permanent', text.slice(0, 300))
  rec.check(/1 new file will be deleted:\s*new-included\.txt/.test(text) && !/new-untracked\.txt/.test(text.split('will be deleted:')[1]?.split('.')[0] ?? ''), 'BUG-017: the dialog names the included new file it will delete (new-included.txt) and not the never-included one', text.slice(0, 400))
  rec.check((await dialog.getByTestId('discard-new').count()) === 1, 'BUG-017: the "new files will be deleted" line is on screen')
  const focused = await page.evaluate(() => (document.activeElement as HTMLElement | null)?.innerText ?? '')
  rec.note(`focused control when the dialog opens: "${focused}"`)
  rec.check(!/discard|reset|delete/i.test(focused), 'the destructive button is not the default focus', focused)
  await rec.step('Cancel the dialog', async () => {
    await dialog.getByRole('button', { name: 'Keep my files' }).click()
    await page.waitForTimeout(600)
  })
  const kept = await porcelain()
  rec.check(kept.length === 4, 'Cancel changes nothing', kept.join(' | '))

  await rec.step('Discard all again, and confirm', async () => {
    await page.getByRole('button', { name: 'More actions' }).click()
    await page.getByRole('menuitem', { name: /Discard all unsaved changes/ }).click()
    await dialog.waitFor()
    const go2 = dialog.getByRole('button', { name: 'Discard and go back' })
    rec.check(await go2.isDisabled(), 'the Discard button is disabled until the box is ticked')
    await dialog.getByRole('checkbox', { name: /I understand/ }).click()
    await go2.click()
    await page.waitForTimeout(3000)
  })
  const after = await porcelain()
  rec.note(`status after Discard all: ${JSON.stringify(after)}`)
  rec.note(`on disk after Discard all: new-untracked.txt ${existsSync(join(repo, 'new-untracked.txt'))}, new-included.txt ${existsSync(join(repo, 'new-included.txt'))}, talk.jpeg ${existsSync(join(repo, 'refs', 'talk.jpeg'))}`)
  await hashCompare(rec, repo, want, 'after Discard all: tracked files equal the saved version')
  rec.check(existsSync(join(repo, 'new-untracked.txt')), 'a new file that was never included is not deleted')
  rec.check(!existsSync(join(repo, 'new-included.txt')), 'BUG-017: the included new file is gone, as the dialog said it would be (warned, not a surprise)', `exists ${existsSync(join(repo, 'new-included.txt'))}`)
  rec.note(`screen after Discard all: ${(await page.locator('#app').innerText()).replace(/\n+/g, ' / ').slice(-300)}`)
  rec.note(`toast offers an undo: ${/undo/i.test((await visibleErrors(page)).join(' '))} ${JSON.stringify(await visibleErrors(page)).slice(0, 200)}`)
  await intCheck(rec, repo)
  await rec.finish()
  await closeTerm()
})
