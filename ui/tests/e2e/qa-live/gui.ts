import type { Page } from '@playwright/test'

// Shared GUI helpers for the live QA cases.

/**
 * The native folder picker cannot be driven over CDP. Answer the dialog plugin's IPC call
 * from the page instead (same trick as tests/e2e/real/w2.spec.ts). `dir = null` = user cancelled.
 * Every later picker call gets this answer until it is set again.
 */
export async function answerPicker(page: Page, dir: string | null) {
  await page.evaluate((d) => {
    const w = window as unknown as { fetch: typeof fetch; __qaRealFetch?: typeof fetch; __qaPick: string | null; __qaPickHits: number }
    w.__qaPick = d
    w.__qaPickHits = 0
    if (w.__qaRealFetch) return
    w.__qaRealFetch = w.fetch.bind(window)
    w.fetch = (input, init) => {
      if (String(input instanceof Request ? input.url : input).includes('plugin%3Adialog')) {
        w.__qaPickHits++
        return Promise.resolve(new Response(JSON.stringify(w.__qaPick), { headers: { 'Content-Type': 'application/json', 'Tauri-Response': 'ok' } }))
      }
      return w.__qaRealFetch!(input, init)
    }
  }, dir)
}

export const pickerHits = (page: Page) => page.evaluate(() => (window as unknown as { __qaPickHits: number }).__qaPickHits)

/** Navigate by hash route (`welcome`, `files/changes`, `settings/maintenance`...). */
export async function go(page: Page, path: string) {
  await page.evaluate((p) => { window.location.hash = `#/${p}` }, path)
  await page.waitForTimeout(250)
}

/** Back to a clean Welcome screen with no open repository. Recents are kept unless `clearRecent`. */
export async function toWelcome(page: Page, clearRecent = false) {
  await page.evaluate(async (clear) => {
    // R-03: the app prefers the desktop side's last-repo record on reload.
    await (window as unknown as { __TAURI_INTERNALS__: { invoke(c: string, a: unknown): Promise<unknown> } }).__TAURI_INTERNALS__.invoke('last_repo_set', { repo: null })
    localStorage.removeItem('mg.repo')
    if (clear) localStorage.removeItem('mg.recent')
    window.location.hash = '#/welcome'
  }, clearRecent)
  await page.reload()
  await page.getByRole('heading', { name: 'Welcome' }).waitFor()
}

/** Ask the app to refresh the way a window focus does. */
export const refocus = (page: Page) => page.evaluate(() => window.dispatchEvent(new Event('focus')))

/** Text of every visible alert, toast and error panel: for the "plain words" review. */
export async function visibleErrors(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll('[role="alert"], [role="status"], .toast, .error-state, .fail')]
      .map((e) => (e as HTMLElement).innerText.trim())
      .filter(Boolean),
  )
}

/** Open a repository through the Welcome recent list (the picker-free path a user also has). */
export async function openRepo(page: Page, path: string) {
  const name = path.split(/[\\/]/).pop()!
  await page.evaluate(async ([p, n]) => {
    await (window as unknown as { __TAURI_INTERNALS__: { invoke(c: string, a: unknown): Promise<unknown> } }).__TAURI_INTERNALS__.invoke('last_repo_set', { repo: null })
    localStorage.removeItem('mg.repo')
    const list = JSON.parse(localStorage.getItem('mg.recent') ?? '[]') as { path: string }[]
    localStorage.setItem('mg.recent', JSON.stringify([{ path: p, name: n, openedAt: Date.now() }, ...list.filter((r) => r.path !== p)]))
    window.location.hash = '#/welcome'
  }, [path, name] as const)
  await page.reload()
  await page.getByTestId('recent-list').locator('button.open', { hasText: name }).first().click()
  await page.getByRole('navigation', { name: 'Main' }).waitFor({ timeout: 30_000 })
}

export const flat = (s: string) => s.replace(/\n+/g, ' / ')

/** Pick an option of one of the app's Select fields (label text, then the option). */
export async function choose(page: Page, label: string, option: RegExp | string, settle = 1500) {
  await page.locator('.field').filter({ has: page.locator('label', { hasText: new RegExp(`^${label}$`) }) }).locator('.trigger').last().click()
  await page.waitForTimeout(400)
  await page.getByRole('option', { name: option }).first().click()
  await page.waitForTimeout(settle)
}

/** Read one expression of a Pinia store (`s` is the store). */
export const store = <T>(page: Page, id: string, expr: string) =>
  page.evaluate(([i, e]) => {
    const app = (document.querySelector('#app') as unknown as { __vue_app__: { config: { globalProperties: { $pinia: { _s: Map<string, Record<string, unknown>> } } } } }).__vue_app__
    const s = app.config.globalProperties.$pinia._s.get(i!)
    return s ? (JSON.parse(JSON.stringify(new Function('s', `return ${e}`)(s) ?? null)) as unknown) : '(no store)'
  }, [id, expr] as const) as Promise<T>

export const dialogText = async (page: Page) => flat(await page.locator('[role="dialog"], [role="alertdialog"]').last().innerText().catch(() => '(no dialog)'))

/** Row menu: open "Actions for <name>" and click the item. */
export async function rowMenu(page: Page, name: string, item: RegExp) {
  await page.getByRole('button', { name: `Actions for ${name}`, exact: true }).click()
  await page.waitForTimeout(400)
  await page.getByRole('menuitem', { name: item }).click()
  await page.waitForTimeout(700)
}

export interface ProgressSample { t: number; files: string; bytes: string; rate: string; eta: string; bar: string | null; pill: string }

/** Read the header activity pill once. Empty strings mean the element is not on screen. */
export function sampleProgress(page: Page, t0: number): Promise<ProgressSample> {
  return page.evaluate((start) => {
    const txt = (id: string) => (document.querySelector(`[data-testid="${id}"]`) as HTMLElement | null)?.innerText.trim() ?? ''
    const bar = document.querySelector('[role="progressbar"]')
    const pill = (document.querySelector('[data-activity-pill]') as HTMLElement | null)?.innerText.replace(/\s+/g, ' ').trim() ?? ''
    return { t: Date.now() - start, files: txt('pill-files'), bytes: txt('pill-bytes'), rate: txt('pill-rate'), eta: txt('pill-eta'), bar: bar ? String((100 * Number(bar.getAttribute('aria-valuenow'))) / (Number(bar.getAttribute('aria-valuemax')) || 100)) : null, pill }
  }, t0)
}

/** "1.2 GB" / "340 MB of 2.3 GB" to bytes done (first number). NaN when unreadable. */
export function parseBytes(s: string): number {
  const m = s.match(/([\d.,]+)\s*(B|KB|MB|GB|TB|KiB|MiB|GiB)/i)
  if (!m) return NaN
  const mult: Record<string, number> = { b: 1, kb: 1e3, mb: 1e6, gb: 1e9, tb: 1e12, kib: 1024, mib: 1024 ** 2, gib: 1024 ** 3 }
  return Number(m[1]!.replace(',', '')) * mult[m[2]!.toLowerCase()]!
}
