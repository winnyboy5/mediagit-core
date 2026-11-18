import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { attach, mg, openRecent, stubPicker, tmp, type Ctx } from './bugs'

// Wave 6 / 6A3, real app. Held with `test.fixme` until a release build with these fixes exists
// (the app under test is the built binary). Remove `.fixme` per test once it has run green.
let c: Ctx
test.beforeAll(async () => { c = await attach() })
test.afterAll(async () => { await c?.browser.close().catch(() => {}) })

function repoWith(tag: string) {
  const repo = join(tmp(tag), `${tag}-film`)
  mkdirSync(repo, { recursive: true })
  mg(c.st, repo, ['init'])
  writeFileSync(join(repo, 'a.txt'), 'one\n')
  mg(c.st, repo, ['add', '.'])
  mg(c.st, repo, ['commit', '-m', 'one'])
  return repo
}

test.fixme('BUG-005: Create from the rail, then Cancel, keeps the open repository', async () => {
  const { page } = c
  const repo = repoWith('bug005')
  await openRecent(page, repo, 'bug005-film')
  await stubPicker(page, join(tmp('bug005-new'), 'new'))
  await page.getByTestId('repo-switcher').click()
  await page.getByRole('menuitem', { name: 'Create a new repository…' }).click()
  await page.getByRole('dialog', { name: 'Create repository' }).getByRole('button', { name: 'Cancel' }).click()
  await expect(page.getByTestId('repo-switcher')).toContainText('bug005-film')
})

test.fixme('BUG-005: Open on a folder that is not a repository says so', async () => {
  const { page } = c
  const repo = repoWith('bug005b')
  await openRecent(page, repo, 'bug005b-film')
  await stubPicker(page, tmp('bug005b-empty'))
  await page.getByTestId('repo-switcher').click()
  await page.getByRole('menuitem', { name: 'Open a repository…' }).click()
  await expect(page.getByText('Could not open this folder')).toBeVisible()
})

test.fixme('BUG-017: Discard lists the files and names the new one it deletes', async () => {
  const { page } = c
  const repo = repoWith('bug017')
  writeFileSync(join(repo, 'new-included.txt'), 'new\n')
  mg(c.st, repo, ['add', 'new-included.txt'])
  writeFileSync(join(repo, 'a.txt'), 'edited\n')
  await openRecent(page, repo, 'bug017-film')
  await page.getByRole('button', { name: 'More actions' }).click()
  await page.getByRole('menuitem', { name: 'Discard all unsaved changes…' }).click()
  const dlg = page.getByRole('dialog')
  await expect(dlg).toContainText('1 new file will be deleted')
  await expect(dlg).toContainText('new-included.txt')
  await expect(dlg).toContainText('a.txt')
})

test.fixme('BUG-038: Cancel staging leaves the repository clean and a second Include all works', async () => {
  const { page } = c
  const repo = repoWith('bug038')
  for (let i = 0; i < 6; i++) writeFileSync(join(repo, `big${i}.bin`), Buffer.alloc(150_000_000, i + 1))
  await openRecent(page, repo, 'bug038-film')
  await page.getByRole('button', { name: 'Include all', exact: true }).click()
  await page.getByRole('button', { name: 'Cancel staging' }).click()
  await expect(page.getByTestId('preview-note')).toContainText('Stopped')
  expect(mg(c.st, repo, ['fsck'])).toContain('PERFECT')
  await page.getByRole('button', { name: 'Include all', exact: true }).click()
  await expect(page.getByTestId('changes-summary')).toBeVisible()
})
