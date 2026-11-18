import type { MergeConflictReport, MergeReport } from '@/api'

export type Side = 'ours' | 'theirs'

export interface Conflict {
  path: string
  kind: string
  /** Which version the person kept; `staged` when it was already staged before this session; null until chosen. */
  resolved: Side | 'staged' | null
}

/** Why a file clashes, in plain words. */
export function conflictReason(kind: string): string {
  switch (kind) {
    case 'modify_modify':
      return 'Both changed'
    case 'add_add':
      return 'Both added'
    case 'delete_modify':
      return 'Deleted by you, changed by them'
    case 'modify_delete':
      return 'Changed by you, deleted by them'
    default:
      return 'Needs a decision'
  }
}

/** Kinds where one side has no file to keep. */
export function missingSide(kind: string): Side | null {
  return kind === 'delete_modify' ? 'ours' : kind === 'modify_delete' ? 'theirs' : null
}

/** A conflict still waiting for a choice. `state` is what the CLI reported; `resolved` is ours. */
export const toConflicts = (list: readonly MergeConflictReport[]): Conflict[] =>
  list.map((c) => ({ path: c.path, kind: c.kind, resolved: c.state === 'unresolved' ? null : 'staged' }))

const SUMMARY: Record<string, string> = {
  fast_forward: 'Fast-forwarded',
  merged: 'Merged',
  squashed: 'Squashed into one change',
  not_committed: 'Merged, not saved yet',
  up_to_date: 'Already up to date',
  aborted: 'Merge cancelled',
}

/** What a finished merge means for the person, and what to do next. */
const RESULT: Record<string, string> = {
  fast_forward: 'Fast-forwarded: your branch now matches the other one.',
  merged: 'Merged: the two branches are joined.',
  squashed: 'Squashed: the changes are in your files. Save a version to keep them.',
  not_committed: 'Merged, but not saved yet. Look over the files, then save a version.',
  up_to_date: 'Already up to date. There was nothing to merge.',
  aborted: 'Merge cancelled. Nothing was changed.',
}
export const mergeResult = (status: string): string => RESULT[status] ?? 'Merge finished.'

export function mergeSummary(r: MergeReport): string {
  if (r.status === 'conflict') {
    const n = r.conflicts.length
    return `Stopped: ${n} ${n === 1 ? 'file needs' : 'files need'} a decision`
  }
  return SUMMARY[r.status] ?? r.status
}
