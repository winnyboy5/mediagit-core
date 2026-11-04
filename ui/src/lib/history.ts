import type { LogCommitReport, LogRefReport } from '@/api'

/** Lane geometry shared with GraphCell.vue: px between lane centres, and side padding. */
export const LANE_W = 14
export const LANE_PAD = 8

/** Width of the graph column for `lanes` lanes. */
export const graphWidth = (lanes: number) => LANE_PAD * 2 + Math.max(lanes - 1, 0) * LANE_W

export interface Pill {
  name: string
  kind: 'branch' | 'tag' | 'remote'
}

const ORDER = { branch: 0, tag: 1, remote: 2 } as const

/** Branch, tag and remote labels per commit oid (branches first). HEAD is not a pill. */
export function decorate(refs: readonly LogRefReport[]): Map<string, Pill[]> {
  const out = new Map<string, Pill[]>()
  for (const r of refs) {
    if (r.kind !== 'branch' && r.kind !== 'tag' && r.kind !== 'remote') continue
    const list = out.get(r.oid) ?? []
    list.push({ name: r.name, kind: r.kind })
    out.set(r.oid, list)
  }
  for (const list of out.values()) list.sort((a, b) => ORDER[a.kind] - ORDER[b.kind] || a.name.localeCompare(b.name))
  return out
}

export function headOid(refs: readonly LogRefReport[]): string | null {
  return refs.find((r) => r.kind === 'head')?.oid ?? null
}

/** `tip` and every loaded ancestor of it. Commits arrive newest first, parents after children. */
export function ancestorsOf(commits: readonly LogCommitReport[], tip: string | null): Set<string> {
  const seen = new Set<string>()
  if (!tip) return seen
  seen.add(tip)
  for (const c of commits) if (seen.has(c.oid)) for (const p of c.parents) seen.add(p)
  return seen
}

export const shortOid = (oid: string) => oid.slice(0, 7)

/** Case-insensitive match over summary, author name, email and id prefix. */
export function matchesQuery(c: LogCommitReport, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return (
    c.summary.toLowerCase().includes(q) ||
    c.author.name.toLowerCase().includes(q) ||
    c.author.email.toLowerCase().includes(q) ||
    c.oid.startsWith(q)
  )
}

/** Full local date and time for a unix-seconds timestamp. */
export const formatWhen = (unixSeconds: number) =>
  new Date(unixSeconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
