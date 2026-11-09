import { defineStore } from 'pinia'
import { computed, reactive, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type StatusReport } from '@/api'
import type { FileStatus } from '@/components/StatusBadge.vue'
import { buildTree, flatRows, flattenTree, type FileEntry, type Row, type TreeNode } from '@/lib/fileTree'
import { load, save } from '@/lib/persist'
import { includeAction, mergeChanges, ProgressJoin, tickOf } from '@/lib/staging'
import { useOpsStore } from './ops'
import { useRepoStore } from './repo'

export type Group = 'staged' | 'unstaged'
export type ViewMode = 'tree' | 'flat'

/** A file the inspector can describe. */
export interface SelectedFile {
  path: string
  /** Absent for a file with no change. */
  status?: FileStatus
  group?: Group
  size: number | null
}

/** Paths per `add` call, and UTF-16 units per call (separators included): well under the 32,767 Windows command-line limit. */
const ADD_BATCH = 200
const ADD_BUDGET = 24_000

/** Split paths into `add` batches capped by count and cumulative length; an oversized path gets its own batch. */
export function batchPaths(paths: string[]): string[][] {
  const out: string[][] = []
  let cur: string[] = []
  let size = 0
  for (const p of paths) {
    if (cur.length && (cur.length >= ADD_BATCH || size + p.length + 1 > ADD_BUDGET)) {
      out.push(cur)
      cur = []
      size = 0
    }
    cur.push(p)
    size += p.length + 1
  }
  if (cur.length) out.push(cur)
  return out
}

/** The Changes list is one list: selection keys carry this prefix. */
const LIST = 'changes'
export const rowKey = (path: string) => `${LIST}:${path}`

function entriesOf(s: StatusReport | null): Record<Group, FileEntry[]> {
  if (!s) return { staged: [], unstaged: [] }
  return {
    staged: s.staged.map((f) => ({ path: f.path, status: f.kind })),
    unstaged: [
      ...s.modified.map((path) => ({ path, status: 'modified' as const })),
      ...s.deleted.map((path) => ({ path, status: 'deleted' as const })),
      ...s.untracked.map((path) => ({ path, status: 'untracked' as const })),
    ],
  }
}

/** Changes view state: groups, tree/flat rows, selection, stage + commit. */
export const useChangesStore = defineStore('changes', () => {
  const repo = useRepoStore()
  const ops = useOpsStore()

  const mode = ref<ViewMode>(load<ViewMode>('mg.changes.mode', 'tree'))
  const collapsed = ref<ReadonlySet<string>>(new Set())
  const selected = shallowRef<ReadonlySet<string>>(new Set())
  const anchor = ref<string | null>(null)
  const message = ref('')
  const busy = ref<'add' | 'commit' | 'unstage' | null>(null)
  /** More options for the next stage and the next save (the person's ticks, kept until changed). */
  const addOpts = reactive({ all: false, dryRun: false, force: false, update: false, ignoreRemoval: false })
  const commitForm = reactive({ messageFile: '', all: false, include: '', author: '', date: '', allowEmpty: false, signoff: false, dryRun: false })
  /** What the last preview said; cleared by the next real action. */
  const preview = ref<string | null>(null)
  const error = ref<ApiError | null>(null)

  watch(mode, (m) => save('mg.changes.mode', m))

  const entries = computed(() => entriesOf(repo.status))
  /** Staged and not-staged files as one list; `included` = staged. */
  const list = computed(() => mergeChanges(entries.value.staged, entries.value.unstaged))
  const rows = computed<Row[]>(() => (mode.value === 'tree' ? flattenTree(buildTree(list.value), collapsed.value) : flatRows(list.value)))
  const includedCount = computed(() => list.value.filter((e) => e.included).length)

  const counts = computed(() => ({
    staged: entries.value.staged.length,
    unstaged: entries.value.unstaged.length,
    total: entries.value.staged.length + entries.value.unstaged.length,
  }))

  // Selection ------------------------------------------------------------

  /** Click semantics: plain = only this row, Ctrl = toggle, Shift = range from the anchor. */
  function select(path: string, mods: { ctrl?: boolean; shift?: boolean } = {}) {
    const key = rowKey(path)
    const list = rows.value
    const next = new Set(selected.value)
    const anchorKey = anchor.value
    if (mods.shift && anchorKey) {
      const a = list.findIndex((r) => rowKey(r.node.path) === anchorKey)
      const b = list.findIndex((r) => rowKey(r.node.path) === key)
      if (a >= 0 && b >= 0) {
        if (!mods.ctrl) next.clear()
        for (let i = Math.min(a, b); i <= Math.max(a, b); i++) next.add(rowKey(list[i]!.node.path))
      }
    } else if (mods.ctrl) {
      if (!next.delete(key)) next.add(key)
      anchor.value = key
    } else {
      next.clear()
      next.add(key)
      anchor.value = key
    }
    selected.value = next
  }

  function selectAll() {
    selected.value = new Set(rows.value.map((r) => rowKey(r.node.path)))
  }

  function clearSelection() {
    selected.value = new Set()
    anchor.value = null
  }

  function toggleFolder(path: string) {
    const next = new Set(collapsed.value)
    if (!next.delete(path)) next.add(path)
    collapsed.value = next
  }

  /** Files covered by the selection (folders expand to their files). */
  const selectedFiles = computed<SelectedFile[]>(() => {
    const out = new Map<string, SelectedFile>()
    const byPath = new Map<string, TreeNode>()
    for (const r of rows.value) byPath.set(r.node.path, r.node)
    for (const key of selected.value) {
      const node = byPath.get(key.slice(LIST.length + 1))
      if (!node) continue
      for (const f of node.isDir ? descendants(node) : [node]) {
        out.set(f.path, { path: f.path, status: f.status ?? 'modified', group: f.included ? 'staged' : 'unstaged', size: f.size })
      }
    }
    return [...out.values()]
  })

  // Drop selection entries that vanished after a refresh.
  watch(rows, (r) => {
    if (!selected.value.size) return
    const live = new Set<string>()
    for (const x of r) live.add(rowKey(x.node.path))
    const kept = [...selected.value].filter((k) => live.has(k))
    if (kept.length !== selected.value.size) selected.value = new Set(kept)
  })

  // Mutations -------------------------------------------------------------

  const adding = computed(() => ops.running.find((o) => o.kind === 'add'))

  async function stagePaths(paths: string[], label: string) {
    if (!repo.path || busy.value || !paths.length) return
    const root = repo.path
    busy.value = 'add'
    error.value = null
    preview.value = null
    let dry = 0
    try {
      await ops.track(
        { kind: 'add', label, retry: () => void stagePaths(paths, label) },
        async (report) => {
          const join = new ProgressJoin()
          for (const batch of batchPaths(paths)) {
            const r = await api.add(root, batch, { ...addOpts }, (p) => report(join.map(p)))
            join.next()
            if (addOpts.dryRun) dry += r.added + r.modified + r.deleted
          }
        },
        () => (addOpts.dryRun ? `Preview: ${dry} files would be staged` : label.replace(/^Staging/, 'Staged')),
      )
      if (addOpts.dryRun) preview.value = `Preview only: ${dry} ${dry === 1 ? 'file' : 'files'} would be staged. Nothing changed.`
      else clearSelection()
    } catch (e) {
      error.value = toApiError(e)
    } finally {
      busy.value = null
      await repo.refresh()
    }
  }

  const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`

  const nodeOf = (path: string) => rows.value.find((r) => r.node.path === path)?.node

  /** Include or exclude one row (a folder covers every file below it). */
  function toggleInclude(path: string) {
    const n = nodeOf(path)
    if (!n) return
    if (includeAction(tickOf(n)) === 'unstage') return unstagePaths([path], `Unstaging ${plural(n.count, 'file')}`)
    return stagePaths([path], `Staging ${plural(n.count - n.included, 'file')}`)
  }

  /** Include (stage) or exclude (unstage) known paths, e.g. a file picked in the explorer. */
  function setIncluded(paths: string[], on: boolean) {
    return on ? stagePaths(paths, `Staging ${plural(paths.length, 'file')}`) : unstagePaths(paths, `Unstaging ${plural(paths.length, 'item')}`)
  }

  /** Selected rows (a folder stays one path; the CLI takes everything under it). */
  const selectedPaths = () => [...selected.value].map((k) => k.slice(LIST.length + 1))
  const selectedCount = computed(() => selected.value.size)

  function includeSelected() {
    const paths = selectedPaths()
    const n = paths.reduce((t, p) => t + (nodeOf(p)?.count ?? 1) - (nodeOf(p)?.included ?? 0), 0)
    return stagePaths(paths, `Staging ${plural(n, 'file')}`)
  }

  function excludeSelected() {
    const paths = selectedPaths()
    return unstagePaths(paths, `Unstaging ${plural(paths.length, 'item')}`)
  }

  /** Include everything that is not yet included. */
  function stageAll() {
    return stagePaths(['.'], `Staging ${plural(counts.value.unstaged, 'file')}`)
  }

  /** Something to save: staged files, or an option that stages or allows an empty version. */
  const canSave = computed(() => counts.value.staged > 0 || commitForm.all || commitForm.allowEmpty || !!commitForm.include.trim())
  const needsMessage = computed(() => !commitForm.messageFile.trim())

  async function commit() {
    const text = message.value.trim()
    if (!repo.path || busy.value || (needsMessage.value && !text) || !canSave.value) return
    const root = repo.path
    busy.value = 'commit'
    error.value = null
    preview.value = null
    try {
      await ops.track(
        { kind: 'commit', label: `Save version "${text.split('\n')[0]!.slice(0, 40)}"`, retry: () => void commit() },
        async () => {
          const r = await api.commit(root, text, commitOpts())
          if (commitForm.dryRun) {
            preview.value = r.would_commit
              ? `Preview only: would save ${r.staged} files, ${r.deleted} deletions. Nothing changed.`
              : 'Preview only: nothing to save yet.'
          }
        },
        () => (commitForm.dryRun ? 'Previewed the version' : plural(counts.value.staged, 'file') + ' saved'),
      )
      if (!commitForm.dryRun) {
        message.value = ''
        clearSelection()
      }
    } catch (e) {
      error.value = toApiError(e)
    } finally {
      busy.value = null
      await repo.refresh()
    }
  }

  /** Options as the API wants them: blanks and unticked boxes left out. */
  function commitOpts() {
    const f = commitForm
    const include = f.include.split(String.fromCharCode(10)).map((x) => x.trim()).filter(Boolean)
    return {
      ...(f.messageFile.trim() ? { messageFile: f.messageFile.trim() } : {}),
      ...(f.all ? { all: true } : {}),
      ...(include.length ? { include } : {}),
      ...(f.author.trim() ? { author: f.author.trim() } : {}),
      ...(f.date.trim() ? { date: f.date.trim() } : {}),
      ...(f.allowEmpty ? { allowEmpty: true } : {}),
      ...(f.signoff ? { signoff: true } : {}),
      ...(f.dryRun ? { dryRun: true } : {}),
    }
  }

  /** Take paths out of the staging area; files stay as they are. */
  async function unstagePaths(paths: string[], label: string) {
    if (!repo.path || busy.value || !paths.length) return
    const root = repo.path
    busy.value = 'unstage'
    error.value = null
    preview.value = null
    try {
      await ops.track({ kind: 'other', label, retry: () => void unstagePaths(paths, label) }, () => api.unstage(root, paths), () => label.replace(/^Unstaging/, 'Unstaged'))
      clearSelection()
    } catch (e) {
      error.value = toApiError(e)
    } finally {
      busy.value = null
      await repo.refresh()
    }
  }

  const unstageAll = () => unstagePaths(['.'], `Unstaging ${plural(counts.value.staged, 'file')}`)

  return {
    mode,
    addOpts,
    commitForm,
    preview,
    canSave,
    needsMessage,
    unstageAll,
    toggleInclude,
    setIncluded,
    includeSelected,
    excludeSelected,
    selectedCount,
    includedCount,
    list,
    collapsed,
    selected,
    message,
    busy,
    error,
    entries,
    rows,
    counts,
    selectedFiles,
    adding,
    select,
    selectAll,
    clearSelection,
    toggleFolder,
    stageAll,
    commit,
  }
})

function descendants(n: TreeNode): TreeNode[] {
  return n.isDir ? n.children.flatMap(descendants) : [n]
}
