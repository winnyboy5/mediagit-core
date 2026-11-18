import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { attach, mg, openRecent, tmp, type Ctx } from './bugs'

// BUG-029, real app: finish a rebase conflict by keeping each side, on fresh repos.
// During a rebase HEAD is the branch being moved (feature); "Keep main" must take the branch built on.
let c: Ctx
test.beforeAll(async () => { c = await attach() })
test.afterAll(async () => { await c?.browser.close().catch(() => {}) })

const MAIN = 'main says this\n'
const FEATURE = 'feature says this\n'

for (const side of ['main', 'feature'] as const) {
  test(`BUG-029: Keep ${side} finishes the rebase with ${side}'s text`, async () => {
    const { st, page } = c
    const repo = join(tmp('bug029'), `bug029-${side}`)
    mkdirSync(repo)
    const commit = (text: string, msg: string) => {
      writeFileSync(join(repo, 'notes.txt'), text)
      mg(st, repo, ['add', '.'])
      mg(st, repo, ['commit', '-m', msg])
    }
    mg(st, repo, ['init'])
    commit('base\n', 'base')
    mg(st, repo, ['branch', 'create', 'feature'])
    commit(MAIN, 'main edit')
    mg(st, repo, ['branch', 'switch', 'feature'])
    commit(FEATURE, 'feature edit')

    await openRecent(page, repo, `bug029-${side}`)
    await page.evaluate(() => { window.location.hash = '#/settings/advanced' })
    await expect(page.getByLabel('Rebase onto')).toBeVisible()
    await page.getByLabel('Rebase onto').click()
    await page.getByRole('option', { name: 'main', exact: true }).click()
    await page.getByRole('button', { name: 'Start rebase…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Start rebase' }).click()

    // The rebase stopped: Changes says so and points at Advanced tools.
    await page.evaluate(() => { window.location.hash = '#/files/changes' })
    const banner = page.getByTestId('operation-banner')
    await expect(banner).toContainText('A rebase is in progress.')
    await banner.getByRole('link', { name: 'Decide the files' }).click()

    const row = page.locator('[data-file="notes.txt"]')
    await expect(row.getByRole('button', { name: 'Keep main' })).toBeVisible()
    await expect(row.getByRole('button', { name: 'Keep feature' })).toBeVisible()
    await expect(page.getByRole('button', { name: /Keep mine/ })).toHaveCount(0)
    await row.getByRole('button', { name: `Keep ${side}` }).click()
    await page.getByRole('button', { name: 'Continue rebase' }).click()
    await expect(page.getByTestId('rebase-result')).toContainText('Rebased.')

    expect(readFileSync(join(repo, 'notes.txt'), 'utf8')).toBe(side === 'main' ? MAIN : FEATURE)
  })
}
