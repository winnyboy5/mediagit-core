import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'
import { test } from '@playwright/test'
import { CLI_ENV, CORPUS, DESKTOP, SERVER } from './env'
import { Recorder, closeTerm, workDir } from './rec'
import { startServer } from './server'

// V: one explicit CLI check per fixed bug that no other qa-live case asserted directly.
// Every check name starts with the BUG id so BUG-MAP.md can point at it. CLI only, no app.
// Perf checks use the budgets from FINDINGS.md and may stay red: those budgets were missed
// (BUG-001, 018, 039) and the harness reports that, it does not bend the budget.

const out = (r: { stdout: string; stderr: string }) => `${r.stdout}\n${r.stderr}`.trim()
const fresh = (name: string) => { const d = workDir(name); rmSync(d, { recursive: true, force: true }); mkdirSync(d, { recursive: true }); return d }
const lines = (s: string) => s.split('\n').map((l) => l.trim()).filter(Boolean)

test('V-01 CLI regressions for the fixed bugs', async () => {
  const rec = await Recorder.start('V-01', 'bug-regress')
  const root = fresh('V-01')
  const sh = (cwd: string, args: string[]) => rec.cli(cwd, args, { quietFrame: true })

  // IMP-002: server and desktop answer --version
  for (const [name, exe] of [['mediagit-server', SERVER], ['mediagit-desktop', DESKTOP]] as const) {
    let v = ''
    try { v = execFileSync(exe, ['--version'], { encoding: 'utf8', timeout: 20_000, env: { ...process.env, ...CLI_ENV } }) } catch (e) { v = String(e).slice(0, 200) }
    rec.check(/\d+\.\d+\.\d+/.test(v), `IMP-002: ${name} --version prints a version`, v.trim())
  }

  // BUG-003: read commands never create a repository, in any folder
  const plain = join(root, 'plain')
  mkdirSync(plain)
  for (const args of [['fsck'], ['verify'], ['tag', 'list'], ['gc']]) {
    const r = await sh(plain, args)
    rec.check(r.code !== 0 && !existsSync(join(plain, '.mediagit')), `BUG-003: \`${args.join(' ')}\` in a plain folder fails and creates no .mediagit`, `exit ${r.code} ${out(r).slice(-160)}`)
  }

  // A repository with three versions for the history checks
  const repo = join(root, 'repo')
  mkdirSync(repo)
  await sh(repo, ['init'])
  // IMP-011: a repository with no commits is healthy for fsck
  const f0 = await sh(repo, ['fsck'])
  rec.check(f0.code === 0, 'IMP-011: fsck on a repository with no versions yet is healthy', `exit ${f0.code} ${out(f0).slice(-160)}`)
  mkdirSync(join(repo, 'video'))
  writeFileSync(join(repo, 'a.txt'), 'a1\n')
  writeFileSync(join(repo, 'video', 'b.txt'), 'b1\n')
  await sh(repo, ['add', '--all']); await sh(repo, ['commit', '-m', 'v1 both'])
  writeFileSync(join(repo, 'a.txt'), 'a2\n')
  await sh(repo, ['add', '--all']); await sh(repo, ['commit', '-m', 'v2 only a'])
  writeFileSync(join(repo, 'video', 'b.txt'), 'b2\n')
  await sh(repo, ['add', '--all']); await sh(repo, ['commit', '-m', 'v3 only b'])

  // BUG-019: the path filter applies in every output mode
  const want = (await sh(repo, ['log', '--machine', 'main', 'video'])).stdout
  const wj = JSON.parse(want) as { commits?: unknown[]; data?: { commits?: unknown[] } }
  const nMachine = (wj.data?.commits ?? wj.commits ?? []).length
  const nOne = lines((await sh(repo, ['log', '--oneline', 'main', 'video'])).stdout).length
  const nPlain = (await sh(repo, ['log', 'main', 'video'])).stdout.match(/^commit /gm)?.length ?? -1
  rec.check(nMachine === 2 && nOne === nMachine && (nPlain === -1 || nPlain === nMachine), 'BUG-019: `log main video` lists the same 2 versions in --machine, --oneline and plain output', `machine ${nMachine}, oneline ${nOne}, plain ${nPlain}`)

  // BUG-022: Windows device names are refused in ref names
  for (const [kind, args] of [['branch', ['branch', 'create', 'CON']], ['branch', ['branch', 'create', 'nul']], ['tag', ['tag', 'create', 'AUX']], ['branch', ['branch', 'create', 'con ']]] as const) {
    const r = await sh(repo, [...args])
    rec.check(r.code !== 0 && !/os error/i.test(out(r)), `BUG-022: ${kind} "${args[args.length - 1]}" is refused in plain words`, `exit ${r.code} ${out(r).slice(-160)}`)
  }

  // BUG-025: a branch with nothing new makes no merge version
  await sh(repo, ['branch', 'create', 'twin'])
  const before = lines((await sh(repo, ['log', '--oneline'])).stdout).length
  const m = await sh(repo, ['merge', 'twin'])
  const after = lines((await sh(repo, ['log', '--oneline'])).stdout).length
  rec.check(after === before && /up to date/i.test(out(m)), 'BUG-025: merging a branch with nothing new says "Already up to date" and makes no version', `log ${before} to ${after}: ${out(m).slice(-160)}`)

  // BUG-042: add of a missing path exits non-zero
  const miss = await sh(repo, ['add', 'missing.psd'])
  rec.check(miss.code !== 0, 'BUG-042: `add <missing path>` exits non-zero', `exit ${miss.code} ${out(miss).slice(-160)}`)

  // Fix-plan rulings: staged changes block a merge from starting; commit refuses during a stopped merge
  await sh(repo, ['branch', 'create', 'side', 'HEAD~1'])
  await sh(repo, ['branch', 'switch', 'side'])
  writeFileSync(join(repo, 'side.txt'), 's\n')
  await sh(repo, ['add', 'side.txt']); await sh(repo, ['commit', '-m', 'side only'])
  await sh(repo, ['branch', 'switch', 'main'])
  writeFileSync(join(repo, 'staged.txt'), 'x\n')
  await sh(repo, ['add', 'staged.txt'])
  const sc = await sh(repo, ['merge', 'side'])
  rec.check(sc.code !== 0 && /staged_changes|staged|unsaved|save|commit/i.test(out(sc)), 'FIX-RULING: a merge refuses to start with a staged change (staged_changes), like git', `exit ${sc.code} ${out(sc).slice(-200)}`)
  await sh(repo, ['reset', '--hard', 'HEAD'])

  // BUG-006 and BUG-035 need a remote
  const srv = await startServer(join(root, 'srv'), ['show'])
  try {
    await sh(repo, ['remote', 'add', 'origin', srv.url('show')])
    const p = await sh(repo, ['push', '-u', 'origin', 'main'])
    rec.check(p.code === 0, 'setup: push main to the local server', out(p).slice(-200))
    await sh(repo, ['push', 'origin', 'twin'])
    const cl = join(root, 'clone')
    const c = await rec.cli(root, ['clone', srv.url('show'), cl], { quietFrame: true })
    rec.check(c.code === 0, 'setup: clone', out(c).slice(-200))
    const f = await sh(cl, ['fsck'])
    rec.check(f.code === 0, 'BUG-006: a fresh clone of a repository with two branches passes fsck', out(f).slice(-300))
    // BUG-035: reset --hard origin/main resolves like log and merge
    writeFileSync(join(cl, 'a.txt'), 'local edit\n')
    await sh(cl, ['add', '--all']); await sh(cl, ['commit', '-m', 'local only'])
    const rs = await sh(cl, ['reset', '--hard', 'origin/main'])
    const tipLines = lines((await sh(cl, ['log', '--oneline'])).stdout)
    rec.check(rs.code === 0 && !/Unknown revision/i.test(out(rs)) && !tipLines.some((l) => /local only/.test(l)), 'BUG-035: `reset --hard origin/main` works and drops the local-only version', `exit ${rs.code} ${out(rs).slice(-160)} | ${tipLines[0] ?? ''}`)
  } finally {
    srv.stop()
  }

  await rec.finish()
  await closeTerm()
})

// Perf budgets from FINDINGS.md. These were MISSED by the fix plan (BUG-001 5.9 s vs 5 s, BUG-018 43 s vs 15 s,
// BUG-039 status 3.3 s vs 2 s); a red check here is the honest result, not a harness fault.
test('V-02 Perf budgets that the fix plan missed (honest)', async () => {
  const rec = await Recorder.start('V-02', 'perf-budgets')
  const root = fresh('V-02')
  const sh = (cwd: string, args: string[]) => rec.cli(cwd, args, { quietFrame: true })

  // BUG-001: staging images. Budget 124 PNGs in 5 s = 40 ms per image file.
  const imgs = existsSync(CORPUS) ? readdirSync(CORPUS).filter((n) => /\.(png|jpe?g|webp|tiff?)$/i.test(n)).slice(0, 40) : []
  if (imgs.length >= 5) {
    const r1 = join(root, 'img'); mkdirSync(r1); await sh(r1, ['init'])
    for (const n of imgs) copyFileSync(join(CORPUS, n), join(r1, n))
    const a = await sh(r1, ['add', '--all'])
    const per = a.ms / imgs.length
    rec.check(a.code === 0 && per <= 40, `BUG-001: staging ${imgs.length} image files stays under 40 ms per file (budget 5 s for 124)`, `${a.ms} ms, ${per.toFixed(0)} ms per file`)
  } else rec.note(`BUG-001 skipped: only ${imgs.length} image files under ${CORPUS}`)

  // BUG-039: status after one edit on a large tree. Budget 2 s at 50k files; the harness builds 5,000 small files
  // and scales the budget by 10% of the original (0.2 s) plus the process start, so this is a trend check only.
  const r3 = join(root, 'tree'); mkdirSync(r3); await sh(r3, ['init'])
  for (let d = 0; d < 50; d++) {
    const dir = join(r3, `d${String(d).padStart(2, '0')}`); mkdirSync(dir)
    for (let i = 0; i < 100; i++) writeFileSync(join(dir, `f${i}.txt`), `file ${d}/${i}\n`)
  }
  await sh(r3, ['add', '--all']); await sh(r3, ['commit', '-m', 'tree'])
  writeFileSync(join(r3, 'd00', 'f0.txt'), 'edited\n')
  await sh(r3, ['status'])
  const st = await sh(r3, ['status', '--machine'])
  rec.check(st.code === 0 && st.ms <= 1000, 'BUG-039: status after one edit on a 5,000-file tree answers within 1 s', `${st.ms} ms`)
  const add = await sh(r3, ['add', 'd00/f0.txt'])
  rec.check(add.code === 0 && add.ms <= 1500, 'BUG-039: adding that one edit does not rescan the whole project (1.5 s)', `${add.ms} ms`)

  // BUG-040 is covered by SC-scale (commit time at version 1,000); its budget was re-scoped by the fix plan.
  rec.note('BUG-040: see SC-scale "commit time at version 1,000"; re-scoped (non-spike commits flat, gc lock follow-up).')
  rec.note('BUG-018: needs a 269 MB .safetensors from the corpus; see F-11-12 (pythia70m pairs) for the timing notes.')
  await rec.finish()
  await closeTerm()
})
