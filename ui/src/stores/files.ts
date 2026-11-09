import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type LsFilesEntry } from '@/api'
import type { FileStatus } from '@/components/StatusBadge.vue'
import { buildTree, flatRows, flattenTree, type FileEntry, type Row } from '@/lib/fileTree'
import { load, save } from '@/lib/persist'
import type { SelectedFile } from './changes'
import { useChangesStore } from './changes'
import { useRepoStore } from './repo'

const STATUS: Record<LsFilesEntry['status'], FileStatus | null> = {
  new: 'untracked',
  modified: 'modified',
  deleted: 'deleted',
  unchanged: null,
  ignored: null,
}

/** ls-files entries as tree entries: only changed files carry a status. */
export function toEntries(files: LsFilesEntry[]): FileEntry[] {
  return files.map((f) => ({ path: f.path, status: STATUS[f.status], size: f.size, included: f.staged, ignored: f.status === 'ignored' }))
}

/** Entries whose path contains every word of the query (case-insensitive); all of them for a blank query. */
export function filterEntries(entries: FileEntry[], query: string): FileEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return entries
  return entries.filter((e) => {
    const p = e.path.toLowerCase()
    return words.every((w) => p.includes(w))
  })
}

/** "All files" explorer state: the whole working folder, a find box, one selected file. */
export const useFilesStore = defineStore('files', () => {
  const repo = useRepoStore()
  const changes = useChangesStore()

  const entries = shallowRef<FileEntry[]>([])
  const loading = ref(false)
  const loaded = ref(false)
  const error = ref<ApiError | null>(null)
  const query = ref('')
  const showIgnored = ref<boolean>(load('mg.files.ignored', false))
  const collapsed = ref<ReadonlySet<string>>(new Set())
  /** Rows picked with the mouse or keyboard, as `files:path`. */
  const selected = shallowRef<ReadonlySet<string>>(new Set())
  let seq = 0

  watch(showIgnored, (v) => {
    save('mg.files.ignored', v)
    void refresh()
  })

  async function refresh() {
    const root = repo.path
    if (!root) return
    const mine = ++seq
    loading.value = true
    try {
      const r = await api.lsFiles(root, showIgnored.value)
      if (mine !== seq) return
      entries.value = toEntries(r.files)
      error.value = null
      loaded.value = true
    } catch (e) {
      if (mine === seq) error.value = toApiError(e)
    } finally {
      if (mine === seq) loading.value = false
    }
  }

  // The working folder changes under us: every status refresh re-reads it.
  watch(() => repo.status, () => loaded.value && void refresh())
  watch(
    () => repo.path,
    () => {
      entries.value = []
      loaded.value = false
      selected.value = new Set()
      collapsed.value = new Set()
    },
  )

  const shown = computed(() => filterEntries(entries.value, query.value))
  const rows = computed<Row[]>(() =>
    changes.mode === 'tree' ? flattenTree(buildTree(shown.value), query.value.trim() ? new Set() : collapsed.value) : flatRows(shown.value),
  )
  const changedCount = computed(() => entries.value.filter((e) => e.status).length)

  const key = (p: string) => `files:${p}`

  function select(path: string, mods: { ctrl?: boolean; shift?: boolean } = {}) {
    const next = new Set(mods.ctrl || mods.shift ? selected.value : [])
    if (!next.delete(key(path)) || !(mods.ctrl || mods.shift)) next.add(key(path))
    selected.value = next
  }

  function toggleFolder(path: string) {
    const next = new Set(collapsed.value)
    if (!next.delete(path)) next.add(path)
    collapsed.value = next
  }

  const clearSelection = () => (selected.value = new Set())

  const selectedFiles = computed<SelectedFile[]>(() => {
    const byPath = new Map(entries.value.map((e) => [e.path, e]))
    const out: SelectedFile[] = []
    for (const k of selected.value) {
      const e = byPath.get(k.slice('files:'.length))
      if (e) out.push({ path: e.path, status: e.status ?? undefined, group: e.included ? 'staged' : e.status ? 'unstaged' : undefined, size: e.size ?? null })
    }
    return out
  })

  return { entries, loading, loaded, error, query, showIgnored, collapsed, selected, rows, changedCount, selectedFiles, refresh, select, toggleFolder, clearSelection }
})
