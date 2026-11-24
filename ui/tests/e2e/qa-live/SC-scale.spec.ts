import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { CORPUS, WORK } from './env'
import { Recorder, closeTerm } from './rec'
import { flat } from './gui'
import { sha256, walk } from './integrity'
import { startServer } from './server'

// Reduced Scale tier, CLI only: SC-01 file-count curve, SC-03 history length, SC-05 concurrent clients.
const NOKEY = { MEDIAGIT_NO_KEYRING: '1' }
const row = (o: Record<string, string | number>) => Object.entries(o).map(([k, v]) => `${k}=${v}`).join(' ')

test('SC-01 File-count curve: 1k, 10k, 50k files', async () => {
  const rec = await Recorder.start('SC-01', 'file-count-curve')
  // Real small files as seeds; each copy gets a unique tail line so content differs (no free dedup).
  const seeds = walk(join(CORPUS, 'creative-pairs')).concat(walk(join(CORPUS, 'pdf'))).filter((f) => statSync(f).size < 300_000).slice(0, 40).map((f) => readFileSync(f))
  rec.note(`seed files: ${seeds.length}, ${(seeds.reduce((n, b) => n + b.length, 0) / seeds.length / 1024).toFixed(0)} KB each on average`)
  const out: Record<string, number>[] = []
  for (const n of [1000, 10_000, 50_000]) {
    const dir = join(WORK, `SC-01-${n}`); rmSync(dir, { recursive: true, force: true }); mkdirSync(dir, { recursive: true })
    const run = (...a: string[]) => rec.cli(dir, a, { quietFrame: true })
    await run('init')
    let bytes = 0
    for (let i = 0; i < n; i++) { const d = join(dir, `set${String(i % 97).padStart(2, '0')}`, `d${String(Math.floor(i / 97) % 53).padStart(2, '0')}`, `e${i % 7}`); if (i < 97 * 53 * 7) mkdirSync(d, { recursive: true }); const b = Buffer.concat([seeds[i % seeds.length]!, Buffer.from(`\n%% copy ${i}\n`)]); writeFileSync(join(d, `f${String(i).padStart(6, '0')}.bin`), b); bytes += b.length }
    const add = await run('add', '--all'); const commit = await run('commit', '-m', `${n} files`)
    const cold = await run('status'); const warm = await run('status')
    const ls = await run('ls-files'); const fsck = await run('fsck', '--full'); const gc = await run('gc')
    const one = join(dir, 'set00', 'd00', 'e0', 'f000000.bin'); writeFileSync(one, 'edited\n')
    const st1 = await run('status'); const add1 = await run('add', '--all'); const c1 = await run('commit', '-m', 'one edit')
    const sample = walk(dir).filter((_, i) => i % Math.ceil(n / 25) === 0).slice(0, 25); const want: Record<string, string> = {}
    for (const f of sample) want[f] = await sha256(f)
    for (const f of sample) rmSync(f)
    const co = await run('reset', '--hard', 'HEAD')
    let same = co.code === 0; for (const f of sample) same = same && existsSync(f) && (await sha256(f)) === want[f]
    const store = walk(join(dir, '.mediagit'), () => false).reduce((s, f) => s + statSync(f).size, 0)
    const r = { files: n, MB: Math.round(bytes / 1e6), add_s: +(add.ms / 1000).toFixed(1), add_peakMB: Math.round(add.peakMb), commit_s: +(commit.ms / 1000).toFixed(1), status_cold_ms: cold.ms, status_warm_ms: warm.ms, ls_ms: ls.ms, fsck_s: +(fsck.ms / 1000).toFixed(1), fsck_peakMB: Math.round(fsck.peakMb), gc_s: +(gc.ms / 1000).toFixed(1), edit1_status_ms: st1.ms, edit1_add_ms: add1.ms, edit1_commit_ms: c1.ms, restore25_ms: co.ms, store_MB: Math.round(store / 1e6) }
    out.push(r); rec.note(`SC-01 ${row(r)}`)
    rec.check([add, commit, cold, ls, fsck, gc, c1].every((x) => x.code === 0), `${n} files: every command succeeds`, [add, commit, cold, ls, fsck, gc, c1].map((x) => x.code).join(','))
    rec.check(same, `${n} files: 25 sampled files restore byte for byte`)
    rmSync(dir, { recursive: true, force: true })
  }
  const per = (k: string) => out.map((r) => (r[k]! / r.files!) * 1000)
  rec.note(`per 1,000 files: add ${per('add_s').map((x) => x.toFixed(2)).join(' / ')} s; warm status ${per('status_warm_ms').map((x) => x.toFixed(0)).join(' / ')} ms; fsck ${per('fsck_s').map((x) => x.toFixed(2)).join(' / ')} s`)
  rec.check(per('add_s')[2]! <= per('add_s')[0]! * 3, 'add time per file does not blow up from 1k to 50k files (under 3x)', per('add_s').map((x) => x.toFixed(2)).join(' / '))
  rec.check(per('status_warm_ms')[2]! <= per('status_warm_ms')[0]! * 3, 'warm status per file does not blow up (under 3x)', per('status_warm_ms').map((x) => x.toFixed(0)).join(' / '))
  await rec.finish(); await closeTerm()
})

test('SC-03 History length: 1,000 versions', async () => {
  const rec = await Recorder.start('SC-03', 'history-length')
  const dir = join(WORK, 'SC-03'); rmSync(dir, { recursive: true, force: true }); mkdirSync(join(dir, 'art'), { recursive: true })
  const run = (...a: string[]) => rec.cli(dir, a, { quietFrame: true })
  await run('init')
  const t0 = Date.now(); const marks: string[] = []
  for (let i = 1; i <= 1000; i++) {
    writeFileSync(join(dir, 'notes.txt'), `version ${i}\n`)
    if (i % 100 === 1) cpSync(join(CORPUS, `creative-pairs/ai/banner_v${1 + (Math.floor(i / 100) % 4)}.ai`), join(dir, 'art/banner.ai'))
    const a = await run('add', '--all'); const c = await run('commit', '-m', `v${i}`)
    if (c.code !== 0) { rec.check(false, `commit ${i} works`, flat(c.stdout + c.stderr).slice(-200)); break }
    if (i === 10 || i % 250 === 0) marks.push(`at ${i}: add ${a.ms} ms, commit ${c.ms} ms`)
  }
  rec.note(`1,000 versions made in ${((Date.now() - t0) / 1000).toFixed(0)} s | ${marks.join(' | ')}`)
  const log = await run('log', '--machine'); const log10 = await run('log', '-n', '10'); const reflog = await run('reflog'); const fsck = await run('fsck', '--full'); const gc = await run('gc')
  const n = (log.stdout.match(/"oid":/g) ?? []).length
  rec.note(`log (all): ${log.ms} ms, ${n} entries in one page, peak ${Math.round(log.peakMb)} MB | log -n 10: ${log10.ms} ms | reflog: ${reflog.ms} ms | fsck --full: ${fsck.ms} ms | gc: ${gc.ms} ms, exit ${gc.code}`)
  const old = await run('show', 'HEAD~999:notes.txt')
  rec.check(old.stdout.includes('version 1'), 'the first of 1,000 versions is still readable', flat(old.stdout + old.stderr).slice(0, 120))
  const store = walk(join(dir, '.mediagit'), () => false)
  rec.note(`store: ${store.length} files, ${(store.reduce((s, f) => s + statSync(f).size, 0) / 1e6).toFixed(1)} MB for 1,000 versions of a 1-line file plus 10 banner changes`)
  const m = marks.map((x) => Number(x.match(/commit (\d+) ms/)![1]))
  rec.check(m[m.length - 1]! <= Math.max(m[0]! * 4, 400), 'commit time at version 1,000 is not much slower than at version 10', marks.join(' | '))
  rec.check(fsck.code === 0 && gc.code === 0, 'fsck --full and gc pass on 1,000 versions')
  rmSync(dir, { recursive: true, force: true })
  await rec.finish(); await closeTerm()
})

test('SC-05 Concurrent clients', async () => {
  const rec = await Recorder.start('SC-05', 'concurrency')
  const root = join(WORK, 'SC-05'); rmSync(root, { recursive: true, force: true }); mkdirSync(root, { recursive: true })
  const s = await startServer(join(root, 'srv'), ['r'])
  const run = (cwd: string, ...a: string[]) => rec.cli(cwd, a, { quietFrame: true, env: NOKEY })
  const seed = join(root, 'seed'); mkdirSync(seed)
  await run(seed, 'init'); cpSync(join(CORPUS, 'freepik__talk__71826.jpeg'), join(seed, 'photo.jpeg')); writeFileSync(join(seed, 'notes.txt'), 'base\n'); await run(seed, 'add', '--all'); await run(seed, 'commit', '-m', 'base')
  await run(seed, 'remote', 'add', 'origin', s.url('r')); await run(seed, 'push', '-u', 'origin', 'main')
  const clients = [0, 1, 2, 3].map((i) => join(root, `c${i}`))
  for (const c of clients) await run(root, 'clone', s.url('r'), c)
  const media = ['101394-video-720.mp4', 'freepik__talk__72772.jpeg', 'Workstation_cube_lid_off.webp', 'video-variants/bbb-5s-h264.mov']
  for (const [i, c] of clients.entries()) { await run(c, 'branch', 'create', `b${i}`); await run(c, 'branch', 'switch', `b${i}`); cpSync(join(CORPUS, media[i]!), join(c, `media${i}${media[i]!.slice(media[i]!.lastIndexOf('.'))}`)); await run(c, 'add', '--all'); await run(c, 'commit', '-m', `client ${i}`) }
  const r1 = await Promise.all(clients.map((c, i) => run(c, 'push', 'origin', `b${i}`)))
  rec.note(`4 clients push 4 different branches at once: exits ${r1.map((r) => r.code).join(',')} in ${r1.map((r) => r.ms).join('/')} ms`)
  rec.check(r1.every((r) => r.code === 0), 'four pushes of different branches at the same time all succeed', r1.filter((r) => r.code !== 0).map((r) => flat(r.stdout + r.stderr).slice(-160)).join(' || '))
  const probe = join(root, 'probe'); await run(root, 'clone', s.url('r'), probe); await run(probe, 'fetch', '--all')
  let ok = true
  for (const [i] of clients.entries()) { const sw = await run(probe, 'branch', 'switch', `b${i}`); const f = join(probe, `media${i}${media[i]!.slice(media[i]!.lastIndexOf('.'))}`); ok = ok && sw.code === 0 && existsSync(f) && (await sha256(f)) === (await sha256(join(CORPUS, media[i]!))) }
  rec.check(ok, 'a fresh clone sees all four branches with their media byte for byte')
  for (const [i, c] of clients.slice(0, 2).entries()) { await run(c, 'branch', 'switch', 'main'); writeFileSync(join(c, 'notes.txt'), `client ${i} was here\n`); await run(c, 'add', '--all'); await run(c, 'commit', '-m', `main by client ${i}`) }
  const r2 = await Promise.all(clients.slice(0, 2).map((c) => run(c, 'push', 'origin', 'main')))
  rec.note(`2 clients push the same branch at once: exits ${r2.map((r) => r.code).join(',')} | ${r2.map((r) => flat(r.stdout + r.stderr).slice(-140)).join(' || ')}`)
  rec.check(r2.filter((r) => r.code === 0).length === 1, 'exactly one of two same-branch pushes wins; the other is refused cleanly', r2.map((r) => r.code).join(','))
  rec.check(r2.every((r) => !/panicked|os error/i.test(r.stdout + r.stderr)), 'the loser gets a clean message')
  const r3 = await Promise.all(clients.map((c) => run(c, 'pull', 'origin', 'main')))
  rec.note(`4 clients pull at once: exits ${r3.map((r) => r.code).join(',')}`)
  const locks = await Promise.all(clients.map((c, i) => run(c, 'lock', 'create', 'photo.jpeg', '--owner', `user${i}`)))
  rec.note(`4 clients lock the same file at once: exits ${locks.map((r) => r.code).join(',')}`)
  rec.check(locks.filter((r) => r.code === 0).length === 1, 'exactly one of four simultaneous locks on one file wins', locks.map((r) => r.code).join(','))
  const gcSrv = rec.cli(join(s.dir, 'repos', 'r'), ['gc'], { quietFrame: true })
  cpSync(join(CORPUS, '1965_ac_shelby_427_cobra_sc.glb'), join(clients[3]!, 'cobra.glb')); await run(clients[3]!, 'branch', 'switch', 'b3'); await run(clients[3]!, 'add', '--all'); await run(clients[3]!, 'commit', '-m', 'during gc')
  const [g, p] = await Promise.all([gcSrv, run(clients[3]!, 'push', 'origin', 'b3')])
  rec.note(`push while the server repository runs gc: gc exit ${g.code}, push exit ${p.code} ${flat(p.stdout + p.stderr).slice(-140)}`)
  const probe2 = join(root, 'probe2'); const cl = await run(root, 'clone', '--branch', 'b3', s.url('r'), probe2)
  rec.check(cl.code === 0 && existsSync(join(probe2, 'cobra.glb')) && (await sha256(join(probe2, 'cobra.glb'))) === (await sha256(join(CORPUS, '1965_ac_shelby_427_cobra_sc.glb'))), 'the version pushed during gc is complete on the server (clone equals the original by hash)', flat(cl.stdout + cl.stderr).slice(-160))
  const f = await run(probe2, 'fsck', '--full')
  rec.check(f.code === 0, 'fsck --full is clean on that clone')
  s.stop(); rmSync(root, { recursive: true, force: true })
  await rec.finish(); await closeTerm()
})
