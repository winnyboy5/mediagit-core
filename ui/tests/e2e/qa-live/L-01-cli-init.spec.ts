import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { Recorder, closeTerm, workDir } from './rec'
import { intCheck } from './integrity'

// L-01: CLI init in every form, plus a second init on a repo that already holds work.
test('L-01 CLI init', async () => {
  const rec = await Recorder.start('L-01', 'cli-init')
  const root = workDir('L-01')
  rmSync(root, { recursive: true, force: true })
  mkdirSync(root, { recursive: true })

  // 1. init in the current folder
  const a = join(root, 'a')
  mkdirSync(a)
  let r = await rec.cli(a, ['init'])
  rec.check(r.code === 0 && existsSync(join(a, '.mediagit')), 'init in cwd makes .mediagit')
  r = await rec.cli(a, ['status'])
  rec.check(r.code === 0 && /No commits yet/.test(r.stdout), 'status works on the empty repo', r.stdout.slice(0, 200))
  // Plain `status` prints no branch line (only with --branch): logged as IMP-005, not a failure.
  rec.note(`plain status names the branch: ${/main/.test(r.stdout)}`)
  r = await rec.cli(a, ['status', '--branch'])
  rec.check(/On branch: main/.test(r.stdout), 'status --branch shows main', r.stdout.slice(0, 200))

  // 2. init <path> (folder does not exist yet)
  const b = join(root, 'b', 'nested', 'repo')
  r = await rec.cli(root, ['init', b])
  rec.check(r.code === 0 && existsSync(join(b, '.mediagit')), 'init <path> creates missing parent folders')

  // 3. --initial-branch
  const c = join(root, 'c')
  r = await rec.cli(root, ['init', '--initial-branch', 'dev', c])
  const st = await rec.cli(c, ['status', '--branch', '--short'])
  rec.check(r.code === 0 && /dev/.test(st.stdout), '--initial-branch dev is honoured', st.stdout.slice(0, 200))

  // 4. --bare (documented alias: same layout)
  const d = join(root, 'd')
  r = await rec.cli(root, ['init', '--bare', d])
  rec.check(r.code === 0 && existsSync(join(d, '.mediagit')), 'init --bare makes the same .mediagit layout')

  // 5. bad branch names
  for (const bad of ['has space', '..', 'a..b', 'end.lock', '-dash', 'x~y']) {
    const e = join(root, `bad-${bad.replace(/[^a-z]/g, '_')}`)
    r = await rec.cli(root, ['init', '--initial-branch', bad, e], { quietFrame: true })
    rec.check(r.code !== 0, `init refuses branch name "${bad}"`, `exit ${r.code} ${(r.stderr || r.stdout).trim().slice(0, 160)}`)
  }

  // 6. second init on a repo with a commit: must not damage it
  writeFileSync(join(a, 'notes.txt'), 'first draft\n')
  await rec.cli(a, ['add', '.'])
  await rec.cli(a, ['commit', '-m', 'first'])
  const before = (await rec.cli(a, ['log', '--oneline'], { quietFrame: true })).stdout
  const cfgBefore = readFileSync(join(a, '.mediagit', 'config.toml'), 'utf8')
  r = await rec.cli(a, ['init'])
  rec.note(`second init: exit ${r.code}: ${(r.stdout + r.stderr).trim().slice(0, 300)}`)
  const after = (await rec.cli(a, ['log', '--oneline'], { quietFrame: true })).stdout
  rec.check(before === after && before.includes('first'), 'second init leaves history untouched', after.slice(0, 120))
  rec.check(readFileSync(join(a, '.mediagit', 'config.toml'), 'utf8') === cfgBefore, 'second init leaves config.toml untouched')
  rec.check(!/panick|backtrace|os error/i.test(r.stdout + r.stderr), 'second init message is plain words')
  r = await rec.cli(a, ['init', '--initial-branch', 'other'])
  const br = await rec.cli(a, ['branch', 'list'])
  rec.check(/main/.test(br.stdout) && !/^\*\s*other/m.test(br.stdout), 'second init with another branch name does not move HEAD', br.stdout.slice(0, 200))

  // 7. init inside a subfolder of an existing repo (nested repo)
  const sub = join(a, 'sub')
  mkdirSync(sub)
  r = await rec.cli(sub, ['init'])
  rec.note(`nested init: exit ${r.code}, .mediagit made in sub: ${existsSync(join(sub, '.mediagit'))}: ${(r.stdout + r.stderr).trim().slice(0, 300)}`)

  // 8. init on a path that is a file
  writeFileSync(join(root, 'afile'), 'x')
  r = await rec.cli(root, ['init', join(root, 'afile')])
  rec.check(r.code !== 0 && !/panick/i.test(r.stderr), 'init on a file path fails cleanly', (r.stderr || r.stdout).trim().slice(0, 200))

  // 9. machine mode envelope
  const m = join(root, 'm')
  r = await rec.cli(root, ['--machine', 'init', m])
  let env: { ok?: boolean } = {}
  try { env = JSON.parse(r.stdout) } catch { /* checked below */ }
  rec.check(env.ok === true && r.stdout.trim().split('\n').length === 1, '--machine init prints one JSON envelope', r.stdout.slice(0, 200))
  r = await rec.cli(m, ['--machine', 'init'])
  rec.note(`machine second init: exit ${r.code} ${r.stdout.trim().slice(0, 300)}`)

  await intCheck(rec, a)
  for (const x of [b, c, d, m]) rec.touch(x)
  await rec.finish()
  await closeTerm()
})
