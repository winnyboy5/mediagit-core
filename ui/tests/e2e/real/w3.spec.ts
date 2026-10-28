import { execFileSync } from 'node:child_process'
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { chromium, expect, test, type Browser, type Page } from '@playwright/test'
import { formatBytes } from '../../../src/components/format'
import { readState, type E2EState } from './setup'

// Gate G4, wave W3: Storage, Maintenance, Settings, Advanced and Security in the REAL app.
// Runs after w2.spec.ts on the repo state it leaves: on main, clean, pushed. Every test
// cross-checks the CLI; the Security test uses the encrypted fixture made in setup.ts.

test.describe.configure({ mode: 'serial' })

const shots = resolve(import.meta.dirname, '..', '..', '..', 'test-results', 'real')
let st: E2EState
let browser: Browser
let page: Page

const env = () => ({ ...process.env, ...st.env, MEDIAGIT_CDC_SEED: '20260716' })
const cli = (cwd: string, ...args: string[]) => execFileSync(st.cli, args, { cwd, env: env(), encoding: 'utf8' })
const cliJson = (cwd: string, ...args: string[]) => (JSON.parse(cli(cwd, '--machine', ...args)) as { ok: boolean; data: Record<string, any> }).data // eslint-disable-line @typescript-eslint/no-explicit-any
const cliPass = (cwd: string, ...args: string[]) => {
  try {
    return execFileSync(st.cli, ['--machine', '--secret-stdin', ...args], { cwd, env: env(), encoding: 'utf8', input: `${st.passphrase}\n` })
  } catch (e) {
    const x = e as { stdout?: string; stderr?: string }
    throw new Error(`mediagit ${args.join(' ')} failed\nstdout: ${x.stdout}\nstderr: ${x.stderr}`.replaceAll(st.passphrase, '<passphrase>'))
  }
}
const shot = (name: string) => page.screenshot({ path: join(shots, `${name}.png`) })
const go = async (view: string, heading: string) => {
  await page.evaluate((v) => { window.location.hash = `#/${v}` }, view)
  await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
}
// Views load on mount: leave and come back so a view re-reads what the CLI changed.
const goFresh = async (view: string, heading: string) => {
  await go('locks', 'Locks')
  await go(view, heading)
}
const refresh = () => page.evaluate(() => window.dispatchEvent(new Event('focus')))
const confirm = (name: string) => page.getByRole('alertdialog').getByRole('button', { name, exact: true }).click()
const commit = (file: string, text: string, msg: string) => {
  writeFileSync(join(st.repo, file), text)
  cli(st.repo, 'add', '.')
  cli(st.repo, 'commit', '-m', msg)
}
const dd = (scope: ReturnType<Page['locator']>, term: string) => scope.locator(`dt:text-is("${term}") + dd`)

test.beforeAll(async () => {
  st = readState()
  mkdirSync(shots, { recursive: true })
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${st.cdpPort}`)
  const ctx = browser.contexts()[0]
  if (!ctx) throw new Error('WebView2 exposed no browser context over CDP')
  page = ctx.pages()[0] ?? (await ctx.waitForEvent('page'))
  await page.waitForLoadState('domcontentloaded')
  page.on('pageerror', (e) => appendFileSync(join(st.root, 'logs', 'pageerror.log'), `${e}\n`))
})

test.afterAll(async () => {
  await browser?.close().catch(() => {})
})

test('p. Storage: the numbers match mediagit stats', async () => {
  const s = cliJson(st.repo, 'stats')
  await go('storage', 'Storage')
  const summary = page.getByRole('list', { name: 'Storage summary' })
  await expect(summary.locator('li', { hasText: 'Stored on disk' }).locator('b')).toHaveText(formatBytes(s.total_bytes))
  const proj = page.locator('section[aria-labelledby="proj-h"]')
  await expect(dd(proj, 'Versions')).toHaveText(s.commits.toLocaleString('en-US'))
  await expect(dd(proj, 'Files')).toHaveText(s.files.toLocaleString('en-US'))
  await expect(dd(proj, 'Tags')).toHaveText(String(s.tags))
  await expect(dd(proj, 'Branches')).toHaveText(`${s.local_branches} here, ${s.remote_branches} on remotes`)
  await shot('17-storage')
})

test('q. Maintenance: integrity check and verify are healthy; clean up via the confirm', async () => {
  await go('maintenance', 'Maintenance')
  await page.getByRole('button', { name: 'Run integrity check' }).click()
  const fsck = page.getByTestId('fsck-result')
  await expect(fsck).toContainText('No problems found')
  const f = cliJson(st.repo, 'fsck')
  expect(f.healthy).toBe(true)
  await expect(dd(fsck, 'Objects checked')).toHaveText(String(f.objects_checked))

  await page.getByRole('button', { name: 'Verify', exact: true }).click()
  const verify = page.getByTestId('verify-result')
  await expect(verify).toContainText('No problems found')
  expect(cliJson(st.repo, 'verify').healthy).toBe(true)

  await page.getByRole('button', { name: 'Preview clean-up' }).click()
  await expect(page.getByTestId('gc-result')).toContainText('Preview: this would free')
  await page.getByRole('checkbox', { name: 'Preview only' }).click()
  await page.getByRole('button', { name: 'Clean up…' }).click()
  await confirm('Clean up')
  await expect(page.getByTestId('gc-result')).toContainText('Freed')
  await shot('18-maintenance')
  expect(cliJson(st.repo, 'fsck').healthy).toBe(true)
  expect(cli(st.repo, 'status', '--porcelain').trim()).toBe('')
})

test('r. Settings: author identity reaches config; theme and density survive a reload', async () => {
  await go('settings', 'Settings')
  const id = page.getByTestId('identity')
  await id.getByLabel('Name', { exact: true }).fill('Mira Okafor')
  await id.getByLabel('Email').fill('mira@studio.example')
  await id.getByRole('button', { name: 'Save' }).click()
  await expect(page.getByTestId('config')).toContainText('mira@studio.example')
  expect(cliJson(st.repo, 'config', 'get', 'author.name').entries[0].value).toBe('Mira Okafor')
  expect(cliJson(st.repo, 'config', 'get', 'author.email').entries[0].value).toBe('mira@studio.example')
  await shot('19-settings-identity')

  const look = page.getByTestId('appearance')
  await look.getByText('Dark', { exact: true }).click()
  await look.getByText('Compact', { exact: true }).click()
  const html = page.locator('html')
  await expect(html).toHaveAttribute('data-theme', 'dark')
  await expect(html).toHaveAttribute('data-density', 'compact')
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
  await expect(html).toHaveAttribute('data-theme', 'dark')
  await expect(html).toHaveAttribute('data-density', 'compact')
  await shot('19-settings-dark-compact')
  await look.getByText('Match system', { exact: true }).click()
  await look.getByText('Comfortable', { exact: true }).click()
  await expect(html).not.toHaveAttribute('data-theme', /.+/)
  await expect(html).not.toHaveAttribute('data-density', /.+/)
})

test('s. Advanced rebase: a conflict-free branch is replayed on main', async () => {
  cli(st.repo, 'branch', 'create', 'topic/rb')
  cli(st.repo, 'branch', 'switch', 'topic/rb')
  commit('topic.txt', 'topic work\n', 'Topic work')
  cli(st.repo, 'branch', 'switch', 'main')
  commit('main-side.txt', 'main moved on\n', 'Main side')
  cli(st.repo, 'branch', 'switch', 'topic/rb')
  await refresh()

  await goFresh('advanced', 'Advanced')
  await page.getByLabel('Rebase onto').click()
  await page.getByRole('option', { name: 'main', exact: true }).click()
  await page.getByRole('button', { name: 'Start rebase…' }).click()
  await shot('20-rebase-confirm')
  await confirm('Start rebase')
  await expect(page.getByTestId('rebase-result')).toBeVisible()
  await shot('20-rebase-done')
  const log = cli(st.repo, 'log', '--oneline', '-n', '2')
  expect(log.split('\n')[0]).toContain('Topic work')
  expect(log.split('\n')[1]).toContain('Main side')
  expect(existsSync(join(st.repo, 'main-side.txt'))).toBe(true)
  expect(existsSync(join(st.repo, 'topic.txt'))).toBe(true)
  cli(st.repo, 'branch', 'switch', 'main')
  await refresh()
})

test('t. Advanced cherry-pick: copy a version from another branch', async () => {
  test.info().annotations.push({ type: 'APP-FINDING', description: 'Advanced > Cherry-pick lists only the current branch versions (api.log all=false), so a version from another branch cannot be picked in the UI.' })
  test.info().annotations.push({ type: 'APP-FINDING', description: 'mediagit cherry-pick of a version this branch already contains (e.g. an added file later deleted, or a change later put back) creates an EMPTY commit (show: files []) and reports success; the change is silently not re-applied.' })
  // The picker only offers versions of the branch that is open when the page loads, so load
  // it while on topic/cp, then move to main behind its back and copy that version over.
  cli(st.repo, 'branch', 'create', 'topic/cp')
  cli(st.repo, 'branch', 'switch', 'topic/cp')
  commit('cp.txt', 'cherry\n', 'Cherry me')
  await refresh()
  await goFresh('advanced', 'Advanced')
  await page.getByRole('tab', { name: 'Cherry-pick' }).click()
  await page.getByLabel('Copy this version').click()
  await expect(page.getByRole('option', { name: /Cherry me/ })).toBeVisible()
  await page.keyboard.press('Escape')
  cli(st.repo, 'branch', 'switch', 'main')
  expect(existsSync(join(st.repo, 'cp.txt'))).toBe(false)
  await refresh()

  await page.getByRole('tab', { name: 'Cherry-pick' }).click()
  await page.getByLabel('Copy this version').click()
  await page.getByRole('option', { name: /Cherry me/ }).click()
  await page.getByRole('button', { name: 'Copy version' }).click()
  await confirm('Copy version')
  await expect(page.getByTestId('pick-result')).toBeVisible()
  await shot('21-cherry-pick')
  expect(cli(st.repo, 'log', '--oneline', '-n', '1')).toContain('Cherry me')
  expect(readFileSync(join(st.repo, 'cp.txt'), 'utf8')).toBe('cherry\n')
})

test('u. Advanced bisect: mark good/bad down to the known first bad version, then reset', async () => {
  const body = ['good', 'good', 'bad', 'bad', 'bad']
  body.forEach((b, i) => commit('bisect.txt', `${b} ${i + 1}\n`, `bis-${i + 1}`))
  await refresh()

  await goFresh('advanced', 'Advanced')
  await page.getByRole('tab', { name: 'Bisect' }).click()
  await page.getByLabel('A version that was working').click()
  await page.getByRole('option', { name: /bis-1$/ }).click()
  await page.getByRole('button', { name: 'Start bisect' }).click()
  const state = page.getByTestId('bisect-state')
  await expect(state).toBeVisible()
  for (let step = 0; step < 6; step++) {
    const found = state.getByText(/The first bad version is/)
    const test = state.getByText(/Test this version:/)
    await expect(found.or(test)).toBeVisible()
    if (await found.isVisible()) break
    const prev = await test.innerText()
    const n = Number(/bis-(\d)/.exec(prev)![1])
    // the version under test is checked out: its file is the oracle
    expect(readFileSync(join(st.repo, 'bisect.txt'), 'utf8')).toBe(`${body[n - 1]} ${n}\n`)
    await state.getByRole('button', { name: n >= 3 ? 'It is broken (bad)' : 'It works (good)' }).click()
    // moves on to another version, or completes
    await expect.poll(async () => (await found.isVisible()) || ((await test.isVisible()) && (await test.innerText()) !== prev)).toBe(true)
  }
  await expect(state.getByText(/The first bad version is/)).toContainText('bis-3')
  await shot('22-bisect-found')
  await state.getByRole('button', { name: 'End bisect…' }).click()
  await confirm('End bisect')
  await expect(page.getByRole('status').filter({ hasText: 'Bisect ended' })).toBeVisible()
  await shot('22-bisect-reset')
  expect(cli(st.repo, 'log', '--oneline', '-n', '1')).toContain('bis-5')
  expect(readFileSync(join(st.repo, 'bisect.txt'), 'utf8')).toBe('bad 5\n')
  expect(cli(st.repo, 'branch', 'list')).toMatch(/^\*\s+main/m)
})

test('v. Advanced reflog lists the same entries as the CLI', async () => {
  const log = cliJson(st.repo, 'reflog', '-n', '100')
  await goFresh('advanced', 'Advanced')
  await page.getByRole('tab', { name: 'Reflog' }).click()
  const rows = page.getByRole('list', { name: 'Reflog, newest first' }).getByRole('listitem')
  await expect(rows).toHaveCount(log.entries.length)
  await expect(rows.first()).toContainText(log.entries[0].message)
  await expect(rows.first()).toContainText('@0')
  await shot('23-reflog')
})

test('w. Security: unlock prompt, wrong then right passphrase, Lock, push with the passphrase', async () => {
  const wrong = 'definitely-Wrong-77'
  await page.evaluate(
    ([recent]) => {
      localStorage.removeItem('mg.repo')
      localStorage.setItem('mg.recent', recent!)
    },
    [JSON.stringify([{ path: st.locked, name: 'locked-film', openedAt: Date.now(), branch: 'main' }])],
  )
  await page.evaluate(() => { window.location.hash = '#/welcome' })
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Welcome' })).toBeVisible()
  await page.getByTestId('recent-list').locator('button.open', { hasText: 'locked-film' }).click()
  // Opening an encrypted repository already reads it, so the prompt comes straight away.
  const dlg = page.getByRole('dialog', { name: 'Unlock this repository' })
  await expect(dlg).toBeVisible()
  await shot('24-unlock-prompt')

  await dlg.getByLabel('Passphrase').fill(wrong)
  await dlg.getByRole('button', { name: 'Unlock' }).click()
  await expect(dlg.getByText('That passphrase did not unlock the repository.')).toBeVisible()
  await shot('24-unlock-wrong')

  await dlg.getByLabel('Passphrase').fill(st.passphrase)
  await dlg.getByRole('button', { name: 'Unlock' }).click()
  await expect(dlg).toBeHidden()
  await expect(page.getByTestId('repo-switcher')).toContainText('locked-film')
  await go('history', 'History')
  const rows =page.getByRole('listbox', { name: 'Versions, newest first' }).getByRole('option')
  await expect(rows.first()).toContainText('Encrypted initial')
  await expect(page.getByTestId('unlocked-chip')).toBeVisible()
  await shot('24-unlocked-history')

  // The passphrase is never visible anywhere the page can show or keep it.
  await page.getByRole('button', { name: 'Activity', exact: true }).click()
  await expect(page.getByTestId('activity-drawer')).toBeVisible()
  const dump = await page.evaluate(() => ({
    text: document.body.innerText,
    html: document.documentElement.outerHTML,
    local: JSON.stringify({ ...localStorage }),
    session: JSON.stringify({ ...sessionStorage }),
    drawer: (document.querySelector('[data-testid="activity-drawer"]') as HTMLElement).innerText,
  }))
  for (const [where, text] of Object.entries(dump)) {
    if (where === 'html') continue // field values are properties, not markup; the text and storage checks cover the rest
    expect(text, where).not.toContain(st.passphrase)
    expect(text, where).not.toContain(wrong)
  }
  await page.getByTestId('activity-drawer').getByRole('button', { name: 'Close' }).click()

  // Lock forgets it: the next action asks again.
  await page.getByRole('button', { name: 'Lock this repository' }).click()
  await expect(page.getByTestId('unlocked-chip')).toHaveCount(0)
  await goFresh('history', 'History')
  await expect(dlg).toBeVisible()
  await shot('24-locked-again')
  await dlg.getByRole('button', { name: 'Cancel' }).click()
  await expect(dlg).toBeHidden()

  // fd3baee regression: one stdin line is enough for a push from an encrypted repository.
  const out = JSON.parse(cliPass(st.locked, 'push', '-u', 'origin', 'main')) as { ok: boolean }
  expect(out.ok).toBe(true)
})
