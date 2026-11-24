import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { CORPUS, WORK } from './env'
import { Recorder, closeTerm } from './rec'
import { flat } from './gui'
import { startServer } from './server'

// CLI-only second reproductions for S-09 (force-with-lease) and S-10 (pull with a conflict).
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }

test('X-S09 force-with-lease from a stale copy, CLI only', async () => {
  const rec = await Recorder.start('X-S09', 'lease-cli-repro')
  const root = join(WORK, 'X-S09'); rmSync(root, { recursive: true, force: true }); mkdirSync(root, { recursive: true })
  const s = await startServer(join(root, 'srv'), ['r'])
  const a = join(root, 'a'); const b = join(root, 'b'); mkdirSync(a)
  const run = (cwd: string, ...args: string[]) => rec.cli(cwd, args, { env: NOKEY })
  const tips = async (cwd: string, ...rev: string[]) => { const r = await rec.cli(cwd, ['log', '--machine', ...rev], { env: NOKEY, quietFrame: true }); return [...r.stdout.matchAll(/"summary":"([^"]*)"/g)].map((m) => m[1]!) }
  const remote = async () => { const p = join(root, 'probe'); rmSync(p, { recursive: true, force: true }); await rec.cli(root, ['clone', s.url('r'), p], { env: NOKEY, quietFrame: true }); return { tips: await tips(p), files: readdirSync(p).filter((n) => n !== '.mediagit') } }
  await run(a, 'init'); writeFileSync(join(a, 'notes.txt'), 'base\n'); await run(a, 'add', '--all'); await run(a, 'commit', '-m', 'v1')
  await run(a, 'remote', 'add', 'origin', s.url('r')); await run(a, 'push', '-u', 'origin', 'main')
  await run(root, 'clone', s.url('r'), b)
  writeFileSync(join(b, 'ben.txt'), 'ben\n'); await run(b, 'add', '--all'); await run(b, 'commit', '-m', 'b1'); const pb = await run(b, 'push', 'origin', 'main')
  rec.check(pb.code === 0, 'setup: Ben pushed b1')
  writeFileSync(join(a, 'ana.txt'), 'ana\n'); await run(a, 'add', '--all'); await run(a, 'commit', '-m', 'a1')
  rec.note(`Ana has never fetched b1. Her origin/main: ${(await tips(a, 'refs/remotes/origin/main')).join(' < ') || '(not resolvable)'}`)
  const plain = await run(a, 'push', 'origin', 'main')
  rec.check(plain.code !== 0 || /non-fast-forward|rejected/i.test(plain.stdout + plain.stderr), 'a plain push from the stale copy is refused', flat(plain.stdout + plain.stderr).slice(-200))
  const lease = await run(a, 'push', '--force-with-lease', 'origin', 'main')
  const r = await remote()
  rec.note(`push --force-with-lease from the stale copy: exit ${lease.code} ${flat(lease.stdout + lease.stderr).slice(-250)} | remote now: ${r.tips.join(' < ')} | files ${r.files.join(',')}`)
  rec.check(r.tips[0] === 'b1' && r.files.includes('ben.txt'), '--force-with-lease must NOT overwrite a version the pusher has never seen', `remote tip ${r.tips[0]}; ben.txt ${r.files.includes('ben.txt') ? 'kept' : 'GONE from the branch'}`)
  s.stop()
  await rec.finish()
  await closeTerm()
})

test('X-S10 pull when both sides changed the same file, CLI only', async () => {
  const rec = await Recorder.start('X-S10', 'pull-conflict-cli-repro')
  const root = join(WORK, 'X-S10'); rmSync(root, { recursive: true, force: true }); mkdirSync(root, { recursive: true })
  const s = await startServer(join(root, 'srv'), ['r'])
  const a = join(root, 'a'); const b = join(root, 'b'); mkdirSync(a)
  const run = (cwd: string, ...args: string[]) => rec.cli(cwd, args, { env: NOKEY })
  await run(a, 'init'); writeFileSync(join(a, 'notes.txt'), 'base\n'); cpSync(join(CORPUS, 'freepik__talk__71826.jpeg'), join(a, 'photo.jpeg')); await run(a, 'add', '--all'); await run(a, 'commit', '-m', 'v1')
  await run(a, 'remote', 'add', 'origin', s.url('r')); await run(a, 'push', '-u', 'origin', 'main')
  await run(root, 'clone', s.url('r'), b)
  writeFileSync(join(b, 'notes.txt'), 'ben notes\n'); await run(b, 'add', '--all'); await run(b, 'commit', '-m', 'b1'); await run(b, 'push', 'origin', 'main')
  writeFileSync(join(a, 'notes.txt'), 'ana notes\n'); await run(a, 'add', '--all'); await run(a, 'commit', '-m', 'a1')
  const pull = await run(a, 'pull', 'origin', 'main')
  const state = readdirSync(join(a, '.mediagit')).filter((n) => /MERGE|ORIG/.test(n)).join(',')
  const st = await run(a, 'status')
  rec.note(`pull with a text clash: exit ${pull.code} ${flat(pull.stdout + pull.stderr).slice(-300)} | merge state: ${state || 'none'} | notes.txt ${JSON.stringify(readFileSync(join(a, 'notes.txt'), 'utf8'))} | status: ${flat(st.stdout).slice(0, 200)}`)
  rec.check(state.includes('MERGE_HEAD'), 'a conflicting pull leaves a merge to resolve (like `merge` does)', flat(pull.stdout + pull.stderr).slice(-200))
  rec.check(!/no tree result/i.test(pull.stdout + pull.stderr), 'the message is not the raw "Merge failed: no tree result"')
  const f = await run(a, 'fetch', 'origin')
  const viaMerge = await run(a, 'merge', 'origin/main')
  rec.note(`way out by hand: fetch exit ${f.code}; \`merge origin/main\`: exit ${viaMerge.code} ${flat(viaMerge.stdout + viaMerge.stderr).slice(-200)}; merge state now ${readdirSync(join(a, '.mediagit')).filter((n) => /MERGE/.test(n)).join(',') || 'none'}`)
  const lg = await run(a, 'log', 'origin/main')
  rec.note(`\`log origin/main\`: exit ${lg.code} ${flat(lg.stdout + lg.stderr).slice(0, 160)}`)
  const rs = await run(a, 'reset', '--hard', 'origin/main')
  rec.note(`\`reset --hard origin/main\`: exit ${rs.code} ${flat(rs.stdout + rs.stderr).slice(0, 160)}`)
  rec.check(existsSync(join(a, 'photo.jpeg')), 'files are intact')
  s.stop()
  await rec.finish()
  await closeTerm()
})
