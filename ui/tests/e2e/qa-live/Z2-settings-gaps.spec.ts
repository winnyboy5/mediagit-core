import { closeSync, mkdirSync, openSync, readSync, rmSync, statSync, writeFileSync, writeSync } from 'node:fs'
import { join } from 'node:path'
import { test, type Page } from '@playwright/test'
import { WORK } from './env'
import { Recorder, closeTerm } from './rec'
import { dialogText, flat, go, openRepo, toWelcome, visibleErrors } from './gui'
import { copyCorpus, walk } from './integrity'

// Settings gaps from the first pass: repair from the app (T-06), encryption lock and unlock (T-08).
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
const fresh = (name: string) => { const d = join(WORK, name); rmSync(d, { recursive: true, force: true }); mkdirSync(d, { recursive: true }); return d }
const alerts = async (page: Page) => flat((await visibleErrors(page)).join(' || ')).slice(0, 400)
const openAll = (page: Page) => page.evaluate(() => document.querySelectorAll('details').forEach((d) => { d.open = true }))
const main = async (page: Page, n = 700) => flat(await page.locator('main').innerText().catch(() => '')).slice(0, n)

test('T-06b Repair from the app on a damaged store', async () => {
  const rec = await Recorder.start('T-06b', 'repair-in-app', { gui: true })
  const page = rec.page!
  const repo = fresh('T-06b')
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY, quietFrame: true })
  await copyCorpus(repo, { 'freepik__talk__71826.jpeg': 'photo.jpeg', 'creative-pairs/ai/banner_v1.ai': 'banner.ai' })
  await run('init'); await run('add', '--all'); await run('commit', '-m', 'v1')
  // Damage: flip one byte in the middle of the largest stored object.
  const objs = walk(join(repo, '.mediagit'), () => false).filter((f) => /objects/i.test(f)).sort((a, b) => statSync(b).size - statSync(a).size)
  const victim = objs[0]!
  const fd = openSync(victim, 'r+'); const pos = Math.floor(statSync(victim).size / 2); const b = Buffer.alloc(1); readSync(fd, b, 0, 1, pos); b[0] = b[0]! ^ 0xff; writeSync(fd, b, 0, 1, pos); closeSync(fd)
  rec.note(`damaged one byte at ${pos} of ${victim.replace(repo, '')} (${statSync(victim).size} bytes)`)
  const f0 = await rec.cli(repo, ['fsck', '--full'], { env: NOKEY })
  rec.check(f0.code !== 0, 'setup: the CLI fsck sees the damage', flat(f0.stdout + f0.stderr).slice(-300))

  await toWelcome(page)
  await rec.step('Open the project, Settings > Maintenance', async () => { await openRepo(page, repo); await go(page, 'settings/maintenance'); await page.waitForTimeout(2500); await openAll(page) })
  const result = async () => flat(await page.getByTestId('fsck-result').innerText().catch(() => '(no result)'))
  await rec.step('Run integrity check', async () => { await page.getByRole('button', { name: 'Run integrity check' }).click(); await page.getByTestId('fsck-result').waitFor({ timeout: 120_000 }).catch(() => {}); await page.waitForTimeout(800) })
  const r1 = await result()
  rec.note(`check result in the app: ${r1} | alerts ${await alerts(page)}`)
  rec.check(/problem|damag|corrupt|error|issue|fail/i.test(r1 + (await alerts(page))) && !/no problems|all good|healthy|perfect/i.test(r1), 'the app reports the damage', r1)
  await rec.shot('check-damaged').catch(() => {})

  await rec.step('Tick "Try to repair what it finds" (preview stays on), Preview repair', async () => {
    await openAll(page); await page.getByText('Try to repair what it finds').click(); await page.waitForTimeout(500)
    rec.note(`options now: ${(await main(page, 1600)).match(/Try to repair[^/]*\/[^/]*\/[^/]*\/[^/]*/)?.[0] ?? '(not found)'}`)
    await page.getByRole('button', { name: 'Preview repair' }).click(); await page.waitForTimeout(1000); await page.getByTestId('fsck-result').waitFor({ timeout: 120_000 }).catch(() => {}); await page.waitForTimeout(1500)
  })
  const r2 = await result()
  rec.note(`preview result: ${r2} | alerts ${await alerts(page)}`)
  rec.check(/Would repair/i.test(r2), 'the preview says what a repair would change', r2)
  const f1 = await run('fsck', '--full')
  rec.check(f1.code !== 0, 'the preview changed nothing (the store is still damaged)', flat(f1.stdout).slice(-200))

  await rec.step('Untick the preview: the button becomes "Repair…"; cancel the dialog', async () => {
    await page.getByText('Only show what a repair would change').click(); await page.waitForTimeout(500)
    await page.getByRole('button', { name: 'Repair…' }).click(); await page.waitForTimeout(700)
    rec.note(`repair dialog: ${flat(await page.getByRole('alertdialog').innerText().catch(() => '(no dialog)'))}`)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click(); await page.waitForTimeout(600)
  })
  await rec.step('Repair…, confirm', async () => {
    await page.getByRole('button', { name: 'Repair…' }).click(); await page.waitForTimeout(700)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Repair', exact: true }).click(); await page.waitForTimeout(1500)
    await page.getByTestId('fsck-result').waitFor({ timeout: 120_000 }).catch(() => {}); await page.waitForTimeout(1500)
  })
  const r3 = await result()
  rec.note(`repair result in the app: ${r3} | alerts ${await alerts(page)}`)
  await rec.shot('after-repair').catch(() => {})
  const f2 = await rec.cli(repo, ['fsck', '--full'], { env: NOKEY })
  const co = await rec.cli(repo, ['reset', '--hard', 'HEAD'], { env: NOKEY })
  const claims = /Repaired\s*\/?\s*[1-9]/i.test(r3)
  rec.note(`truth after the repair: fsck exit ${f2.code}; reset --hard exit ${co.code} ${flat(co.stdout + co.stderr).slice(-160)}`)
  rec.check(!(claims && f2.code !== 0), 'the app does not claim a repair while the store is still damaged (BUG-007 from the app)', `app: ${r3} | fsck exit ${f2.code}`)
  rec.check(f2.code === 0 || /could not|cannot|not repair|still|remain/i.test(r3 + (await alerts(page))), 'if the damage cannot be repaired, the app says so', r3)
  await toWelcome(page)
  await rec.finish(); await closeTerm()
})

test('T-08b Encryption with a passphrase: lock, unlock, wrong passphrase', async () => {
  const rec = await Recorder.start('T-08b', 'lock-unlock', { gui: true })
  const page = rec.page!
  const repo = fresh('T-08b')
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY, quietFrame: true })
  await run('init') // encryption can only be turned on in an empty repository
  const PASS = 'Qa-Live!2026-passphrase'
  const enc = page.getByTestId('encryption')
  const dlg = page.getByRole('dialog')
  const card = async () => flat(await enc.innerText().catch(() => '(no encryption card)'))

  await toWelcome(page)
  await rec.step('Open the project, Settings > Security', async () => { await openRepo(page, repo); await go(page, 'settings/security'); await page.waitForTimeout(2500) })
  rec.note(`card before: ${await card()}`)
  await rec.step('Turn on encryption… with a passphrase', async () => {
    await enc.getByRole('button', { name: 'Turn on encryption…' }).click(); await page.waitForTimeout(500)
    rec.note(`dialog: ${await dialogText(page)}`)
    await dlg.getByLabel('Passphrase (optional)').fill(PASS); await page.waitForTimeout(200)
    await dlg.getByRole('button', { name: 'Turn on encryption', exact: true }).click(); await page.waitForTimeout(4000)
  })
  const codeShown = await page.getByTestId('recovery-code').isVisible().catch(() => false)
  rec.check(codeShown, 'a recovery code is shown once', await alerts(page))
  rec.note(`recovery dialog: ${(await dialogText(page)).replace(/[A-Za-z0-9]{4,}(-[A-Za-z0-9]{4,}){2,}/g, '<code hidden>').slice(0, 400)} | key source: ${flat(await page.getByTestId('key-source').innerText().catch(() => ''))}`)
  await rec.step('Confirm "I saved it" and close', async () => {
    const buttons = await dlg.getByRole('button').allInnerTexts()
    rec.note(`buttons: ${buttons.join(' | ')}`)
    await dlg.getByText('I saved it').click(); await page.waitForTimeout(300)
    await dlg.getByRole('button').last().click(); await page.waitForTimeout(1500)
  })
  rec.note(`card after turning on: ${await card()}`)
  const hasLock = await enc.getByRole('button', { name: 'Lock now' }).isVisible().catch(() => false)
  rec.note(`"Lock now" visible right after turning encryption on with a passphrase: ${hasLock}`)

  // Work that needs the key: save a new version from the app.
  const saveOne = async (name: string) => {
    writeFileSync(join(repo, name), `${name}\n`)
    await go(page, 'files/changes'); await page.waitForTimeout(2500)
    await page.getByRole('button', { name: 'Include all', exact: true }).click().catch(() => {}); await page.waitForTimeout(2500)
  }
  if (hasLock) await rec.step('Lock now', async () => { await enc.getByRole('button', { name: 'Lock now' }).click(); await page.waitForTimeout(800); rec.note(`card after Lock now: ${await card()}`) })
  await rec.step('Locked (or never unlocked): include a new file', () => saveOne('b.txt'))
  const locked = `${await main(page, 500)} | alerts ${await alerts(page)} | dialog ${await dlg.isVisible().catch(() => false) ? await dialogText(page) : '(none)'}`
  rec.note(`screen when the key is needed: ${locked}`)
  await rec.shot('needs-key').catch(() => {})
  const asks = /locked|passphrase|unlock/i.test(locked)
  rec.check(asks, 'the app asks for the passphrase (or says the repository is locked) when the key is needed', locked.slice(0, 300))
  const openUnlock = async () => { if (!(await dlg.isVisible().catch(() => false))) await page.getByRole('button', { name: 'Unlock…' }).first().click().catch(() => {}); await page.waitForTimeout(600) }
  if (asks) {
    await rec.step('Unlock with a wrong passphrase', async () => {
      await openUnlock(); rec.note(`unlock dialog: ${await dialogText(page)}`)
      await dlg.locator('input[type="password"]').first().fill('not the passphrase'); await dlg.getByRole('button').last().click(); await page.waitForTimeout(3500)
    })
    const wrong = `${await alerts(page)} | dialog ${await dlg.isVisible().catch(() => false) ? await dialogText(page) : '(closed)'}`
    rec.note(`wrong passphrase: ${wrong.slice(0, 400)}`)
    rec.check(/passphrase|wrong|incorrect|did not|does not|locked/i.test(wrong) && !/os error|panicked|exit code|aead|decrypt.*failed:/i.test(wrong), 'a wrong passphrase is refused in plain words', wrong.slice(0, 300))
    await rec.step('Unlock with the right passphrase', async () => {
      await openUnlock()
      await dlg.locator('input[type="password"]').first().fill(PASS); await dlg.getByRole('button').last().click(); await page.waitForTimeout(4000)
    })
    rec.note(`after the right passphrase: ${await main(page, 300)} | alerts ${await alerts(page)}`)
  }
  await rec.step('Include all, message, Save version', async () => {
    await go(page, 'files/changes'); await page.waitForTimeout(2000)
    const inc = page.getByRole('button', { name: 'Include all', exact: true }); if (await inc.isVisible().catch(() => false)) { await inc.click(); await page.waitForTimeout(2500) }
    await page.getByLabel('Message').first().fill('v2 encrypted'); await page.getByRole('button', { name: 'Save version', exact: true }).click(); await page.waitForTimeout(4000)
  })
  rec.note(`after Save: alerts ${await alerts(page)}`)
  const log = await rec.cli(repo, ['--machine', '--secret-stdin', 'log', '-n', '5'], { env: NOKEY, input: PASS + '\n' })
  rec.check(/v2 encrypted/.test(log.stdout), 'the version saved after unlocking', flat(log.stdout + log.stderr).slice(0, 200))
  await go(page, 'settings/security'); await page.waitForTimeout(2000)
  rec.note(`card at the end: ${await card()}`)
  rec.check(await enc.getByRole('button', { name: 'Lock now' }).isVisible().catch(() => false), '"Lock now" is offered once the session is unlocked', await card())
  const noPass = await rec.cli(repo, ['reset', '--hard', 'HEAD'], { env: NOKEY })
  rec.note(`CLI reset --hard with no passphrase given: exit ${noPass.code} ${flat(noPass.stdout + noPass.stderr).slice(-200)}`)
  const plain = walk(join(repo, '.mediagit'), () => false).filter((f) => statSync(f).size < 5e6).filter((f) => { const fd = openSync(f, 'r'); const buf = Buffer.alloc(Math.min(statSync(f).size, 5e6)); readSync(fd, buf, 0, buf.length, 0); closeSync(fd); return buf.includes(PASS) })
  rec.check(plain.length === 0, 'the passphrase is in no file under .mediagit', plain.join(', '))
  await toWelcome(page)
  await rec.finish(); await closeTerm()
})

for (const withPass of [true, false]) {
  test(`T-08c Recovery code shown after Turn on encryption (${withPass ? 'with' : 'without'} a passphrase)`, async () => {
    const id = withPass ? 'T-08c' : 'T-08d'
    const rec = await Recorder.start(id, withPass ? 'recovery-code-with-passphrase' : 'recovery-code-no-passphrase', { gui: true })
    const page = rec.page!
    const repo = fresh(id)
    await rec.cli(repo, ['init'], { env: NOKEY, quietFrame: true })
    const enc = page.getByTestId('encryption'); const dlg = page.getByRole('dialog')
    await toWelcome(page); await openRepo(page, repo); await go(page, 'settings/security'); await page.waitForTimeout(2500)
    await enc.getByRole('button', { name: 'Turn on encryption…' }).click(); await page.waitForTimeout(500)
    if (withPass) await dlg.getByLabel('Passphrase (optional)').fill('Qa-Live!2026-passphrase')
    await dlg.getByRole('button', { name: 'Turn on encryption', exact: true }).click()
    const seen: string[] = []
    let codeMs = -1; let codeGoneMs = -1
    const t = Date.now()
    while (Date.now() - t < 12_000) {
      const code = await page.getByTestId('recovery-code').isVisible().catch(() => false)
      const titles = (await page.locator('[role="dialog"] h2, [role="alertdialog"] h2').allInnerTexts().catch(() => [] as string[])).join('+') || '(no dialog)'
      const s = `${code ? 'CODE' : 'no-code'} ${titles}`
      if (seen[seen.length - 1]?.split(' @')[0] !== s) seen.push(`${s} @${Date.now() - t}ms`)
      if (code && codeMs < 0) { codeMs = Date.now() - t; await rec.shot('code-visible').catch(() => {}) }
      if (!code && codeMs >= 0 && codeGoneMs < 0) codeGoneMs = Date.now() - t
      await page.waitForTimeout(150)
    }
    rec.note(`dialog timeline over 12 s: ${seen.join(' | ')}`)
    rec.note(`encryption card: ${flat(await enc.innerText().catch(() => ''))}`)
    await rec.shot('after-12s').catch(() => {})
    rec.check(codeMs >= 0, 'the recovery code is shown', seen.join(' | '))
    rec.check(codeMs >= 0 && codeGoneMs < 0, 'the recovery code stays on screen until the user confirms "I saved it"', codeGoneMs >= 0 ? `it disappeared after ${codeGoneMs - codeMs} ms without any click` : seen.join(' | '))
    await page.keyboard.press('Escape').catch(() => {})
    await toWelcome(page).catch(() => {})
    await rec.finish(); await closeTerm()
  })
}

// BUG-037 (fixed): a repository that already holds a version is not offered encryption it cannot take.
test('T-08d Encryption is not offered once a version exists', async () => {
  const rec = await Recorder.start('T-08d', 'enc-not-offered', { gui: true })
  const page = rec.page!
  const repo = fresh('T-08d')
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY, quietFrame: true })
  await run('init')
  writeFileSync(join(repo, 'a.txt'), 'first\n')
  await run('add', '--all')
  await run('commit', '-m', 'first version')
  await toWelcome(page)
  await rec.step('Open the project, Settings > Security', async () => { await openRepo(page, repo); await go(page, 'settings/security'); await page.waitForTimeout(2500) })
  const enc = page.getByTestId('encryption')
  rec.check((await enc.getByRole('button', { name: 'Turn on encryption…' }).count()) === 0, 'BUG-037: no "Turn on encryption…" button when the repository already has a version')
  rec.check(/must be turned on before the first version is saved/i.test(flat(await enc.innerText().catch(() => ''))), 'BUG-037: the card says why, in one plain sentence', flat(await enc.innerText().catch(() => '')).slice(0, 300))
  await rec.finish()
  await closeTerm()
})
