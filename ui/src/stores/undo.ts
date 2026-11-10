import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '@/api'
import { shortOid } from '@/lib/history'
import { useAction } from './action'
import { useHistoryStore } from './history'

export type Pending =
  | { kind: 'reset'; mode: 'soft' | 'mixed' | 'hard'; rev: string }
  | { kind: 'revert'; rev: string; summary: string; also?: string[]; noCommit?: boolean; message?: string }
  | { kind: 'restore'; rev: string; path: string }

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
        const r = await run(`Go back to ${shortOid(p.rev)} and replace files`, (root, report) => api.resetHard(root, p.rev, true, report), () => 'Files now match that version')
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

  return { pending, open, busy, error, running, revertConflicts, ask, confirm, revertControl }
})
