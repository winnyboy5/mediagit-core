import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type StatusReport } from '@/api'
import type { FileStatus } from '@/components/StatusBadge.vue'
import { buildTree, flatRows, flattenTree, type FileEntry, type Row, type TreeNode } from '@/lib/fileTree'
import { load, save } from '@/lib/persist'
import { useOpsStore } from './ops'
import { useRepoStore } from './repo'

export type Group = 'staged' | 'unstaged'
export type ViewMode = 'tree' | 'flat'

/** A file the inspector can describe. */
export interface SelectedFile {
  path: string
  status: FileStatus
  group: Group
  size: number | null
}

/** Paths per `add` call: keeps argv far below the Windows command-line limit. */
const ADD_BATCH = 200

export const rowKey = (group: Group, path: string) => `${group}:${path}`

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
  const collapsed = ref<Record<Group, Set<string>>>({ staged: new Set(), unstaged: new Set() })
  const selected = shallowRef<ReadonlySet<string>>(new Set())
  const anchor = ref<string | null>(null)
  const message = ref('')
  const busy = ref<'add' | 'commit' | null>(null)
  const error = ref<ApiError | null>(null)

  watch(mode, (m) => save('mg.changes.mode', m))

  const entries = computed(() => entriesOf(repo.status))
  const trees = computed<Record<Group, TreeNode[]>>(() => ({
    staged: buildTree(entries.value.staged),
    unstaged: buildTree(entries.value.unstaged),
  }))

  function rowsFor(group: Group): Row[] {
    return mode.value === 'tree'
      ? flattenTree(trees.value[group], collapsed.value[group])
      : flatRows(entries.value[group])
  }
  const rows = computed<Record<Group, Row[]>>(() => ({ staged: rowsFor('staged'), unstaged: rowsFor('unstaged') }))

  const counts = computed(() => ({
    staged: entries.value.staged.length,
    unstaged: entries.value.unstaged.length,
    total: entries.value.staged.length + entries.value.unstaged.length,
  }))

  // Selection ------------------------------------------------------------

  /** Click semantics: plain = only this row, Ctrl = toggle, Shift = range from the anchor. */
  function select(group: Group, path: string, mods: { ctrl?: boolean; shift?: boolean } = {}) {
    const key = rowKey(group, path)
    const list = rows.value[group]
    const next = new Set(selected.value)
    const anchorKey = anchor.value
    if (mods.shift && anchorKey?.startsWith(group + ':')) {
      const a = list.findIndex((r) => rowKey(group, r.node.path) === anchorKey)
      const b = list.findIndex((r) => rowKey(group, r.node.path) === key)
      if (a >= 0 && b >= 0) {
        if (!mods.ctrl) next.clear()
        for (let i = Math.min(a, b); i <= Math.max(a, b); i++) next.add(rowKey(group, list[i]!.node.path))
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

  function selectAll(group: Group) {
    selected.value = new Set(rows.value[group].map((r) => rowKey(group, r.node.path)))
  }

  function clearSelection() {
    selected.value = new Set()
    anchor.value = null
  }

  function toggleFolder(group: Group, path: string) {
    const next = new Set(collapsed.value[group])
    if (!next.delete(path)) next.add(path)
    collapsed.value = { ...collapsed.value, [group]: next }
  }

  /** Files covered by the selection (folders expand to their files). */
  const selectedFiles = computed<SelectedFile[]>(() => {
    const out = new Map<string, SelectedFile>()
    for (const group of ['staged', 'unstaged'] as const) {
      const byPath = new Map<string, TreeNode>()
      for (const r of rows.value[group]) byPath.set(r.node.path, r.node)
      for (const key of selected.value) {
        if (!key.startsWith(group + ':')) continue
        const node = byPath.get(key.slice(group.length + 1))
        if (!node) continue
        for (const f of node.isDir ? descendants(node) : [node]) {
          out.set(rowKey(group, f.path), { path: f.path, status: f.status, group, size: f.size })
        }
      }
    }
    return [...out.values()]
  })

  // Drop selection entries that vanished after a refresh.
  watch(rows, (r) => {
    if (!selected.value.size) return
    const live = new Set<string>()
    for (const g of ['staged', 'unstaged'] as const) for (const x of r[g]) live.add(rowKey(g, x.node.path))
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
    try {
      await ops.track(
        { kind: 'add', label },
        async () => {
          for (let i = 0; i < paths.length; i += ADD_BATCH) await api.add(root, paths.slice(i, i + ADD_BATCH))
        },
        () => label.replace(/^Staging/, 'Staged'),
      )
      clearSelection()
    } catch (e) {
      error.value = toApiError(e)
    } finally {
      busy.value = null
      await repo.refresh()
    }
  }

  const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`

  function stageSelected() {
    const byPath = new Map(rows.value.unstaged.map((r) => [r.node.path, r.node]))
    const paths = new Set<string>()
    let n = 0
    for (const key of selected.value) {
      if (!key.startsWith('unstaged:')) continue
      const p = key.slice('unstaged:'.length)
      // A folder row stays one path; the CLI stages everything under it.
      paths.add(p)
      n += byPath.get(p)?.count ?? 1
    }
    return stagePaths([...paths], `Staging ${plural(n, 'file')}`)
  }

  function stageAll() {
    return stagePaths(['.'], `Staging ${plural(counts.value.unstaged, 'file')}`)
  }

  async function commit() {
    const text = message.value.trim()
    if (!repo.path || busy.value || !text || !counts.value.staged) return
    const root = repo.path
    busy.value = 'commit'
    error.value = null
    try {
      await ops.track({ kind: 'commit', label: `Save version "${text.split('\n')[0]!.slice(0, 40)}"` }, () => api.commit(root, text), () =>
        plural(counts.value.staged, 'file') + ' saved',
      )
      message.value = ''
      clearSelection()
    } catch (e) {
      error.value = toApiError(e)
    } finally {
      busy.value = null
      await repo.refresh()
    }
  }

  return {
    mode,
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
    stageSelected,
    stageAll,
    commit,
  }
})

function descendants(n: TreeNode): TreeNode[] {
  return n.isDir ? n.children.flatMap(descendants) : [n]
}
