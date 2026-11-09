import type { FetchOpts, FetchReport, PullOpts, PullReport, PushOpts, PushReport } from '@/api'
import { formatBytes } from '@/components/format'

const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`

/** One-line toast/drawer summary for a finished transfer. */
export function summarizeTransfer(kind: 'fetch', r: FetchReport): string
export function summarizeTransfer(kind: 'pull', r: PullReport): string
export function summarizeTransfer(kind: 'push', r: PushReport): string
export function summarizeTransfer(kind: 'fetch' | 'pull' | 'push', r: FetchReport | PullReport | PushReport): string {
  if (kind === 'fetch') {
    const f = r as FetchReport
    const n = f.branches_updated + f.tags_updated
    return n === 0 ? 'Already up to date' : `Fetched ${plural(n, 'update')}, ${formatBytes(f.bytes_downloaded)}`
  }
  if (kind === 'pull') {
    const p = r as PullReport
    if (p.result === 'up_to_date') return 'Already up to date'
    if (p.result === 'dry_run') return 'Checked what a pull would change'
    return `Got latest: ${plural(p.objects_received, 'object')}, ${formatBytes(p.bytes_downloaded)}`
  }
  const s = r as PushReport
  if (s.result === 'up_to_date') return 'Nothing new to send'
  if (s.result === 'dry_run') return 'Checked what a push would send'
  if (s.result === 'deleted') return 'Removed from the remote'
  return `Sent ${plural(s.objects_sent, 'object')}, ${formatBytes(s.bytes_uploaded)}`
}

// ---- options ----------------------------------------------------------------

/** What the person has typed or ticked for the next fetch / pull / push. */
export interface SyncForm {
  fetch: { branch: string; all: boolean; prune: boolean }
  pull: { branch: string; rebase: boolean; dryRun: boolean }
  push: {
    refspecs: string
    all: boolean
    tags: boolean
    followTags: boolean
    dryRun: boolean
    overwrite: 'none' | 'lease' | 'force'
    delete: boolean
    setUpstream: boolean
    noTrack: boolean
    repair: boolean
  }
}

export const emptyForm = (): SyncForm => ({
  fetch: { branch: '', all: false, prune: false },
  pull: { branch: '', rebase: false, dryRun: false },
  push: { refspecs: '', all: false, tags: false, followTags: false, dryRun: false, overwrite: 'none', delete: false, setUpstream: false, noTrack: false, repair: false },
})

/** `main, dev:release` or one per line -> ['main', 'dev:release']. */
export const parseRefspecs = (text: string): string[] => text.split(/[\s,]+/).filter(Boolean)

/** A push that can replace or remove work on the remote; the person must agree first. */
export const pushIsRisky = (f: SyncForm['push']): boolean => f.overwrite !== 'none' || f.delete

/** The option bag for one transfer; blank fields and unticked boxes are left out. `confirmed` only ever rides along with a risky push. */
export function transferOpts(kind: 'fetch', f: SyncForm, confirmed?: boolean): FetchOpts
export function transferOpts(kind: 'pull', f: SyncForm, confirmed?: boolean): PullOpts
export function transferOpts(kind: 'push', f: SyncForm, confirmed?: boolean): PushOpts
export function transferOpts(kind: 'fetch' | 'pull' | 'push', f: SyncForm, confirmed = false): FetchOpts | PullOpts | PushOpts {
  if (kind === 'fetch') {
    const o = f.fetch
    return { ...(o.branch.trim() && !o.all ? { branch: o.branch.trim() } : {}), ...(o.all ? { all: true } : {}), ...(o.prune ? { prune: true } : {}) }
  }
  if (kind === 'pull') {
    const o = f.pull
    return { ...(o.branch.trim() ? { branch: o.branch.trim() } : {}), ...(o.rebase ? { rebase: true } : {}), ...(o.dryRun ? { dryRun: true } : {}) }
  }
  const o = f.push
  const refspecs = parseRefspecs(o.refspecs)
  return {
    ...(refspecs.length ? { refspecs } : {}),
    // Deleting names its branches and nothing else; the form hides the rest, this keeps stale ticks out.
    ...(o.delete ? { delete: true } : {
      ...(o.all ? { all: true } : {}),
      ...(o.tags ? { tags: true } : {}),
      ...(o.followTags ? { followTags: true } : {}),
    }),
    ...(o.dryRun ? { dryRun: true } : {}),
    ...(!o.delete && o.overwrite !== 'none' ? { overwrite: o.overwrite } : {}),
    ...(o.setUpstream ? { setUpstream: true } : {}),
    ...(o.noTrack && !o.setUpstream ? { noTrack: true } : {}),
    ...(o.repair ? { repair: true } : {}),
    ...(confirmed && pushIsRisky(o) ? { confirmed: true } : {}),
  }
}
