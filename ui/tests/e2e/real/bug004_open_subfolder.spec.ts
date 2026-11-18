import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { attach, mg, stubPicker, tmp, type Ctx } from './bugs'

// BUG-004, real app: choosing a subfolder in the folder picker opens the repository root.
let c: Ctx
test.beforeAll(async () => { c = await attach() })
test.afterAll(async () => { await c?.browser.close().catch(() => {}) })

test('BUG-004: the rail shows the parent name and no nested .mediagit appears', async () => {
  const { st, page } = c
  const repo = join(tmp('bug004'), 'bug004-film')
  mkdirSync(join(repo, 'art'), { recursive: true })
  mg(st, repo, ['init'])
  writeFileSync(join(repo, 'art', 'a.txt'), 'art\n')
  mg(st, repo, ['add', '.'])
  mg(st, repo, ['commit', '-m', 'art'])

  await page.evaluate(() => { localStorage.removeItem('mg.repo'); localStorage.setItem('mg.recent', '[]') })
  await page.evaluate(() => { window.location.hash = '#/welcome' })
  await page.reload()
  await stubPicker(page, join(repo, 'art'))
  await page.getByRole('button', { name: 'Open folder…' }).click()
  await expect(page.getByTestId('repo-switcher')).toContainText('bug004-film')
  await expect(page.getByTestId('repo-switcher')).not.toHaveText(/^\s*art\s*$/)
  expect(existsSync(join(repo, 'art', '.mediagit'))).toBe(false)
})
