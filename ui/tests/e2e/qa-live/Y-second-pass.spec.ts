import { spawn } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { CLI, CLI_ENV, WORK } from './env'
import { Recorder, closeTerm } from './rec'
import { flat } from './gui'
import { copyCorpus, hashCompare, intCheck, restoreCheck, walk } from './integrity'
import { startServer } from './server'

// Second pass, CLI only: cases skipped for time in the first pass (B-11, R-04, R-12, R-10).
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
const BIG = 'video-implementation/DW0001C004_251020_113048_p1I7H.mxf' // 1.29 GB
const RAW = /panicked|RUST_BACKTRACE|os error \d+|Os \{|stack backtrace|unwrap\(\)/i
const fresh = (name: string) => { const d = join(WORK, name); rmSync(d, { recursive: true, force: true }); mkdirSync(d, { recursive: true }); return d }
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
const tail = (r: { stdout: string; stderr: string }, n = 260) => flat(r.stdout + r.stderr).slice(-n)

/** Start the CLI, kill it hard after `ms`, and say whether it was still running. */
function killAfter(cwd: string, args: string[], ms: number): Promise<{ killed: boolean; ranMs: number }> {
  return new Promise((done) => {
    const t = Date.now()
    const p = spawn(CLI, args, { cwd, env: { ...process.env, ...CLI_ENV, ...NOKEY }, windowsHide: true, stdio: 'ignore' })
    let killed = false
    const timer = setTimeout(() => { killed = p.kill() }, ms)
    p.on('close', () => { clearTimeout(timer); done({ killed, ranMs: Date.now() - t }) })
  })
}

test('B-11 sparse checkout', async () => {
  const rec = await Recorder.start('B-11', 'sparse-checkout')
  const repo = fresh('B-11')
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY })
  const want = await copyCorpus(repo, {
    'freepik__talk__71826.jpeg': 'assets/textures/talk.jpeg',
    'creative-pairs': 'assets/pairs',
  })
  mkdirSync(join(repo, 'docs'), { recursive: true })
  writeFileSync(join(repo, 'docs', 'readme.txt'), 'docs v1\n')
  writeFileSync(join(repo, 'top.txt'), 'top v1\n')
  await run('init'); await run('add', '--all'); const c = await run('commit', '-m', 'v1')
  rec.check(c.code === 0, 'setup: v1 saved', tail(c))
  const pairs = Object.keys(want).filter((k) => k.startsWith('assets/pairs/'))
  rec.note(`tree: 1 texture, ${pairs.length} files under assets/pairs, docs/readme.txt, top.txt`)

  const list0 = await run('sparse-checkout', 'list')
  rec.note(`list before any set: exit ${list0.code} ${tail(list0, 160)}`)

  const set = await run('sparse-checkout', 'set', 'assets/textures')
  rec.check(set.code === 0, 'sparse-checkout set assets/textures succeeds', tail(set))
  rec.check(existsSync(join(repo, 'assets/textures/talk.jpeg')), 'in-cone file is still on disk')
  const left = pairs.filter((k) => existsSync(join(repo, k)))
  rec.check(left.length === 0, 'out-of-cone files are removed from disk', `${left.length} of ${pairs.length} still present`)
  rec.check(!existsSync(join(repo, 'docs/readme.txt')), 'out-of-cone docs/readme.txt is removed')
  rec.note(`top-level top.txt after set: ${existsSync(join(repo, 'top.txt')) ? 'kept' : 'removed'}`)
  const st = await run('status', '--porcelain')
  rec.check(!/^\s*D\s/m.test(st.stdout), 'status does not report sparse-excluded files as deleted', tail(st, 300))
  const stm = await rec.cli(repo, ['status', '--machine'], { env: NOKEY, quietFrame: true })
  rec.check(stm.stdout.trimStart().startsWith('{'), 'status --machine is still pure JSON under sparse checkout', stm.stdout.slice(0, 120))
  const list1 = await run('sparse-checkout', 'list')
  rec.check(/assets\/textures/.test(list1.stdout), 'list shows the active pattern', tail(list1, 160))

  // A new version while sparse must not drop the excluded files from history.
  writeFileSync(join(repo, 'assets/textures/note.txt'), 'added while sparse\n')
  await run('add', '--all'); const c2 = await run('commit', '-m', 'v2 while sparse')
  rec.check(c2.code === 0, 'a version can be saved while sparse', tail(c2))
  const ls = await rec.cli(repo, ['ls-tree', '-r', 'HEAD'], { env: NOKEY, quietFrame: true })
  const lsText = ls.stdout + ls.stderr
  if (ls.code === 0) rec.check(/docs\/readme\.txt/.test(lsText) && /assets\/pairs\//.test(lsText), 'v2 still holds the excluded files (add --all did not record them as deleted)', flat(lsText).slice(0, 300))
  else rec.note(`ls-tree not available (exit ${ls.code}); excluded files are checked through disable below`)

  // Branch switch must not materialize excluded files.
  await run('branch', 'create', 'side'); const sw = await run('checkout', 'side')
  rec.note(`branch create + checkout side: exit ${sw.code} ${tail(sw, 160)}`)
  rec.check(pairs.every((k) => !existsSync(join(repo, k))), 'a branch switch does not materialize excluded files')
  await run('checkout', 'main')

  // Guard: a locally modified file that the new cone would exclude.
  writeFileSync(join(repo, 'assets/textures/note.txt'), 'edited, not saved\n')
  const guard = await run('sparse-checkout', 'set', 'docs')
  rec.check(guard.code !== 0, 'set refuses to exclude a locally modified file', tail(guard))
  rec.check(existsSync(join(repo, 'assets/textures/note.txt')) && readFileSync(join(repo, 'assets/textures/note.txt'), 'utf8').startsWith('edited'), 'the modified file is untouched after the refusal')
  rec.check(!RAW.test(guard.stdout + guard.stderr), 'the refusal is in plain words', tail(guard))
  await run('reset', '--hard', 'HEAD')

  // Pattern mode.
  const pat = await run('sparse-checkout', 'set', '--patterns', '*.txt')
  rec.note(`set --patterns *.txt: exit ${pat.code} ${tail(pat, 200)}`)
  if (pat.code === 0) {
    rec.check(existsSync(join(repo, 'docs/readme.txt')) && existsSync(join(repo, 'top.txt')), 'pattern mode materializes matching files')
    rec.check(!existsSync(join(repo, 'assets/textures/talk.jpeg')), 'pattern mode removes non-matching files')
  }

  const dis = await run('sparse-checkout', 'disable')
  rec.check(dis.code === 0, 'sparse-checkout disable succeeds', tail(dis))
  await hashCompare(rec, repo, want, 'after disable')
  rec.check(existsSync(join(repo, 'docs/readme.txt')) && readFileSync(join(repo, 'docs/readme.txt'), 'utf8') === 'docs v1\n', 'docs/readme.txt is back with its bytes')
  const d = await run('diff', 'HEAD~1', 'HEAD')
  rec.check(!/deleted:/.test(d.stdout), 'BUG-041: the version saved while sparse must not delete the excluded files', flat(d.stdout).slice(0, 400))
  const st2 = await run('status', '--porcelain')
  rec.check(st2.stdout.trim() === '', 'status is clean after disable', tail(st2, 300))
  await intCheck(rec, repo)
  await rec.finish(); await closeTerm()
})

test('R-04 kill the CLI mid-add and mid-restore', async () => {
  const rec = await Recorder.start('R-04', 'kill-cli-mid-op')
  const repo = fresh('R-04')
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY })
  const want = await copyCorpus(repo, { [BIG]: 'shot.mxf', 'freepik__talk__71826.jpeg': 'still.jpeg' })
  await run('init'); await run('add', 'still.jpeg'); await run('commit', '-m', 'v1 still')

  const k1 = await killAfter(repo, ['add', 'shot.mxf'], 1200)
  rec.check(k1.killed, `add of 1.29 GB was killed while running (after ${k1.ranMs} ms)`, k1.killed ? undefined : 'it finished before the kill; the case proves nothing')
  rec.note(`leftovers in .mediagit after the kill: ${readdirSync(join(repo, '.mediagit')).filter((n) => /lock|tmp|partial/i.test(n)).join(', ') || '(none at top level)'}`)
  const st = await run('status')
  rec.check(st.code === 0 && !RAW.test(st.stdout + st.stderr), 'status works after the kill', tail(st))
  const fs1 = await run('fsck', '--full')
  rec.note(`fsck right after the kill: exit ${fs1.code} ${tail(fs1, 200)}`)
  const add2 = await run('add', 'shot.mxf')
  rec.check(add2.code === 0, `add again succeeds (${add2.ms} ms)`, tail(add2))
  const c = await run('commit', '-m', 'v2 shot')
  rec.check(c.code === 0, 'the version saves after the retry', tail(c))
  await restoreCheck(rec, repo, want)

  // Kill a restore that is rewriting the big file.
  rmSync(join(repo, 'shot.mxf'))
  const k2 = await killAfter(repo, ['reset', '--hard', 'HEAD'], 700)
  rec.check(k2.killed, `reset --hard was killed while running (after ${k2.ranMs} ms)`, k2.killed ? undefined : 'it finished before the kill')
  const half = existsSync(join(repo, 'shot.mxf')) ? statSync(join(repo, 'shot.mxf')).size : -1
  rec.note(`shot.mxf after the killed restore: ${half < 0 ? 'absent' : `${half} bytes of 1291051092`}; stray temp files: ${walk(repo).filter((f) => /\.tmp|\.partial|~$/i.test(f)).length}`)
  const st2 = await run('status')
  rec.check(st2.code === 0, 'status works after the killed restore', tail(st2))
  rec.check(half < 0 || half === 1291051092 || /modified|deleted|shot\.mxf/i.test(st2.stdout), 'a half-written file is reported as changed, not as clean', tail(st2))
  await restoreCheck(rec, repo, want)
  await intCheck(rec, repo, want)
  await rec.finish(); await closeTerm()
})

test('R-12 file locked by another program', async () => {
  const rec = await Recorder.start('R-12', 'file-locked')
  const repo = fresh('R-12')
  const run = (...args: string[]) => rec.cli(repo, args, { env: NOKEY })
  const want = await copyCorpus(repo, { 'freepik__talk__71826.jpeg': 'a.jpeg' })
  writeFileSync(join(repo, 'b.txt'), 'b v1\n'); writeFileSync(join(repo, 'c.txt'), 'c v1\n')
  await run('init'); await run('add', '--all'); await run('commit', '-m', 'v1')

  const hold = (file: string, secs: number) => {
    const ps = `$f=[IO.File]::Open('${file}','Open','ReadWrite','None'); Start-Sleep ${secs}; $f.Close()`
    return spawn('powershell', ['-NoProfile', '-Command', ps], { windowsHide: true, stdio: 'ignore' })
  }

  // add while one of three changed files is locked.
  writeFileSync(join(repo, 'b.txt'), 'b v2\n'); writeFileSync(join(repo, 'c.txt'), 'c v2\n'); writeFileSync(join(repo, 'new.txt'), 'new\n')
  let h = hold(join(repo, 'b.txt'), 25); await sleep(3500)
  const add = await run('add', '--all')
  rec.note(`add --all with b.txt locked: exit ${add.code} ${tail(add, 300)}`)
  rec.check(add.code !== 0 || /b\.txt/.test(add.stdout + add.stderr), 'add does not silently skip a locked file', tail(add))
  rec.check(!RAW.test(add.stdout + add.stderr), 'the message is in plain words', tail(add))
  rec.check(/b\.txt/.test(add.stdout + add.stderr), 'the message names the locked file', tail(add))
  const st = await run('status', '--porcelain')
  rec.note(`status after the failed add: ${flat(st.stdout)}`)
  h.kill(); await sleep(800)
  const add2 = await run('add', '--all'); const c2 = await run('commit', '-m', 'v2')
  rec.check(add2.code === 0 && c2.code === 0, 'after the lock is released, add and commit succeed', tail(c2))

  // Restore over a locked file.
  writeFileSync(join(repo, 'b.txt'), 'b edited\n'); writeFileSync(join(repo, 'c.txt'), 'c edited\n')
  h = hold(join(repo, 'b.txt'), 25); await sleep(3500)
  const rs = await run('reset', '--hard', 'HEAD')
  rec.note(`reset --hard with b.txt locked: exit ${rs.code} ${tail(rs, 300)}`)
  rec.check(rs.code !== 0, 'reset --hard reports failure when it cannot rewrite a locked file', tail(rs))
  rec.check(!RAW.test(rs.stdout + rs.stderr) && /b\.txt/.test(rs.stdout + rs.stderr), 'the message is plain and names the file', tail(rs))
  h.kill(); await sleep(800)
  const bNow = readFileSync(join(repo, 'b.txt'), 'utf8'); const cNow = readFileSync(join(repo, 'c.txt'), 'utf8')
  rec.note(`after the failed reset: b.txt="${bNow.trim()}" c.txt="${cNow.trim()}"`)
  const st2 = await run('status', '--porcelain')
  rec.check(bNow === 'b v2\n' || /b\.txt/.test(st2.stdout), 'a file the reset could not rewrite still shows as changed', flat(st2.stdout))
  const rs2 = await run('reset', '--hard', 'HEAD')
  rec.check(rs2.code === 0 && readFileSync(join(repo, 'b.txt'), 'utf8') === 'b v2\n' && readFileSync(join(repo, 'c.txt'), 'utf8') === 'c v2\n', 'reset --hard again, lock released, restores both files', tail(rs2))
  await intCheck(rec, repo, want)
  await rec.finish(); await closeTerm()
})

test('R-10 forced errors read in plain words', async () => {
  const rec = await Recorder.start('R-10', 'forced-errors')
  const root = fresh('R-10')
  const repo = join(root, 'repo'); mkdirSync(repo)
  const run = (cwd: string, ...args: string[]) => rec.cli(cwd, args, { env: NOKEY })
  const plain = (r: { code: number | null; stdout: string; stderr: string }, what: string, mustMatch: RegExp) => {
    const text = r.stdout + r.stderr
    rec.check(r.code !== 0, `${what}: exits non-zero`, `exit ${r.code}`)
    rec.check(!RAW.test(text.split('\n').filter((l) => !/^\s*Details:/.test(l)).join('\n')), `${what}: no raw internals in the headline (the Details line may carry raw text)`, tail(r, 300))
    rec.check(mustMatch.test(text), `${what}: says what went wrong`, tail(r, 300))
  }
  writeFileSync(join(repo, 'a.txt'), 'a v1\n')
  await run(repo, 'init'); await run(repo, 'add', '--all'); await run(repo, 'commit', '-m', 'v1')

  const outside = join(root, 'plain'); mkdirSync(outside)
  plain(await run(outside, 'status'), 'status outside a repository', /not a (mediagit )?repository|no repository/i)
  plain(await run(outside, 'log'), 'log outside a repository', /not a (mediagit )?repository|no repository/i)
  rec.check(!existsSync(join(outside, '.mediagit')), 'no .mediagit is created in a plain folder by status or log')
  plain(await run(repo, 'commit', '-m', 'empty'), 'commit with nothing staged', /nothing|no changes/i)
  plain(await run(repo, 'checkout', 'no-such-branch'), 'checkout of an unknown branch', /no-such-branch|not found|unknown/i)
  plain(await run(repo, 'add', 'missing.psd'), 'add of a missing file', /missing\.psd|not found|no such/i)
  plain(await run(repo, 'merge', 'no-such-branch'), 'merge of an unknown branch', /no-such-branch|not found|unknown/i)
  plain(await run(repo, 'reset', '--hard', 'deadbeef'), 'reset to an unknown version', /deadbeef|not found|unknown|invalid/i)
  plain(await run(repo, 'push', 'origin', 'main'), 'push with no remote set', /remote|origin/i)

  // Server down.
  const s = await startServer(join(root, 'srv'), ['r'])
  await run(repo, 'remote', 'add', 'origin', s.url('r'))
  const ok = await run(repo, 'push', '-u', 'origin', 'main')
  rec.check(ok.code === 0, 'setup: push to the live server works', tail(ok))
  s.stop(); await sleep(1500)
  writeFileSync(join(repo, 'a.txt'), 'a v2\n'); await run(repo, 'add', '--all'); await run(repo, 'commit', '-m', 'v2')
  for (const cmd of ['push', 'pull', 'fetch'] as const) {
    const r = await run(repo, cmd, 'origin', 'main')
    plain(r, `${cmd} with the server down (${r.ms} ms)`, /connect|reach|refused|unavailable|offline|server/i)
    rec.check(r.ms < 60_000, `${cmd} with the server down gives up within 60 s`, `${r.ms} ms`)
  }
  plain(await run(root, 'clone', s.url('r'), join(root, 'c1')), 'clone with the server down', /connect|reach|refused|unavailable|offline|server/i)
  rec.check(!existsSync(join(root, 'c1', '.mediagit')), 'a failed clone leaves no half-made repository')

  // Read-only working file must still be restored by reset --hard, or fail loudly.
  writeFileSync(join(repo, 'a.txt'), 'a edited\n'); chmodSync(join(repo, 'a.txt'), 0o444)
  const ro = await run(repo, 'reset', '--hard', 'HEAD')
  const now = readFileSync(join(repo, 'a.txt'), 'utf8')
  rec.note(`reset --hard over a read-only file: exit ${ro.code}, a.txt="${now.trim()}" ${tail(ro, 200)}`)
  rec.check((ro.code === 0 && now === 'a v2\n') || (ro.code !== 0 && /a\.txt/.test(ro.stdout + ro.stderr)), 'reset --hard over a read-only file either restores it or fails naming the file', tail(ro))
  rec.check(!RAW.test(ro.stdout + ro.stderr), 'read-only failure text has no raw internals', tail(ro))
  chmodSync(join(repo, 'a.txt'), 0o666)

  // Corrupt config.
  const cfg = join(repo, '.mediagit', 'config.toml')
  if (existsSync(cfg)) {
    const keep = readFileSync(cfg, 'utf8'); writeFileSync(cfg, `${keep}\n[broken\nx = = 1\n`)
    plain(await run(repo, 'status'), 'status with a broken config.toml', /config/i)
    writeFileSync(cfg, keep)
  } else rec.note(`no config.toml at ${cfg}; broken-config step skipped`)
  await intCheck(rec, repo)
  await rec.finish(); await closeTerm()
})
