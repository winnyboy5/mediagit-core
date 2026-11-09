import type { Progress } from '@/api'
import { etaOf, rateOf, type Op } from '@/stores/ops'
import type { FileEntry, TreeNode } from './fileTree'

export type Tick = boolean | 'indeterminate'

/** Include checkbox of a row: a file is on/off, a folder is on, off or mixed by the files below it. */
export function tickOf(n: Pick<TreeNode, 'count' | 'included'>): Tick {
  return n.included === 0 ? false : n.included >= n.count ? true : 'indeterminate'
}

/** Checking includes (stages), unchecking excludes (unstages); a mixed folder is included. */
export const includeAction = (t: Tick): 'stage' | 'unstage' => (t === true ? 'unstage' : 'stage')

/**
 * One list for staged and not-staged files. A file in both (staged, then edited again)
 * counts as included: the next save takes the staged version.
 */
export function mergeChanges(staged: FileEntry[], unstaged: FileEntry[]): FileEntry[] {
  const out = new Map<string, FileEntry>()
  for (const e of staged) out.set(e.path, { ...e, included: true })
  for (const e of unstaged) if (!out.has(e.path)) out.set(e.path, { ...e, included: false })
  return [...out.values()]
}

/** Running totals across several `add` calls: each call restarts its bars, so finished calls are folded in. */
export class ProgressJoin {
  private base = { bytes: { pos: 0, len: 0 }, items: { pos: 0, len: 0 } }
  private last = { bytes: { pos: 0, len: 0 }, items: { pos: 0, len: 0 } }

  /** Map one call's progress to whole-job progress; bars are keyed by unit so a new call reuses them. */
  map(p: Progress): Progress {
    const b = this.base[p.unit]
    this.last[p.unit] = { pos: p.pos, len: p.len ?? p.pos }
    return { ...p, id: p.unit === 'bytes' ? 0 : 1, pos: b.pos + p.pos, len: p.len == null ? null : b.len + p.len, done: false }
  }

  /** The call finished: its totals become the base of the next. */
  next() {
    for (const u of ['bytes', 'items'] as const) {
      this.base[u] = { pos: this.base[u].pos + this.last[u].pos, len: this.base[u].len + this.last[u].len }
      this.last[u] = { pos: 0, len: 0 }
    }
  }
}

export interface StagingStats {
  files: { done: number; total: number | null } | null
  bytes: { done: number; total: number | null } | null
  /** Bytes per second and seconds left; null until measured. */
  rate: number | null
  eta: number | null
  /** 0..100 of the main bar, null when its length is unknown. */
  pct: number | null
}

/** What a running `add` has measured so far (real data only: a missing bar is a missing number). */
export function stagingStats(op: Op): StagingStats {
  const bars = [...op.bars.values()]
  const bytes = bars.find((b) => b.unit === 'bytes')
  const items = bars.find((b) => b.unit === 'items')
  const main = bytes ?? items
  return {
    files: items ? { done: items.pos, total: items.len } : null,
    bytes: bytes ? { done: bytes.pos, total: bytes.len } : null,
    rate: bytes ? rateOf(bytes) : null,
    eta: bytes ? etaOf(bytes) : null,
    pct: main?.len ? Math.min(100, Math.round((main.pos / main.len) * 100)) : null,
  }
}
