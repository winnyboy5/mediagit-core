import { execFileSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, rmSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { CLI, CLI_ENV, CORPUS, WORK } from './env'

// work/big: one repository, one saved version, three large video files (9.4 GB). Cases that need it
// (B-07, R-01, R-02, F-11, S-07 large download, S-08) call ensureBig(), so none depends on another case
// having run first and left it intact: the runner wipes work\ and F-11 renames files inside it.
export const BIG_FILES: Record<string, string> = {
  'plates/F004C003_190925_MN99.mxf': 'video-implementation/F004C003_190925_MN99.mxf',
  'plates/DW0001C001_251020_111956_c1I7H.mxf': 'video-implementation/DW0001C001_251020_111956_c1I7H.mxf',
  'edit/timing_unique_5g.mov': 'video-spike/timing_unique_5g.mov',
}

const env = { ...process.env, ...CLI_ENV, MEDIAGIT_NO_KEYRING: '1' }
const mg = (cwd: string, ...args: string[]) => execFileSync(CLI, args, { cwd, env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 1 << 28 })

function intact(dir: string): boolean {
  if (!existsSync(join(dir, '.mediagit'))) return false
  for (const [rel, src] of Object.entries(BIG_FILES)) {
    const f = join(dir, rel)
    if (!existsSync(f) || statSync(f).size !== statSync(join(CORPUS, src)).size) return false
  }
  try { return mg(dir, 'status', '--porcelain').trim() === '' } catch { return false }
}

/** Returns work/big, rebuilding it from MG_QA_TESTFILES unless it already holds the three files, saved and unmodified. */
export function ensureBig(note: (s: string) => void = () => {}): string {
  const dir = join(WORK, 'big')
  const missing = Object.values(BIG_FILES).map((s) => join(CORPUS, s)).filter((p) => !existsSync(p))
  if (missing.length) throw new Error(`work/big needs these corpus files (MG_QA_TESTFILES=${CORPUS}): ${missing.join(', ')}`)
  if (intact(dir)) return dir
  note('work/big missing or changed: rebuilding it from the corpus (init, copy 3 files, add, commit)')
  const t = Date.now()
  rmSync(dir, { recursive: true, force: true })
  mkdirSync(dir, { recursive: true })
  mg(dir, 'init')
  for (const [rel, src] of Object.entries(BIG_FILES)) {
    mkdirSync(dirname(join(dir, rel)), { recursive: true })
    cpSync(join(CORPUS, src), join(dir, rel))
  }
  mg(dir, 'add', '--all')
  mg(dir, 'commit', '-m', 'big fixture: three large plates')
  note(`work/big rebuilt in ${Math.round((Date.now() - t) / 1000)} s`)
  return dir
}
