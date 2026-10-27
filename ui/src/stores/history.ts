import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import {
  api,
  toApiError,
  type ApiError,
  type LogCommitReport,
  type LogRefReport,
  type LogReport,
  type ShowCommitReport,
  type ShowTreeEntryReport,
} from '@/api'
import { createGraphLayout, type GraphRow } from '@/lib/graphLayout'
import { ancestorsOf, decorate, headOid, matchesQuery } from '@/lib/history'
import { load, save } from '@/lib/persist'
import { useRepoStore } from './repo'

export const PAGE_SIZE = 200
/** With a filter on, keep paging until at least this many rows match (or history ends). */
const MIN_VISIBLE = 40

/** A file of the selected version, for the inspector. */
export interface VersionFile {
  path: string
  size: number | null
  /** The version has no such file (it was deleted in it). */
  deleted: boolean
}

/** History view state: paged commit list, graph rows, filters, selected version. */
export const useHistoryStore = defineStore('history', () => {
  const repo = useRepoStore()

  const commits = shallowRef<LogCommitReport[]>([])
  const refs = shallowRef<LogRefReport[]>([])
  const allRows = shallowRef<GraphRow[]>([])
  let layout = createGraphLayout()
  const loading = ref(false)
  const loadingMore = ref(false)
  const loaded = ref(false)
  const done = ref(false)
  const error = ref<ApiError | null>(null)
  const moreError = ref<ApiError | null>(null)

  const query = ref('')
  const branchOnly = ref<boolean>(load('mg.history.branchOnly', false))
  watch(branchOnly, (v) => save('mg.history.branchOnly', v))

  const selectedOid = ref<string | null>(null)
  const detail = shallowRef<ShowCommitReport | null>(null)
  const detailLoading = ref(false)
  const detailError = ref<ApiError | null>(null)
  const tree = shallowRef<ShowTreeEntryReport[] | null>(null)
  const treeLoading = ref(false)
  const treeError = ref<ApiError | null>(null)
  const file = shallowRef<VersionFile | null>(null)

  let token = 0
  let detailToken = 0
  let treeToken = 0

  const searching = computed(() => query.value.trim().length > 0)
  const decorations = computed(() => decorate(refs.value))
  const head = computed(() => headOid(refs.value))
  const keep = computed(() => (branchOnly.value ? ancestorsOf(commits.value, head.value) : null))

  const visible = computed(() => {
    const k = keep.value
    if (!k && !searching.value) return commits.value
    return commits.value.filter((c) => (!k || k.has(c.oid)) && matchesQuery(c, query.value))
  })
  /** Graph rows for `visible`; none while searching (a graph of scattered matches misleads). */
  const rows = computed<GraphRow[]>(() => {
    if (searching.value) return []
    return keep.value ? createGraphLayout().push(visible.value) : allRows.value
  })
  const selected = computed(() => commits.value.find((c) => c.oid === selectedOid.value) ?? null)
  const inspectFile = computed(() => (file.value && selectedOid.value ? { ...file.value, rev: selectedOid.value } : null))

  function append(r: LogReport, replace: boolean) {
    if (replace) {
      layout = createGraphLayout()
      commits.value = r.commits
      allRows.value = layout.push(r.commits)
    } else {
      commits.value = [...commits.value, ...r.commits]
      allRows.value = [...allRows.value, ...layout.push(r.commits)]
    }
    refs.value = r.refs
    done.value = r.commits.length < PAGE_SIZE
  }

  /** (Re)load the first page. Old rows stay on screen until the new page arrives. */
  async function reload() {
    if (!repo.path) return
    const mine = ++token
    loading.value = true
    loadingMore.value = false // a superseded loadMore no longer clears it (token mismatch)
    error.value = null
    moreError.value = null
    try {
      const r = await api.log(repo.path, 0, PAGE_SIZE, true)
      if (mine !== token) return
      append(r, true)
      loaded.value = true
      if (!selectedOid.value || !commits.value.some((c) => c.oid === selectedOid.value)) {
        void select(commits.value[0]?.oid ?? null)
      }
      fillIfSparse()
    } catch (e) {
      if (mine !== token) return
      error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  async function loadMore() {
    if (!repo.path || done.value || loading.value || loadingMore.value || !loaded.value) return
    const mine = token
    loadingMore.value = true
    moreError.value = null
    try {
      const r = await api.log(repo.path, commits.value.length, PAGE_SIZE, true)
      if (mine !== token) return
      append({ commits: r.commits, refs: r.refs }, false)
      fillIfSparse()
    } catch (e) {
      if (mine === token) moreError.value = toApiError(e)
    } finally {
      if (mine === token) loadingMore.value = false
    }
  }

  // A filter that matches few rows never scrolls, so scrolling cannot ask for more.
  function fillIfSparse() {
    if ((searching.value || branchOnly.value) && visible.value.length < MIN_VISIBLE && !done.value) queueMicrotask(() => void loadMore())
  }
  watch([query, branchOnly], fillIfSparse)

  async function select(oid: string | null) {
    if (oid === selectedOid.value && (detail.value || detailLoading.value)) return
    selectedOid.value = oid
    file.value = null
    tree.value = null
    treeError.value = null
    treeToken++
    treeLoading.value = false
    detail.value = null
    detailError.value = null
    const mine = ++detailToken
    if (!oid || !repo.path) {
      detailLoading.value = false
      return
    }
    detailLoading.value = true
    try {
      const d = await api.show(repo.path, oid)
      if (mine === detailToken) detail.value = d
    } catch (e) {
      if (mine === detailToken) detailError.value = toApiError(e)
    } finally {
      if (mine === detailToken) detailLoading.value = false
    }
  }

  /** The full file list at the selected version (fetched on demand; can be large). */
  async function loadTree() {
    const oid = selectedOid.value
    if (!oid || !repo.path || tree.value || treeLoading.value) return
    const mine = ++treeToken
    treeLoading.value = true
    treeError.value = null
    try {
      const d = await api.show(repo.path, oid, true)
      if (mine === treeToken) tree.value = d.tree ?? []
    } catch (e) {
      if (mine === treeToken) treeError.value = toApiError(e)
    } finally {
      if (mine === treeToken) treeLoading.value = false
    }
  }

  function selectFile(f: VersionFile | null) {
    file.value = f
  }

  /** Step through the visible list (keyboard). */
  function step(delta: number) {
    const list = visible.value
    if (!list.length) return
    const i = list.findIndex((c) => c.oid === selectedOid.value)
    const next = Math.max(0, Math.min(list.length - 1, (i < 0 ? 0 : i + delta)))
    void select(list[next]!.oid)
    if (next >= list.length - 20) void loadMore()
  }

  function reset() {
    token++
    detailToken++
    treeToken++
    commits.value = []
    refs.value = []
    allRows.value = []
    layout = createGraphLayout()
    loaded.value = false
    loading.value = false
    loadingMore.value = false
    done.value = false
    error.value = null
    moreError.value = null
    selectedOid.value = null
    detail.value = null
    tree.value = null
    file.value = null
    query.value = ''
  }

  watch(
    () => repo.path,
    () => reset(),
  )

  return {
    commits,
    refs,
    visible,
    rows,
    decorations,
    head,
    loading,
    loadingMore,
    loaded,
    done,
    error,
    moreError,
    query,
    searching,
    branchOnly,
    selectedOid,
    selected,
    detail,
    detailLoading,
    detailError,
    tree,
    treeLoading,
    treeError,
    file,
    inspectFile,
    reload,
    loadMore,
    select,
    loadTree,
    selectFile,
    step,
    reset,
  }
})
