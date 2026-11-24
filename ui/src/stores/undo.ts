import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api, toApiError, type RestoreFailure, type RestoreReport } from '@/api'
import { shortOid } from '@/lib/history'
import { useAction } from './action'
import { useHistoryStore } from './history'
import { useRepoStore } from './repo'

export type Pending =
  | { kind: 'reset'; mode: 'soft' | 'mixed' | 'hard'; rev: string }
  | { kind: 'revert'; rev: string; summary: string; also?: string[]; noCommit?: boolean; message?: string }
  | { kind: 'restore'; rev: string; path: string }
  /** Changes view: throw away unsaved changes to these files; `isNew` ones are deleted. */
  | { kind: 'discard'; files: DiscardFile[] }

export interface DiscardFile {
  path: string
  size: number | null
  /** Never saved in any version: discarding deletes it. */
  isNew: boolean
}

/**
 * Undo actions on versions and files. `ask` opens the matching confirm dialog;
 * `confirm` runs it. Nothing here runs without going through a dialog.
 */
export const useUndoStore = defineStore('undo', () => {
  const history = useHistoryStore()
  const { busy, error, running, run } = useAction(() => history.reload())

  /** What the open dialog is about; kept until the next `ask` so a confirm click can still read it. */
  const pending = ref<Pending | null>(null)
  const open = ref(false)
  /** A revert that stopped on clashes: its files, until continued or abandoned. */
  const revertConflicts = ref<string[]>([])
  /** Files the last discard could not touch (for example, open in another program). */
  const discardFailed = ref<RestoreFailure[]>([])

  function ask(p: Pending) {
    error.value = null
    pending.value = p
    open.value = true
  }

  async function confirm() {
    const p = pending.value
    if (!p) return false
    open.value = false
    if (p.kind === 'reset') {
      const { mode } = p
      if (mode === 'hard') {
        // HEAD is git jargon: the Changes view's "Discard" goes back to the latest version.
        const latest = p.rev === 'HEAD'
        const r = await run(
          latest ? 'Discard unsaved changes' : `Go back to ${shortOid(p.rev)} and replace files`,
          (root, report) => api.resetHard(root, p.rev, true, report),
          () => (latest ? 'Discarded unsaved changes. Files match the latest version.' : 'Files now match that version'),
        )
        return !!r
      }
      const r = await run(`Move branch back to ${shortOid(p.rev)}`, (root) => api.reset(root, p.rev, mode), () => 'Branch moved; your files were not touched')
      return !!r
    }
    if (p.kind === 'revert') {
      const revs = [p.rev, ...(p.also ?? [])]
      const opts = { noCommit: p.noCommit || undefined, message: p.message?.trim() || undefined }
      const r = await run(
        revs.length > 1 ? `Undo ${revs.length} versions` : `Undo ${shortOid(p.rev)}`,
        (root) => api.revert(root, revs, opts),
        (x) => (x.status === 'conflict' ? `Stopped: ${x.conflicts.length} files clash` : x.status === 'not_committed' ? 'Undone, not saved yet' : 'Undone with a new version'),
        (x) => x.status === 'conflict',
      )
      if (r) revertConflicts.value = r.value.status === 'conflict' ? r.value.conflicts : []
      return !!r
    }
    if (p.kind === 'discard') {
      const paths = p.files.map((f) => f.path)
      // Two calls: --delete-new only ever covers the files the dialog listed
      // as new, so a file it called "changed" is refused by the CLI rather
      // than deleted if its status was stale.
      const changed = p.files.filter((f) => !f.isNew).map((f) => f.path)
      const fresh = p.files.filter((f) => f.isNew).map((f) => f.path)
      const both = async (root: string) => {
        const a = changed.length ? await api.restore(root, changed, false) : null
        let b: RestoreReport | null = null
        try {
          b = fresh.length ? await api.restore(root, fresh, true) : null
        } catch (e) {
          // The first call already discarded its files: report that, not just the error.
          if (!a) throw e
          const message = toApiError(e).message
          b = { source: a.source, restored: [], deleted: [], unstaged: [], failed: fresh.map((path) => ({ path, message })) }
        }
        const merged = [a, b].filter((x): x is RestoreReport => !!x)
        return {
          restored: merged.flatMap((x) => x.restored),
          deleted: merged.flatMap((x) => x.deleted),
          failed: merged.flatMap((x) => x.failed),
        }
      }
      const r = await run(
        paths.length === 1 ? `Discard ${paths[0]}` : `Discard ${paths.length} files`,
        both,
        (x) => (x.failed.length ? `${x.failed.length} of ${paths.length} could not be discarded` : `Discarded ${paths.length === 1 ? paths[0] : `${paths.length} files`}`),
        (x) => x.failed.length > 0,
      )
      discardFailed.value = r?.value.failed ?? []
      await useRepoStore().refresh()
      return !!r
    }
    const r = await run(`Restore ${p.path}`, (root) => api.restoreFile(root, p.rev, p.path), () => `Restored from ${shortOid(p.rev)}`)
    return !!r
  }

  async function revertControl(action: 'continue' | 'abort' | 'skip') {
    const r = await run(
      action === 'abort' ? 'Abandon undo' : 'Continue undo',
      (root) => api.revertControl(root, action),
      (x) => (x.status === 'conflict' ? 'Still stopped' : 'Done'),
      (x) => x.status === 'conflict',
    )
    if (r) revertConflicts.value = r.value.status === 'conflict' ? r.value.conflicts : []
    return !!r
  }

  return { pending, open, busy, error, running, revertConflicts, discardFailed, ask, confirm, revertControl }
})
