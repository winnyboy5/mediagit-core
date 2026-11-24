import { createHash } from 'node:crypto'
import { cpSync, createReadStream, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { CORPUS, WORK } from './env'
import type { Recorder } from './rec'

// Integrity and corpus helpers. "INT" in the plan = intCheck(): fsck --full, verify, and a
// hash compare of checked-out files against the corpus originals.

const HASHES = join(WORK, 'corpus-hashes.json')
type HashDb = Record<string, { sha256: string; size: number }>
const db = (): HashDb => (existsSync(HASHES) ? (JSON.parse(readFileSync(HASHES, 'utf8')) as HashDb) : {})

export function sha256(file: string): Promise<string> {
  return new Promise((ok, fail) => {
    const h = createHash('sha256')
    createReadStream(file, { highWaterMark: 4 << 20 }).on('data', (d) => h.update(d)).on('error', fail).on('end', () => ok(h.digest('hex')))
  })
}

export function walk(dir: string, skip: (n: string) => boolean = (n) => n === '.mediagit'): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    skip(e.name) ? [] : e.isDirectory() ? walk(join(dir, e.name), skip) : [join(dir, e.name)])
}

/** Hash of a corpus original (by path relative to test-files), cached in work\corpus-hashes.json. */
export async function corpusHash(rel: string): Promise<string> {
  const d = db()
  const src = join(CORPUS, rel)
  const size = statSync(src).size
  if (d[rel]?.size === size) return d[rel]!.sha256
  d[rel] = { sha256: await sha256(src), size }
  writeFileSync(HASHES, JSON.stringify(d, null, 1))
  return d[rel]!.sha256
}

/**
 * Copy corpus files (or whole folders) into a work folder. `items` maps a path relative to
 * test-files to its destination path relative to `dest`. Refuses any destination outside work\.
 * Returns dest-relative path to the expected sha256, for hashCompare().
 */
export async function copyCorpus(dest: string, items: Record<string, string>): Promise<Record<string, string>> {
  const root = resolve(dest)
  if (!root.toLowerCase().startsWith(resolve(WORK).toLowerCase())) throw new Error(`refusing to copy outside the work folder: ${root}`)
  const want: Record<string, string> = {}
  for (const [rel, to] of Object.entries(items)) {
    const src = join(CORPUS, rel)
    const dst = join(root, to)
    mkdirSync(dirname(dst), { recursive: true })
    if (statSync(src).isDirectory()) {
      cpSync(src, dst, { recursive: true })
      for (const f of walk(src, () => false)) {
        const r = relative(src, f)
        want[join(to, r).replace(/\\/g, '/')] = await corpusHash(join(rel, r))
      }
    } else {
      cpSync(src, dst)
      want[to.replace(/\\/g, '/')] = await corpusHash(rel)
    }
  }
  return want
}

/** Compare files on disk under `dir` with expected hashes. Records one check per mismatch. */
export async function hashCompare(rec: Recorder, dir: string, want: Record<string, string>, label = 'hash compare'): Promise<boolean> {
  const bad: string[] = []
  for (const [rel, h] of Object.entries(want)) {
    const f = join(dir, rel)
    if (!existsSync(f)) bad.push(`${rel}: missing`)
    else if ((await sha256(f)) !== h) bad.push(`${rel}: bytes differ`)
  }
  return rec.check(bad.length === 0, `${label}: ${Object.keys(want).length} files equal the originals in ${dir}`, bad.slice(0, 10).join(' | '))
}

/** fsck --full + verify (+ hash compare when `want` is given). The plan's INT check. */
export async function intCheck(rec: Recorder, repo: string, want?: Record<string, string>): Promise<boolean> {
  rec.touch(repo)
  const fsck = await rec.cli(repo, ['fsck', '--full'])
  const a = rec.check(fsck.code === 0, `INT fsck --full clean: ${repo}`, fsck.code === 0 ? undefined : (fsck.stdout + fsck.stderr).slice(-500))
  const ver = await rec.cli(repo, ['verify'])
  const b = rec.check(ver.code === 0, `INT verify clean: ${repo}`, ver.code === 0 ? undefined : (ver.stdout + ver.stderr).slice(-500))
  const c = want ? await hashCompare(rec, repo, want, 'INT hash compare') : true
  return a && b && c
}

/**
 * The real round trip: delete the working copies, bring them back from the store with
 * `reset --hard HEAD`, then hash them against the originals. A plain hashCompare on files
 * that were never rewritten only proves `add` left them alone.
 */
export async function restoreCheck(rec: Recorder, repo: string, want: Record<string, string>): Promise<boolean> {
  for (const rel of Object.keys(want)) rmSync(join(repo, rel), { force: true })
  const r = await rec.cli(repo, ['reset', '--hard', 'HEAD'])
  rec.check(r.code === 0, `restore: reset --hard HEAD rewrote the deleted files (${r.ms} ms, peak ${r.peakMb.toFixed(0)} MB)`, r.code === 0 ? undefined : (r.stdout + r.stderr).slice(-400))
  return hashCompare(rec, repo, want, 'INT restored-from-store hash compare')
}
