import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { attach, mg, openRecent, startServer, tmp, type Ctx, type Srv } from './bugs'

// BUG-033, real app: Get latest that clashes with a local edit stops as a merge (markers,
// MERGE_HEAD) and opens the merge page; deciding the file and finishing makes a two-parent commit.
let c: Ctx
let srv: Srv
test.beforeAll(async () => {
  c = await attach()
  srv = await startServer(c.st, 'pull-repo', false)
})
test.afterAll(async () => {
  srv?.stop()
  await c?.browser.close().catch(() => {})
})

test('BUG-033: Get latest with a clash opens the merge and finishes with two parents', async () => {
  const { st, page } = c
  const base = tmp('bug033')
  const repo = join(base, 'bug033')
  const other = join(base, 'other')
  mkdirSync(repo)
  const commit = (dir: string, text: string, msg: string) => {
    writeFileSync(join(dir, 'notes.txt'), text)
    mg(st, dir, ['add', '.'])
    mg(st, dir, ['commit', '-m', msg])
  }
  mg(st, repo, ['init'])
  commit(repo, 'base\n', 'base')
  mg(st, repo, ['remote', 'add', 'origin', `${srv.url}/pull-repo`])
  mg(st, repo, ['push', '-u', 'origin', 'main'])
  mg(st, base, ['clone', `${srv.url}/pull-repo`, other])
  commit(other, 'from the other clone\n', 'other edit')
  mg(st, other, ['push', 'origin', 'main'])
  commit(repo, 'from my clone\n', 'my edit')

  await openRecent(page, repo, 'bug033')
  await page.evaluate(() => { window.location.hash = '#/branches/sync' })
  await page.getByRole('button', { name: /^Get latest( \d+)?$/ }).click()

  await expect(page).toHaveURL(/#\/branches\/merge/)
  const row = page.locator('[data-file="notes.txt"]')
  await expect(row).toBeVisible()
  expect(existsSync(join(repo, '.mediagit', 'MERGE_HEAD'))).toBe(true)
  expect(readFileSync(join(repo, 'notes.txt'), 'utf8')).toContain('<<<<<<<')

  await row.getByRole('button', { name: /^Keep / }).first().click()
  await page.getByRole('button', { name: 'Finish merge' }).click()
  await expect(page.getByTestId('merge-result')).toContainText('Done')
  expect(existsSync(join(repo, '.mediagit', 'MERGE_HEAD'))).toBe(false)
  expect(mg(st, repo, ['log', '--oneline', '-n', '1'])).toContain('Merge')
})
