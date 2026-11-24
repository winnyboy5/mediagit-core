import { execFileSync, spawn } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { CLI, CLI_ENV, CORPUS, DESKTOP, SERVER, WORK, CDP_PORT, cdpUp, killApp, launchApp, pidsOf, readState, waitFor } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { flat, go, openRepo, sampleProgress, store, toWelcome, visibleErrors } from './gui'
import { intCheck, sha256, walk } from './integrity'

// R-01 cancel mid-add, R-03 kill the app mid-add, R-05 watcher, R-06 second launch, R-07/08 themes and 1024 px, R-09 keyboard.
const alerts = async (page: Page) => JSON.stringify(await visibleErrors(page)).slice(0, 500)
const raw = (s: string) => /os error|panicked|exit code|stderr|\bErr\(|protocol|unexpected output|failed with status/i.test(s)
const put = (dir: string, file: string, body: string | { from: string }) => { mkdirSync(dirname(join(dir, file)), { recursive: true }); if (typeof body === 'string') writeFileSync(join(dir, file), body); else cpSync(join(CORPUS, body.from), join(dir, file)) }
async function open(page: Page, dir: string, route: string) { await toWelcome(page); await openRepo(page, dir); await go(page, route); await page.waitForTimeout(2500) }
const changes = (page: Page) => store<{ total: number; staged: number; unstaged: number }>(page, 'changes', 's.counts')
const leftovers = (dir: string) => walk(join(dir, '.mediagit'), () => false).filter((f) => /\.lock$|\.tmp$|index\.lock|\.partial$/i.test(f)).map((f) => f.slice(dir.length + 1))
/** A repository holding the large video files of work/big as NEW, unsaved files. */
function bigRepo(rec: Recorder, name: string): { dir: string; files: { rel: string; n: number }[] } {
  const dir = workDir(name)
  rmSync(dir, { recursive: true, force: true }); mkdirSync(dir, { recursive: true })
  execFileSync(CLI, ['init'], { cwd: dir, env: { ...process.env, ...CLI_ENV }, stdio: 'ignore' })
  const src = join(WORK, 'big')
  const files = walk(src).map((f) => ({ f, n: statSync(f).size })).sort((a, b) => b.n - a.n).slice(0, 4)
  for (const x of files) { const rel = x.f.slice(src.length + 1); mkdirSync(dirname(join(dir, rel)), { recursive: true }); cpSync(x.f, join(dir, rel)) }
  rec.note(`large files: ${files.map((x) => `${x.f.slice(src.length + 1)} ${(x.n / 1e9).toFixed(2)} GB`).join(', ')}`)
  return { dir, files: files.map((x) => ({ rel: x.f.slice(src.length + 1).replace(/\\/g, '/'), n: x.n })) }
}

test('R-01 Cancel mid-add, then retry', async () => {
  const rec = await Recorder.start('R-01', 'cancel-add', { gui: true })
  const page = rec.page!
  const { dir, files } = bigRepo(rec, 'R-01')
  const total = files.reduce((n, f) => n + f.n, 0)
  const want: Record<string, string> = {}
  for (const f of files) want[f.rel] = await sha256(join(dir, f.rel))
  await rec.step('Open Files > Changes', () => open(page, dir, 'files/changes'))
  rec.note(`changes: ${JSON.stringify(await changes(page))}`)
  for (const at of [30, 80]) {
    let seen = 0
    const t0 = Date.now()
    await rec.step(`Include all, then Cancel at about ${at}%`, async () => {
      await page.getByRole('button', { name: 'Include all', exact: true }).click()
      for (let i = 0; i < 400; i++) { await page.waitForTimeout(150); const s = await sampleProgress(page, t0); const pct = Number(s.bar); if (!Number.isNaN(pct)) seen = pct; if (seen >= at) break; if (i > 20 && !s.pill) break }
      await rec.shot(`at ${seen}%`)
      const cancel = page.getByRole('button', { name: /^Cancel/ }).first()
      rec.note(`cancel control: visible=${await cancel.isVisible().catch(() => false)} at bar=${seen} after ${Date.now() - t0} ms`)
      const t1 = Date.now()
      if (!(await cancel.isVisible().catch(() => false))) await page.locator('[data-activity-pill]').click().catch(() => {})
      await page.getByRole('button', { name: /^Cancel/ }).first().click({ timeout: 5000 }).catch(() => rec.note('no Cancel button could be clicked'))
      for (let i = 0; i < 100 && pidsOf('mediagit.exe').length; i++) await page.waitForTimeout(100)
      rec.note(`after Cancel: the CLI process ended within ${Date.now() - t1} ms; still running: ${pidsOf('mediagit.exe').length}`)
      await page.waitForTimeout(2500)
    })
    const c = await changes(page)
    const al = await alerts(page)
    rec.note(`after cancel at ${seen}%: ${JSON.stringify(c)} | ${al} | leftovers ${leftovers(dir).join(', ') || 'none'}`)
    rec.check(seen >= at - 15 && seen < 100, `BUG-038: staging can be cancelled; the add was really cancelled mid-way (bar at ${seen}%)`)
    rec.check(pidsOf('mediagit.exe').length === 0, 'no CLI process is left running')
    rec.check(!raw(al), 'the cancel is reported in plain words', al)
    rec.check(c.total === files.length, 'all files are still listed as changes', JSON.stringify(c))
    const st = await rec.cli(dir, ['status', '--machine'], { quietFrame: true })
    rec.check(st.code === 0 && st.stdout.split('\n').filter((l) => l.trim()).length === 1, 'the repository answers `status` right after the cancel', flat(st.stdout + st.stderr).slice(0, 200))
    await page.getByRole('button', { name: 'Dismiss' }).click().catch(() => {})
  }
  const t2 = Date.now()
  await rec.step('Include all again and let it finish', async () => { await page.getByRole('button', { name: 'Include all', exact: true }).click(); await page.waitForTimeout(1500); for (let i = 0; i < 2400 && (pidsOf('mediagit.exe').length || (await changes(page)).unstaged > 0); i++) await page.waitForTimeout(250); await page.waitForTimeout(2000) })
  rec.note(`retry took ${Date.now() - t2} ms for ${(total / 1e9).toFixed(2)} GB: ${(total / 1e6 / ((Date.now() - t2) / 1000)).toFixed(0)} MB/s | ${JSON.stringify(await changes(page))}`)
  rec.check((await changes(page)).staged === files.length, 'the retry stages every file', JSON.stringify(await changes(page)))
  const cm = await rec.cli(dir, ['commit', '-m', 'after two cancels'])
  rec.check(cm.code === 0, 'the version saves', flat(cm.stdout + cm.stderr).slice(-200))
  for (const f of files) rmSync(join(dir, f.rel))
  const back = await rec.cli(dir, ['reset', '--hard', 'HEAD'])
  let same = back.code === 0
  for (const f of files) same = same && existsSync(join(dir, f.rel)) && (await sha256(join(dir, f.rel))) === want[f.rel]
  rec.check(same, 'every large file comes back from the store byte for byte after the cancelled attempts', `restore exit ${back.code}, peak ${back.peakMb} MB, ${back.ms} ms`)
  rec.note(`store size ${(walk(join(dir, '.mediagit'), () => false).reduce((n, f) => n + statSync(f).size, 0) / 1e9).toFixed(2)} GB for ${(total / 1e9).toFixed(2)} GB of files (cancelled attempts must not leave duplicate data)`)
  await intCheck(rec, dir)
  await rec.finish()
  await closeTerm()
})

test('R-03 Kill the app mid-add, reopen', async () => {
  const userMg = pidsOf('mediagit.exe') // not ours (a user's CLI, another job): never counted, never killed
  const rec = await Recorder.start('R-03', 'kill-app', { gui: true })
  let page = rec.page!
  const { dir, files } = bigRepo(rec, 'R-03')
  const want: Record<string, string> = {}
  for (const f of files) want[f.rel] = await sha256(join(dir, f.rel))
  await rec.step('Open Files > Changes', () => open(page, dir, 'files/changes'))
  const t0 = Date.now()
  let seen = 0
  await rec.step('Include all, wait for about 40%, then kill the app process', async () => {
    await page.getByRole('button', { name: 'Include all', exact: true }).click()
    for (let i = 0; i < 400; i++) { await page.waitForTimeout(150); const s = await sampleProgress(page, t0); const pct = Number(s.bar); if (!Number.isNaN(pct)) seen = pct; if (seen >= 40) break }
    await rec.shot(`at ${seen}%, about to kill`)
  })
  const appPid = readState().appPid
  const cliBefore = pidsOf('mediagit.exe')
  killApp()
  await new Promise((r) => setTimeout(r, 4000))
  const orphans = pidsOf('mediagit.exe').filter((p) => !userMg.includes(p))
  rec.note(`killed the app (pid ${appPid}) at ${seen}%. CLI processes before the kill: ${cliBefore.length}; 4 s after: ${orphans.length} (${orphans.join(',')})`)
  rec.check(orphans.length === 0, 'no orphan mediagit.exe keeps running after the app is killed')
  for (const p of orphans) { try { process.kill(p) } catch { /* gone */ } }
  rec.note(`leftovers in .mediagit after the kill: ${leftovers(dir).join(', ') || 'none'}`)
  const st = await rec.cli(dir, ['status', '--machine'])
  rec.check(st.code === 0, 'the CLI can read the repository right after the kill (no stale lock)', flat(st.stdout + st.stderr).slice(0, 250))
  await launchApp()
  page = await rec.attach()
  await page.waitForTimeout(4000)
  await rec.shot('app reopened')
  const where = `${await page.evaluate(() => window.location.hash)} | ${flat(await page.locator('#app').innerText()).slice(0, 200)}`
  rec.note(`after relaunch: ${where} | alerts ${await alerts(page)}`)
  // parked platform limitation: WebView2 does not flush localStorage on a hard kill, so the last repo is not remembered; reopen it explicitly
  rec.note(`reopen-last-repo after a hard kill is parked: ${where.includes('R-03') ? 'it did reopen' : 'it did not reopen'}`)
  await rec.step('Reopen the R-03 repository explicitly', async () => { await openRepo(page, dir); await page.waitForTimeout(2000) })
  rec.check(!raw(await alerts(page)), 'no raw error on reopen', await alerts(page))
  await rec.step('Files > Changes after the relaunch', async () => { await go(page, 'files/changes'); await page.waitForTimeout(3000) })
  rec.note(`changes after relaunch: ${JSON.stringify(await changes(page))}`)
  await rec.step('Include all again and let it finish', async () => { await page.getByRole('button', { name: 'Include all', exact: true }).click(); await page.waitForTimeout(1500); for (let i = 0; i < 2400 && (pidsOf('mediagit.exe').length || (await changes(page)).unstaged > 0); i++) await page.waitForTimeout(250); await page.waitForTimeout(2000) })
  rec.check((await changes(page)).staged === files.length, 'the retry stages every file', `${JSON.stringify(await changes(page))} | ${await alerts(page)}`)
  const cm = await rec.cli(dir, ['commit', '-m', 'after a killed add'])
  rec.check(cm.code === 0, 'the version saves', flat(cm.stdout + cm.stderr).slice(-200))
  for (const f of files) rmSync(join(dir, f.rel))
  const back = await rec.cli(dir, ['reset', '--hard', 'HEAD'])
  let same = back.code === 0
  for (const f of files) same = same && existsSync(join(dir, f.rel)) && (await sha256(join(dir, f.rel))) === want[f.rel]
  rec.check(same, 'every large file comes back from the store byte for byte', `restore exit ${back.code}`)
  await intCheck(rec, dir)
  await rec.finish()
  await closeTerm()
  rmSync(dir, { recursive: true, force: true }); rmSync(workDir('R-01'), { recursive: true, force: true })
})

test('R-05 + R-06 Watcher within 3 s, and a second app launch', async () => {
  const rec = await Recorder.start('R-05', 'watcher', { gui: true })
  const page = rec.page!
  const dir = workDir('R-05')
  rmSync(dir, { recursive: true, force: true }); mkdirSync(dir, { recursive: true })
  const cli = (...a: string[]) => rec.cli(dir, a, { quietFrame: true })
  await cli('init'); put(dir, 'art/banner.ai', { from: 'creative-pairs/ai/banner_v1.ai' }); put(dir, 'art/photo.jpeg', { from: 'freepik__talk__71826.jpeg' }); put(dir, 'notes.txt', 'base\n'); await cli('add', '--all'); await cli('commit', '-m', 'base')
  await rec.step('Open Files > Changes on a clean repository', () => open(page, dir, 'files/changes'))
  rec.check((await changes(page)).total === 0, 'clean: no changes', JSON.stringify(await changes(page)))
  /** Do something on disk, then time how long the Changes count takes to reach `want`. No refocus: the watcher must do it. */
  const timed = async (what: string, act: () => void, want: number, budget = 3000) => {
    const t0 = Date.now(); act()
    let ms = -1
    for (let i = 0; i < 150; i++) { if ((await changes(page)).total === want) { ms = Date.now() - t0; break } await page.waitForTimeout(100) }
    await rec.shot(what)
    rec.note(`${what}: ${ms < 0 ? `NOT seen within 15 s (count ${(await changes(page)).total}, wanted ${want})` : `${ms} ms`}`)
    rec.check(ms >= 0 && ms <= budget, `${what}: the list updates within ${budget / 1000} s`, `${ms} ms`)
    return ms
  }
  await timed('create a new file outside the app', () => writeFileSync(join(dir, 'new-shot.txt'), 'shot\n'), 1)
  await timed('edit a tracked file', () => writeFileSync(join(dir, 'notes.txt'), 'edited outside\n'), 2)
  await timed('replace a tracked picture (real media)', () => cpSync(join(CORPUS, 'freepik__talk__72772.jpeg'), join(dir, 'art/photo.jpeg')), 3)
  await timed('DCC-style save: write a temp file, rename it over the original', () => { cpSync(join(CORPUS, 'creative-pairs/ai/banner_v2.ai'), join(dir, 'art/banner.ai.tmp')); renameSync(join(dir, 'art/banner.ai.tmp'), join(dir, 'art/banner.ai')) }, 4)
  await timed('delete the new file again', () => rmSync(join(dir, 'new-shot.txt')), 3)
  await timed('rename a tracked file', () => renameSync(join(dir, 'notes.txt'), join(dir, 'notes-renamed.txt')), 4)
  rec.note(`rows now: ${flat(await page.locator('#app').innerText()).slice(-350)}`)
  await timed('put everything back (no changes left)', () => { renameSync(join(dir, 'notes-renamed.txt'), join(dir, 'notes.txt')); writeFileSync(join(dir, 'notes.txt'), 'base\n'); cpSync(join(CORPUS, 'freepik__talk__71826.jpeg'), join(dir, 'art/photo.jpeg')); cpSync(join(CORPUS, 'creative-pairs/ai/banner_v1.ai'), join(dir, 'art/banner.ai')) }, 0, 5000)
  const cpu0 = pidsOf('mediagit.exe').length
  await timed('save 500 small files at once', () => { mkdirSync(join(dir, 'frames'), { recursive: true }); for (let i = 0; i < 500; i++) writeFileSync(join(dir, 'frames', `f${String(i).padStart(4, '0')}.txt`), `frame ${i}\n`) }, 500, 6000)
  let spawned = 0
  for (let i = 0; i < 50; i++) { spawned = Math.max(spawned, pidsOf('mediagit.exe').length); await page.waitForTimeout(100) }
  rec.note(`refresh storm check: most CLI processes alive at once in the 5 s after the 500-file burst: ${spawned} (before: ${cpu0})`)
  rec.check(spawned <= 3, 'no refresh storm: at most a few CLI calls at once after a 500-file burst', String(spawned))
  await page.waitForTimeout(4000)
  const idle = (() => { try { return execFileSync('powershell', ['-NoProfile', '-Command', "$a=(Get-Process mediagit-desktop,msedgewebview2 -ErrorAction SilentlyContinue | Measure-Object CPU -Sum).Sum; Start-Sleep 5; $b=(Get-Process mediagit-desktop,msedgewebview2 -ErrorAction SilentlyContinue | Measure-Object CPU -Sum).Sum; [math]::Round(($b-$a)/5*100/[Environment]::ProcessorCount,2)"], { encoding: 'utf8' }).trim() } catch { return '?' } })()
  rec.note(`idle CPU of the app and its WebView2 processes over 5 s, 500 changes listed: ${idle}% of the machine`)
  rec.check(Number(idle) < 1, 'idle CPU is under 1%', `${idle}%`)
  rmSync(join(dir, 'frames'), { recursive: true, force: true })

  const before = pidsOf('mediagit-desktop.exe')
  const env = { ...process.env, ...CLI_ENV, MEDIAGIT_DESKTOP_CLI: CLI, MEDIAGIT_DESKTOP_SERVER: SERVER, MEDIAGIT_DESKTOP_SERVER_DIR: join(WORK, 'app-server'), MEDIAGIT_DESKTOP_STATE_DIR: join(WORK, 'app-state'), WEBVIEW2_USER_DATA_FOLDER: join(WORK, 'webview2'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}` }
  const second = spawn(DESKTOP, [], { env, stdio: 'ignore', detached: true }); second.unref()
  await page.waitForTimeout(6000)
  const after = pidsOf('mediagit-desktop.exe')
  await rec.shot('after a second launch')
  rec.note(`R-06 second launch: app processes before ${before.length}, 6 s after the second start ${after.length} (${after.join(',')})`)
  rec.check(after.length === before.length, 'a second launch does not leave a second app running (single instance)', `${before.length} to ${after.length}`)
  rec.check(await cdpUp().catch(() => false), 'the first window is still alive and answering')
  for (const p of after.filter((x) => !before.includes(x) && x === second.pid)) { try { process.kill(p) } catch { /* gone */ } }
  rec.check((await page.evaluate(() => window.location.hash)).length > 0 && !raw(await alerts(page)), 'the first window kept its page and shows no error', await alerts(page))
  await waitFor('CDP still up', 10_000, cdpUp)
  await intCheck(rec, dir)
  await rec.finish()
  await closeTerm()
})

test('R-07 + R-08 + R-09 Dark theme, 1024 px wide, keyboard only', async () => {
  const rec = await Recorder.start('R-07', 'theme-size-keys', { gui: true })
  const page = rec.page!
  const dir = workDir('R-05')
  if (!existsSync(join(dir, '.mediagit'))) throw new Error('needs the repository made by R-05')
  writeFileSync(join(dir, 'notes.txt'), 'changed for the screenshots\n')
  const ROUTES = ['files/all', 'files/changes', 'history/versions', 'history/compare', 'history/tags', 'branches/branches', 'branches/merge', 'branches/sync', 'branches/stash', 'team/locks', 'team/remote', 'team/server', 'settings/general', 'settings/storage', 'settings/maintenance', 'settings/security', 'settings/advanced']
  await rec.step('Open the repository', () => open(page, dir, 'files/all'))
  const cdp = await page.context().newCDPSession(page)
  const setTheme = (t: 'dark' | 'light') => page.evaluate((v) => { document.documentElement.setAttribute('data-theme', v); try { localStorage.setItem('mg.theme', v) } catch { /* ignore */ } }, t)
  /** Text whose colour is too close to its background, plus panels that stayed light in dark mode. */
  const audit = (dark: boolean) => page.evaluate((isDark) => {
    const lum = (c: string) => { const m = c.match(/[\d.]+/g)!.map(Number); const f = (v: number) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4 }; return { l: 0.2126 * f(m[0]!) + 0.7152 * f(m[1]!) + 0.0722 * f(m[2]!), a: m[3] ?? 1 } }
    const bgOf = (e: Element): string => { let n: Element | null = e; while (n) { const b = getComputedStyle(n).backgroundColor; if (lum(b).a > 0.5) return b; n = n.parentElement } return 'rgb(255,255,255)' }
    const low: string[] = []; const light: string[] = []
    for (const e of document.querySelectorAll('#app *')) {
      const el = e as HTMLElement; const r = el.getBoundingClientRect()
      if (r.width < 2 || r.height < 2 || r.bottom < 0 || r.top > innerHeight) continue
      const own = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent!.trim().length > 1)
      const cs = getComputedStyle(el)
      if (own && cs.visibility !== 'hidden' && Number(cs.opacity) > 0.3) { const a = lum(cs.color).l; const b = lum(bgOf(el)).l; const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05); if (ratio < 3) low.push(`${el.innerText.trim().slice(0, 24)} (${ratio.toFixed(1)}:1)`) }
      if (isDark && r.width > 120 && r.height > 40) { const b = lum(cs.backgroundColor); if (b.a > 0.9 && b.l > 0.7) light.push(`${el.tagName.toLowerCase()}.${(el.className || '').toString().split(' ')[0]} ${Math.round(r.width)}x${Math.round(r.height)}`) }
    }
    const sw = document.documentElement.scrollWidth - innerWidth
    const clipped = [...document.querySelectorAll('#app button')].filter((b) => { const r = b.getBoundingClientRect(); return r.width > 0 && (r.right > innerWidth + 1 || r.left < -1) }).map((b) => (b as HTMLElement).innerText.trim().slice(0, 20) || b.getAttribute('aria-label') || '?')
    return { low: [...new Set(low)].slice(0, 6), light: [...new Set(light)].slice(0, 4), overflow: sw, clipped: [...new Set(clipped)].slice(0, 6) }
  }, dark)
  let lowAll = 0; let lightAll = 0; let overflowAll = 0; let clippedAll = 0
  await setTheme('dark')
  for (const r of ROUTES) {
    await go(page, r); await page.waitForTimeout(1400)
    await rec.shot(`dark ${r}`)
    const a = await audit(true)
    lowAll += a.low.length; lightAll += a.light.length
    if (a.low.length || a.light.length) rec.note(`dark ${r}: low contrast ${JSON.stringify(a.low)} | light panels ${JSON.stringify(a.light)}`)
  }
  rec.check(lightAll === 0, 'dark theme: no panel stays light on any of the 17 screens', `${lightAll} found`)
  rec.check(lowAll === 0, 'dark theme: no text under 3:1 contrast on any screen', `${lowAll} found`)
  await setTheme('light')
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1024, height: 800, deviceScaleFactor: 1, mobile: false })
  await page.waitForTimeout(800)
  for (const r of ROUTES) {
    await go(page, r); await page.waitForTimeout(1200)
    await rec.shot(`1024x800 ${r}`)
    const a = await audit(false)
    if (a.overflow > 1) overflowAll++
    clippedAll += a.clipped.length
    if (a.overflow > 1 || a.clipped.length || a.low.length) rec.note(`1024 px ${r}: horizontal overflow ${a.overflow}px | clipped buttons ${JSON.stringify(a.clipped)} | low contrast (light) ${JSON.stringify(a.low)}`)
  }
  rec.check(overflowAll === 0, '1024 px: no horizontal scroll on any screen', `${overflowAll} screens overflow`)
  rec.check(clippedAll === 0, '1024 px: no button is cut off at the window edge', `${clippedAll} found`)
  await cdp.send('Emulation.clearDeviceMetricsOverride')
  await page.waitForTimeout(800)

  const hash = () => page.evaluate(() => window.location.hash)
  const focus = () => page.evaluate(() => { const e = document.activeElement as HTMLElement | null; if (!e || e === document.body) return '(body)'; const o = getComputedStyle(e); return `${e.tagName.toLowerCase()} "${(e.getAttribute('aria-label') || e.innerText || (e as HTMLInputElement).placeholder || '').trim().slice(0, 28)}" ring=${o.outlineStyle !== 'none' && parseFloat(o.outlineWidth) > 0 ? 'outline' : o.boxShadow !== 'none' ? 'shadow' : 'NONE'}` })
  await go(page, 'files/all'); await page.waitForTimeout(800); await page.locator('body').click({ position: { x: 600, y: 5 } }).catch(() => {})
  for (const [key, want] of [['Control+2', 'history'], ['Control+3', 'branches'], ['Control+4', 'team'], ['Control+1', 'files'], ['Control+,', 'settings']] as const) {
    await rec.step(`Keyboard: ${key}`, async () => { await page.keyboard.press(key); await page.waitForTimeout(700) })
    rec.check((await hash()).includes(`/${want}`), `${key} opens ${want}`, await hash())
  }
  await page.keyboard.press('Control+1'); await page.waitForTimeout(600)
  for (const [key, want] of [['Alt+2', 'files/changes'], ['Alt+1', 'files/all']] as const) {
    await rec.step(`Keyboard: ${key}`, async () => { await page.keyboard.press(key); await page.waitForTimeout(700) })
    rec.check((await hash()).includes(want), `${key} opens the section ${want}`, await hash())
  }
  await rec.step('Keyboard: Ctrl+F in All files', async () => { await page.keyboard.press('Control+f'); await page.waitForTimeout(500) })
  rec.note(`focus after Ctrl+F: ${await focus()}`)
  rec.check(/input/.test(await focus()), 'Ctrl+F puts the cursor in the search field', await focus())
  await page.keyboard.press('Escape')
  await rec.step('Keyboard: Alt+2 (Changes), then Tab 25 times', async () => { await page.keyboard.press('Alt+2'); await page.waitForTimeout(1200) })
  const stops: string[] = []
  for (let i = 0; i < 25; i++) { await page.keyboard.press('Tab'); await page.waitForTimeout(60); stops.push(await focus()) }
  await rec.shot('after 25 tabs')
  rec.note(`tab order on Changes: ${stops.join(' > ')}`)
  rec.check(!stops.some((s) => s.includes('ring=NONE')), 'every Tab stop shows a focus ring', stops.filter((s) => s.includes('ring=NONE')).slice(0, 5).join(' | '))
  rec.check(new Set(stops).size >= 10 && stops.filter((s) => s === '(body)').length <= 1, 'Tab walks through real controls and never falls to the page body (one wrap after the last control is normal)', `${new Set(stops).size} distinct stops; (body) x${stops.filter((s) => s === '(body)').length}`)
  await rec.step('Keyboard only: save a version (Space to include, type a message, Ctrl+Enter)', async () => {
    await page.keyboard.press('Alt+2'); await page.waitForTimeout(800)
    const box = page.getByRole('button', { name: 'Include all' }); await box.focus(); await page.keyboard.press('Enter'); await page.waitForTimeout(2500)
    const msg = page.getByPlaceholder(/What changed|Describe|message/i).first(); await msg.focus(); await page.keyboard.type('saved with the keyboard only'); await page.keyboard.press('Control+Enter'); await page.waitForTimeout(4000)
  })
  const lg = await rec.cli(dir, ['log', '--machine', '-n', '1'], { quietFrame: true })
  rec.check(/saved with the keyboard only/.test(lg.stdout), 'the daily loop works with no mouse', `${lg.stdout.slice(0, 160)} | ${await alerts(page)}`)
  await rec.step('Dialog focus: open New branch with the keyboard, Esc, focus returns', async () => { await page.keyboard.press('Control+3'); await page.waitForTimeout(900); const b = page.getByRole('button', { name: 'New branch' }); await b.focus(); await page.keyboard.press('Enter'); await page.waitForTimeout(600); rec.note(`focus inside the dialog: ${await focus()}`); await page.keyboard.press('Escape'); await page.waitForTimeout(500) })
  rec.check(/New branch/.test(await focus()), 'focus returns to the button that opened the dialog', await focus())
  await intCheck(rec, dir)
  await rec.finish()
  await closeTerm()
})
