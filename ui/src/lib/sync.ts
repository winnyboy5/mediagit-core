import type { FetchReport, PullReport, PushReport } from '@/api'
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
