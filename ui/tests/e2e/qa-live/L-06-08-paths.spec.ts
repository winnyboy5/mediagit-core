import { cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CORPUS } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { openRepo, toWelcome, visibleErrors } from './gui'
import { copyCorpus, intCheck, restoreCheck, sha256, walk } from './integrity'
import { startServer } from './server'

const PLAIN = /os error|panick|Error:\s*Error|exit code|stack backtrace|undefined|\[object/i
const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' })
const appText = async (page: Page) => (await page.locator('#app').innerText()).replace(/\n+/g, ' / ')
const halfRepo = (d: string) => existsSync(d) && readdirSync(d).length > 0
const out = (r: { stdout: string; stderr: string }) => `${r.stdout}\n${r.stderr}`.trim()

test('L-06 Repository on a path with spaces, brackets and unicode', async () => {
  const rec = await Recorder.start('L-06', 'unicode-path', { gui: true })
  const page = rec.page!
  const root = workDir('L-06')
  rmSync(root, { recursive: true, force: true })
  const repo = join(root, 'My Film (final) ünïcode 日本語')
  mkdirSync(repo, { recursive: true })
  const srv = await startServer(join(root, 'server'), ['film'])
  try {
    await rec.cli(repo, ['init'])
    const want = await copyCorpus(repo, {
      'pdf/1710.02185v1.pdf': 'カット 01 (v2) – final.pdf',
      '1965_ac_shelby_427_cobra_sc.glb': 'ショット/é à ñ.glb',
      '12690118_5053480.ai': 'design/poster [draft] #1.ai',
      'freepik__talk__71826.jpeg': "refs/it's & 100% ok.jpeg",
    })
    let r = await rec.cli(repo, ['add', '--all'])
    rec.check(r.code === 0, 'add --all works on the unicode path', out(r).slice(0, 300))
    r = await rec.cli(repo, ['status', '--porcelain'])
    rec.check(Object.keys(want).every((f) => r.stdout.includes(f.split('/').pop()!)), 'status prints every odd file name intact', r.stdout.slice(0, 400))
    r = await rec.cli(repo, ['commit', '-m', 'プレート first plates – é'])
    rec.check(r.code === 0, 'commit with a unicode message works', out(r).slice(0, 200))
    r = await rec.cli(repo, ['log'])
    rec.check(r.stdout.includes('プレート first plates – é'), 'log shows the unicode message intact', r.stdout.slice(0, 300))
    r = await rec.cli(repo, ['status', '--machine'])
    rec.check(!/\\u\{?fffd|�/i.test(r.stdout), '--machine output has no replacement characters', r.stdout.slice(0, 300))

    r = await rec.cli(repo, ['branch', 'create', 'look-dev'])
    r = await rec.cli(repo, ['branch', 'switch', 'look-dev'])
    rec.check(r.code === 0, 'branch create and switch work', out(r).slice(0, 200))
    r = await rec.cli(repo, ['branch', 'create', 'ルック/開発'])
    rec.note(`unicode branch name: exit ${r.code} ${out(r).slice(0, 200)}`)
    rec.check(r.code === 0 || !PLAIN.test(out(r)), 'a unicode branch name works or is refused in plain words', out(r).slice(0, 200))
    await rec.cli(repo, ['branch', 'switch', 'main'])

    await rec.cli(repo, ['remote', 'add', 'origin', srv.url('film')])
    r = await rec.cli(repo, ['push', '-u', 'origin', 'main'])
    rec.check(r.code === 0, 'push from the unicode path works', out(r).slice(-300))
    const clone = join(root, 'クローン copy (2)')
    r = await rec.cli(root, ['clone', srv.url('film'), clone])
    rec.check(r.code === 0, 'clone into a unicode path works', out(r).slice(-300))
    await intCheck(rec, clone, want)

    // --- the app on the same repository
    await rec.step('Open the unicode-path repository in the app', async () => {
      await toWelcome(page, true)
      await openRepo(page, repo)
      await page.waitForTimeout(1200)
    })
    const name = await page.getByTestId('repo-switcher').innerText()
    rec.check(name.includes('日本語') && name.includes('ünïcode'), 'rail shows the full unicode repository name', name)
    let body = await appText(page)
    rec.check(body.includes('ショット') && body.includes('カット 01 (v2) – final.pdf'), 'Files hub shows unicode folder and file names', body.slice(0, 400))
    await rec.step('Drop a new unicode-named file in from outside', async () => {
      cpSync(join(CORPUS, 'pdf', '1710.02185v1.pdf'), join(repo, 'ショット', '新しい ノート (β).pdf'))
      await page.waitForTimeout(3500)
    })
    await rec.step('Files > Changes', async () => {
      await page.evaluate(() => { window.location.hash = '#/files/changes' })
      await page.waitForTimeout(1500)
    })
    body = await appText(page)
    rec.check(body.includes('新しい ノート (β).pdf'), 'the watcher lists the new unicode file in Changes within 5 s', body.slice(0, 400))
    await rec.step('History hub on the unicode repository', async () => {
      await page.evaluate(() => { window.location.hash = '#/history' })
      await page.waitForTimeout(1500)
    })
    body = await appText(page)
    rec.check(body.includes('プレート first plates – é'), 'History shows the unicode message intact', body.slice(0, 300))
    rec.check((await visibleErrors(page)).filter((e) => /fail|error/i.test(e)).length === 0, 'no error panels on the unicode repository', JSON.stringify(await visibleErrors(page)))
    rmSync(join(repo, 'ショット', '新しい ノート (β).pdf'))
    await intCheck(rec, repo, want)
  } finally {
    srv.stop()
  }
  await rec.finish()
  await closeTerm()
})

test('L-07 Clone: CLI and app, branch, non-empty folder, wrong URL, server down', async () => {
  const rec = await Recorder.start('L-07', 'clone', { gui: true })
  const page = rec.page!
  const root = workDir('L-07')
  rmSync(root, { recursive: true, force: true })
  const src = join(root, 'src')
  mkdirSync(src, { recursive: true })
  const srv = await startServer(join(root, 'server'), ['show'])
  const dlg = page.getByRole('dialog', { name: 'Clone repository' })
  let stopped = false
  try {
    await rec.cli(src, ['init'], { quietFrame: true })
    const want = await copyCorpus(src, {
      '12690118_5053480.ai': 'design/poster.ai',
      '1965_ac_shelby_427_cobra_sc.glb': 'models/cobra.glb',
      'pdf/1710.02185v1.pdf': 'docs/paper.pdf',
    })
    await rec.cli(src, ['add', '--all'], { quietFrame: true })
    await rec.cli(src, ['commit', '-m', 'main: first plates'], { quietFrame: true })
    await rec.cli(src, ['branch', 'create', 'look-dev'], { quietFrame: true })
    await rec.cli(src, ['branch', 'switch', 'look-dev'], { quietFrame: true })
    const wantDev = { ...want, ...(await copyCorpus(src, { 'freepik__talk__71826.jpeg': 'refs/talk.jpeg' })) }
    await rec.cli(src, ['add', '--all'], { quietFrame: true })
    await rec.cli(src, ['commit', '-m', 'look-dev: add a reference'], { quietFrame: true })
    await rec.cli(src, ['branch', 'switch', 'main'], { quietFrame: true })
    await rec.cli(src, ['remote', 'add', 'origin', srv.url('show')], { quietFrame: true })
    let r = await rec.cli(src, ['push', '--all', 'origin'])
    rec.check(r.code === 0, 'setup: push --all works', out(r).slice(-300))

    // --- CLI
    const c1 = join(root, 'c1')
    r = await rec.cli(root, ['clone', srv.url('show'), c1])
    rec.check(r.code === 0, 'CLI clone works', out(r).slice(-200))
    await intCheck(rec, c1, want)
    rec.check(!existsSync(join(c1, 'refs', 'talk.jpeg')), 'the default clone is on main (no look-dev file)')
    const f1 = await rec.cli(c1, ['fsck'], { quietFrame: true })
    rec.check(f1.code === 0, 'BUG-006: a fresh two-branch clone passes fsck (lazy branches are not an error)', out(f1).slice(-300))
    const sw = await rec.cli(c1, ['branch', 'switch', 'look-dev'])
    rec.check(sw.code === 0 && existsSync(join(c1, 'refs', 'talk.jpeg')), 'BUG-006: branch switch downloads the branch that clone left lazy', out(sw).slice(-300))
    const f1b = await rec.cli(c1, ['fsck'], { quietFrame: true })
    rec.check(f1b.code === 0, 'BUG-006: fsck still passes after the lazy branch was downloaded', out(f1b).slice(-300))
    await rec.cli(c1, ['branch', 'switch', 'main'], { quietFrame: true })

    const c2 = join(root, 'c2')
    r = await rec.cli(root, ['clone', '--branch', 'look-dev', srv.url('show'), c2])
    const st = await rec.cli(c2, ['status', '--branch'])
    rec.check(r.code === 0 && /look-dev/.test(st.stdout), 'clone --branch look-dev checks out look-dev', st.stdout.slice(0, 200))
    await intCheck(rec, c2, wantDev)

    const c3 = join(root, 'c3')
    r = await rec.cli(root, ['clone', '--branch', 'no-such-branch', srv.url('show'), c3])
    rec.check(r.code !== 0, 'clone --branch with a missing branch fails', `exit ${r.code} ${out(r).slice(-300)}`)
    rec.check(!PLAIN.test(out(r)), 'that failure is plain words', out(r).slice(-300))
    rec.check(!halfRepo(c3), 'a failed --branch clone leaves no half repository', existsSync(c3) ? readdirSync(c3).join(',') : 'absent')

    const full = join(root, 'not-empty')
    mkdirSync(full, { recursive: true })
    writeFileSync(join(full, 'my-notes.txt'), 'keep me\n')
    r = await rec.cli(root, ['clone', srv.url('show'), full])
    rec.check(r.code !== 0, 'clone into a non-empty folder is refused', `exit ${r.code} ${out(r).slice(-300)}`)
    rec.check(existsSync(join(full, 'my-notes.txt')) && !existsSync(join(full, '.mediagit')) && readdirSync(full).length === 1, 'the non-empty folder is untouched', readdirSync(full).join(','))

    const empty = join(root, 'empty-ok')
    mkdirSync(empty, { recursive: true })
    r = await rec.cli(root, ['clone', srv.url('show'), empty])
    rec.check(r.code === 0, 'clone into an existing empty folder works', out(r).slice(-200))

    const c4 = join(root, 'c4')
    r = await rec.cli(root, ['clone', srv.url('no-such-repo'), c4])
    rec.check(r.code !== 0 && !PLAIN.test(out(r)), 'a wrong repository name fails in plain words', `exit ${r.code} ${out(r).slice(-300)}`)
    rec.check(!halfRepo(c4), 'a wrong URL leaves no half repository', existsSync(c4) ? readdirSync(c4).join(',') : 'absent')

    r = await rec.cli(root, ['clone', srv.url('show')])
    rec.note(`clone with no directory: exit ${r.code}, made ${existsSync(join(root, 'show'))} ${out(r).slice(-200)}`)
    rec.check(r.code === 0 && existsSync(join(root, 'show', '.mediagit')), 'clone with no directory uses the repository name')

    // --- the app
    const openClone = async () => {
      await toWelcome(page)
      await page.getByRole('button', { name: 'Clone…' }).click()
      await dlg.waitFor()
    }
    const g1 = join(root, 'g1')
    await rec.step('App: Welcome > Clone, fill URL and folder', async () => {
      await openClone()
      await dlg.getByLabel('Repository URL').fill(srv.url('show'))
      await dlg.getByLabel('Save into').fill(g1)
    })
    await rec.step('Click Clone', async () => {
      await dlg.getByRole('button', { name: 'Clone', exact: true }).click()
      await nav(page).waitFor({ timeout: 60_000 })
      await page.waitForTimeout(1000)
    })
    rec.check((await page.getByTestId('repo-switcher').innerText()).includes('g1'), 'the app opens the clone when it finishes')
    await intCheck(rec, g1, want)

    const g2 = join(root, 'g2')
    await rec.step('App: Clone with More options > Branch to check out = look-dev', async () => {
      await openClone()
      await dlg.getByLabel('Repository URL').fill(srv.url('show'))
      await dlg.getByLabel('Save into').fill(g2)
      await dlg.getByText('More options').click()
      await dlg.getByLabel('Branch to check out').fill('look-dev')
      await rec.shot('clone dialog with a branch')
      await dlg.getByRole('button', { name: 'Clone', exact: true }).click()
      await nav(page).waitFor({ timeout: 60_000 })
      await page.waitForTimeout(1000)
    })
    rec.check((await page.getByTestId('branch-indicator').innerText()).includes('look-dev'), 'the app clone lands on look-dev', await page.getByTestId('branch-indicator').innerText())
    await intCheck(rec, g2, wantDev)

    const g3 = join(root, 'g3')
    await rec.step('App: Clone a repository name that does not exist', async () => {
      await openClone()
      await dlg.getByLabel('Repository URL').fill(srv.url('no-such-repo'))
      await dlg.getByLabel('Save into').fill(g3)
      await dlg.getByRole('button', { name: 'Clone', exact: true }).click()
      await page.waitForTimeout(6000)
    })
    let msg = (await dlg.isVisible()) ? (await dlg.innerText()).replace(/\n+/g, ' / ') : `dialog closed; ${JSON.stringify(await visibleErrors(page))}`
    rec.note(`app wrong-URL message: ${msg.slice(0, 400)}`)
    rec.check(await dlg.isVisible(), 'the clone dialog stays open after a failure, with the form filled')
    rec.check(/not found|no such|does not exist|does not have|could not|cannot/i.test(msg) && !PLAIN.test(msg), 'the app explains the wrong URL in plain words', msg.slice(0, 300))
    rec.check(!halfRepo(g3), 'the failed app clone leaves no half repository', existsSync(g3) ? readdirSync(g3).join(',') : 'absent')

    await rec.step('App: Clone into a folder that already has files', async () => {
      await dlg.getByLabel('Repository URL').fill(srv.url('show'))
      await dlg.getByLabel('Save into').fill(full)
      await dlg.getByRole('button', { name: 'Clone', exact: true }).click()
      await page.waitForTimeout(5000)
    })
    msg = (await dlg.isVisible()) ? (await dlg.innerText()).replace(/\n+/g, ' / ') : `dialog closed; url ${page.url()}`
    rec.note(`app non-empty-folder message: ${msg.slice(0, 400)}`)
    rec.check(/not empty|already|exists/i.test(msg) && !PLAIN.test(msg), 'the app explains the non-empty folder in plain words', msg.slice(0, 300))
    rec.check(readdirSync(full).length === 1, 'the non-empty folder is still untouched', readdirSync(full).join(','))

    srv.stop()
    stopped = true
    const c5 = join(root, 'c5')
    r = await rec.cli(root, ['clone', srv.url('show'), c5])
    rec.check(r.code !== 0 && !PLAIN.test(out(r).split('\n').filter((l) => !/^\s*Details:/.test(l)).join('\n')), 'CLI: server down fails in plain words', `exit ${r.code} in ${r.ms} ms: ${out(r).slice(-300)}`)
    rec.check(r.ms < 60_000, 'CLI: server down fails within 60 s', `${r.ms} ms`)
    rec.check(!halfRepo(c5), 'CLI: server down leaves no half repository', existsSync(c5) ? readdirSync(c5).join(',') : 'absent')

    const g4 = join(root, 'g4')
    const t0 = Date.now()
    await rec.step('App: Clone while the server is down', async () => {
      if (!(await dlg.isVisible())) await openClone()
      await dlg.getByLabel('Repository URL').fill(srv.url('show'))
      await dlg.getByLabel('Save into').fill(g4)
      await dlg.getByRole('button', { name: 'Clone', exact: true }).click()
      await dlg.getByRole('button', { name: 'Clone', exact: true }).waitFor({ timeout: 90_000 }).catch(() => {})
      await page.waitForTimeout(800)
    })
    msg = (await dlg.isVisible()) ? (await dlg.innerText()).replace(/\n+/g, ' / ') : 'dialog closed'
    rec.note(`app server-down message after ${Date.now() - t0} ms: ${msg.slice(0, 400)}`)
    rec.check(/connect|reach|server|offline|network/i.test(msg) && !PLAIN.test(msg), 'the app explains the unreachable server in plain words', msg.slice(0, 300))
    rec.check(!halfRepo(g4), 'app: server down leaves no half repository', existsSync(g4) ? readdirSync(g4).join(',') : 'absent')
    if (await dlg.isVisible()) await dlg.getByRole('button', { name: 'Cancel' }).click()
  } finally {
    if (!stopped) srv.stop()
  }
  await rec.finish()
  await closeTerm()
})

test('L-08 Long paths, trailing dot, reserved names, case-only rename', async () => {
  const rec = await Recorder.start('L-08', 'odd-names')
  const root = workDir('L-08')
  rmSync(root, { recursive: true, force: true })
  let repo = join(root, 'repo')
  mkdirSync(repo, { recursive: true })
  await rec.cli(repo, ['init'])
  const pdf = join(CORPUS, 'pdf', '1710.02185v1.pdf')
  const ai = join(CORPUS, '12690118_5053480.ai')

  const made: string[] = []
  const put = (rel: string, src: string) => {
    try {
      const dst = join(repo, rel)
      mkdirSync(join(dst, '..'), { recursive: true })
      cpSync(src, dst)
      made.push(rel)
    } catch (e) {
      rec.note(`could not create ${rel} on this disk: ${String(e).slice(0, 120)}`)
    }
  }
  const deep = Array.from({ length: 9 }, (_, i) => `sequence_${String(i).padStart(2, '0')}_environment_lighting_pass`).join('/')
  put(`${deep}/final_composite_master_v023_approved_by_director.pdf`, pdf)
  put('normal/plate.pdf', pdf)
  put('dots/shot.v1.', ai)
  put('reserved/CON.psd', ai)
  put('reserved/aux.ai', ai)
  put('spaces/ leading and trailing .ai', ai)
  put('Case/Hero.ai', ai)
  const onDisk = walk(repo).map((f) => relative(repo, f).replace(/\\/g, '/'))
  rec.note(`on disk (${onDisk.length}): ${JSON.stringify(onDisk.map((f) => (f.length > 60 ? `…${f.slice(-50)}` : f)))}`)
  rec.note(`longest absolute path: ${Math.max(...onDisk.map((f) => join(repo, f).length))} chars`)

  // BUG-008/009 (fixed): long paths and reserved names are stored repo-relative and come back; a name ending in a
  // dot cannot be restored on Windows, so `add` names it with the reason, exits 1 and still stages the rest.
  const dotFile = 'dots/shot.v1.'
  const hasDot = onDisk.includes(dotFile)
  let r = await rec.cli(repo, ['add', '--all'])
  rec.check(hasDot ? r.code !== 0 && /shot\.v1\./.test(out(r)) && !/panick|os error/i.test(out(r)) : r.code === 0, 'BUG-009: add --all names the trailing-dot file with a reason and exits 1 (no silent skip)', `exit ${r.code} ${out(r).slice(-400)}`)
  rec.check(!/\/\/\?\/|[A-Za-z]:[\/]/.test(r.stdout), 'BUG-008: add stages repo-relative paths, never an absolute //?/D:/ path', out(r).slice(-300))
  r = await rec.cli(repo, ['status', '--porcelain'])
  const silent = onDisk.filter((f) => !r.stdout.includes(f.split('/').pop()!))
  rec.check(silent.length === 0, 'every file on disk shows in status (no silent skip)', `missing from status: ${JSON.stringify(silent)}`)
  const untracked = r.stdout.split('\n').filter((l) => l.startsWith('??'))
  rec.check(untracked.length === (hasDot ? 1 : 0) && (!hasDot || /shot\.v1\./.test(untracked[0]!)), 'BUG-009: only the refused trailing-dot file is left untracked', untracked.join(' | ').slice(0, 300))
  rec.check(!/^A\s+\/\/\?\//m.test(r.stdout), 'BUG-008: status shows no absolute-path twin of the deep file', r.stdout.slice(0, 300))
  r = await rec.cli(repo, ['commit', '-m', 'odd names'])
  rec.check(r.code === 0, 'commit works', out(r).slice(-200))

  const stored = onDisk.filter((f) => f !== dotFile)
  const want: Record<string, string> = {}
  for (const f of stored) want[f] = await sha256(join(repo, f))
  const fsckOdd = await rec.cli(repo, ['fsck'], { quietFrame: true })
  rec.check(fsckOdd.code === 0, 'fsck passes on the odd-names repository', out(fsckOdd).slice(-200))
  await restoreCheck(rec, repo, want)
  const back = walk(repo).map((f) => relative(repo, f).replace(/\\/g, '/'))
  rec.check(stored.every((f) => back.includes(f)), 'BUG-008: the long-path file and the other odd names come back from the store with the same name', `missing: ${JSON.stringify(stored.filter((f) => !back.includes(f)))}`)

  // --- case-only rename, in its own repository (the odd-names one cannot be restored: BUG-008)
  const odd = repo
  repo = join(root, 'case-repo')
  mkdirSync(join(repo, 'Case'), { recursive: true })
  await rec.cli(repo, ['init'], { quietFrame: true })
  cpSync(ai, join(repo, 'Case', 'Hero.ai'))
  cpSync(pdf, join(repo, 'plate.pdf'))
  await rec.cli(repo, ['add', '--all'], { quietFrame: true })
  await rec.cli(repo, ['commit', '-m', 'Hero'], { quietFrame: true })
  for (const k of Object.keys(want)) delete want[k]
  want['plate.pdf'] = await sha256(join(repo, 'plate.pdf'))
  rec.touch(odd)
  renameSync(join(repo, 'Case', 'Hero.ai'), join(repo, 'Case', 'hero.ai'))
  r = await rec.cli(repo, ['status', '--porcelain'])
  rec.note(`status after a case-only rename: ${JSON.stringify(r.stdout.trim())}`)
  rec.check(/hero\.ai/.test(r.stdout), 'a case-only rename shows in status', r.stdout.slice(0, 200))
  await rec.cli(repo, ['add', '--all'])
  r = await rec.cli(repo, ['commit', '-m', 'rename Hero to hero'])
  rec.note(`commit of the case-only rename: exit ${r.code} ${out(r).slice(-200)}`)
  r = await rec.cli(repo, ['status', '--porcelain'])
  rec.check(r.stdout.trim() === '', 'the tree is clean after committing the case-only rename', r.stdout.slice(0, 200))
  want['Case/hero.ai'] = await sha256(join(repo, 'Case', 'hero.ai'))
  await restoreCheck(rec, repo, want)
  rec.check(readdirSync(join(repo, 'Case')).join() === 'hero.ai', 'after restore the file has the new case', readdirSync(join(repo, 'Case')).join())

  // --- two names that differ only by case, in one commit (made by a tool on another OS)
  await intCheck(rec, repo, want)
  await rec.finish()
  await closeTerm()
})
