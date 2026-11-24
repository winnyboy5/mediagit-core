import { cpSync, openSync, closeSync, readSync, writeSync, rmSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { test } from '@playwright/test'
import { Recorder, closeTerm, workDir } from './rec'
import { copyCorpus, hashCompare, intCheck, walk } from './integrity'

// I-02: the integrity checks must be able to go red. Flip one byte in a stored object, in a
// packed store, and in a working file; fsck, verify and the hash compare must each notice.
const MIX: Record<string, string> = {
  '12690118_5053480.ai': 'poster.ai',
  '1965_ac_shelby_427_cobra_sc.glb': 'cobra.glb',
  'pdf/1710.02185v1.pdf': 'paper.pdf',
  'video-variants/bbb-5s-h264.mov': 'clip.mov',
}

function flipByte(file: string) {
  const fd = openSync(file, 'r+')
  const pos = Math.floor(statSync(file).size / 2)
  const b = Buffer.alloc(1)
  readSync(fd, b, 0, 1, pos)
  b[0] = b[0]! ^ 0xff
  writeSync(fd, b, 0, 1, pos)
  closeSync(fd)
}
const biggest = (dir: string) => walk(dir, () => false).map((f) => ({ f, s: statSync(f).size })).sort((a, b) => b.s - a.s)[0]!

test('I-02 negative control: the checks can fail', async () => {
  const rec = await Recorder.start('I-02', 'negative-control')
  const root = workDir('I-02')
  rmSync(root, { recursive: true, force: true })
  const good = join(workDir('I-02'), 'good')
  await rec.cli(root, ['init', good])
  const want = await copyCorpus(good, MIX)
  await rec.cli(good, ['add', '.'])
  await rec.cli(good, ['commit', '-m', 'control'])
  rec.check(await intCheck(rec, good, want), 'baseline: the untouched repository passes every check')

  // --- A: one byte flipped in the biggest stored object
  const a = join(root, 'bad-object')
  cpSync(good, a, { recursive: true })
  const obj = biggest(join(a, '.mediagit', 'objects'))
  flipByte(obj.f)
  rec.note(`flipped one byte in ${obj.f.slice(a.length)} (${obj.s} bytes)`)
  let r = await rec.cli(a, ['fsck', '--full'])
  rec.check(r.code !== 0, 'fsck --full goes red on a corrupt object', `exit ${r.code}: ${(r.stdout + r.stderr).trim().split('\n').slice(-3).join(' / ').slice(0, 300)}`)
  r = await rec.cli(a, ['fsck', '--quick'])
  rec.note(`fsck --quick on the corrupt object: exit ${r.code}`)
  r = await rec.cli(a, ['fsck'])
  rec.check(r.code !== 0, 'plain fsck goes red on a corrupt object', `exit ${r.code}`)
  r = await rec.cli(a, ['verify'])
  rec.check(r.code !== 0, 'verify goes red on a corrupt object', `exit ${r.code}: ${(r.stdout + r.stderr).trim().split('\n').slice(-3).join(' / ').slice(0, 300)}`)
  r = await rec.cli(a, ['verify', '--quick'])
  rec.note(`verify --quick on the corrupt object: exit ${r.code}`)
  r = await rec.cli(a, ['verify', '--checksums'])
  rec.check(r.code !== 0, 'verify --checksums goes red on a corrupt object', `exit ${r.code}`)
  // restore from the corrupt store: must fail loudly or produce bytes the hash compare rejects
  for (const rel of Object.keys(want)) rmSync(join(a, rel), { force: true })
  r = await rec.cli(a, ['reset', '--hard', 'HEAD'])
  const silent: string[] = []
  const probe = await Recorder.start('I-02x', 'probe')
  const same = await hashCompare(probe, a, want, 'probe')
  if (r.code === 0 && same) silent.push('reset --hard gave exit 0 and files that match')
  rec.check(r.code !== 0 || !same, 'checkout from a corrupt store either fails or is caught by the hash compare', `reset exit ${r.code}, files equal originals: ${same}`)
  rec.check(!(r.code === 0 && !same), 'checkout never writes wrong bytes while reporting success (silent corruption)', `reset exit ${r.code}, files equal originals: ${same}: ${(r.stdout + r.stderr).trim().slice(-300)}`)

  // --- B: the same after a repack
  const b = join(root, 'bad-pack')
  cpSync(good, b, { recursive: true })
  r = await rec.cli(b, ['gc', '--repack', '--yes'])
  rec.note(`gc --repack: exit ${r.code}: ${(r.stdout + r.stderr).trim().split('\n').slice(-4).join(' / ').slice(0, 300)}`)
  rec.check(await intCheck(rec, b, want), 'the repacked repository passes every check before it is damaged')
  const pk = biggest(join(b, '.mediagit', 'objects'))
  flipByte(pk.f)
  rec.untouch(b)
  rec.note(`flipped one byte in ${pk.f.slice(b.length)} (${pk.s} bytes)`)
  r = await rec.cli(b, ['fsck', '--full'])
  rec.check(r.code !== 0, 'fsck --full goes red on a corrupt packed store', `exit ${r.code}: ${(r.stdout + r.stderr).trim().split('\n').slice(-3).join(' / ').slice(0, 300)}`)
  r = await rec.cli(b, ['verify'])
  rec.check(r.code !== 0, 'verify goes red on a corrupt packed store', `exit ${r.code}`)

  // --- C: one byte flipped in a working file
  const c = join(root, 'bad-working')
  cpSync(good, c, { recursive: true })
  flipByte(join(c, 'cobra.glb'))
  const p2 = await Recorder.start('I-02y', 'probe')
  rec.check(!(await hashCompare(p2, c, want, 'probe')), 'the hash compare goes red on a changed working file')
  r = await rec.cli(c, ['status', '--porcelain'])
  rec.check(/cobra\.glb/.test(r.stdout), 'status sees the one-byte edit (same size, new content)', r.stdout.trim())

  // --- D: a deleted object
  const d = join(root, 'missing-object')
  cpSync(good, d, { recursive: true })
  rmSync(biggest(join(d, '.mediagit', 'objects')).f)
  r = await rec.cli(d, ['fsck', '--full'])
  rec.check(r.code !== 0, 'fsck --full goes red on a missing object', `exit ${r.code}`)
  r = await rec.cli(d, ['verify'])
  rec.check(r.code !== 0, 'verify goes red on a missing object', `exit ${r.code}`)

  await rec.finish()
  await closeTerm()
})
