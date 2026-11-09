import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type DiffFileReport, type DiffReport, type LogCommitReport } from '@/api'
import { hasMediaInfo, previewKind, type InspectFile } from '@/lib/inspect'
import { useInspectorStore } from './inspector'
import { useRepoStore } from './repo'

/** How many recent versions the pickers offer. */
export const PICKER_COMMITS = 100
export const WORKING = 'working'
/** The changes already set aside for the next version (`diff --cached`). */
export const STAGED = 'staged'

/** Compare: two versions, or the working folder against the latest version. */
export const useCompareStore = defineStore('compare', () => {
  const repo = useRepoStore()
  const inspector = useInspectorStore()

  const commits = shallowRef<LogCommitReport[]>([])
  const commitsLoaded = ref(false)
  const commitsError = ref<ApiError | null>(null)
  /** Version on the left; ignored (it is the latest version) while `to` is the working folder. */
  const from = ref('')
  const to = ref(WORKING)
  const report = shallowRef<DiffReport | null>(null)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  const selectedPath = ref<string | null>(null)
  let token = 0

  const staged = computed(() => to.value === STAGED)
  /** Either side-less comparison: the left side is the latest version. */
  const working = computed(() => to.value === WORKING || to.value === STAGED)
  const files = computed<DiffFileReport[]>(() => report.value?.files ?? [])
  const file = computed(() => files.value.find((f) => f.path === selectedPath.value) ?? null)
  /** The version shown as "before": the report's own, falling back to the latest. */
  const beforeRev = computed(() => report.value?.from ?? 'HEAD')

  /** The two sides of the selected file; null where that side has no file. */
  const sides = computed<{ before: InspectFile | null; after: InspectFile | null }>(() => {
    const f = file.value
    const r = report.value
    if (!f || !r) return { before: null, after: null }
    return {
      before: f.status === 'added' ? null : { path: f.path, size: f.old_size, rev: beforeRev.value },
      after: f.status === 'deleted' ? null : { path: f.path, size: f.new_size, rev: r.to },
    }
  })

  async function loadCommits() {
    if (!repo.path) return
    const root = repo.path
    commitsError.value = null
    try {
      const r = await api.log(root, 0, PICKER_COMMITS, false)
      if (root !== repo.path) return
      commits.value = r.commits
      commitsLoaded.value = true
      if (!from.value) from.value = r.commits[0]?.oid ?? ''
    } catch (e) {
      commitsError.value = toApiError(e)
    }
  }

  async function compare() {
    const root = repo.path
    if (!root || !commits.value.length) return
    if (!working.value && (!from.value || from.value === to.value)) {
      report.value = { mode: 'commits', from: from.value, to: to.value, files: [] }
      error.value = null
      return
    }
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      const r = working.value ? await api.diff(root, null, null, staged.value) : await api.diff(root, from.value, to.value)
      if (mine !== token) return
      report.value = r
      if (!r.files.some((f) => f.path === selectedPath.value)) selectedPath.value = null
    } catch (e) {
      if (mine === token) error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  /** Start fetching what the selected file needs: previews for images, metadata where a parser exists. */
  function select(path: string | null) {
    selectedPath.value = path
    const { before, after } = sides.value
    for (const side of [before, after]) {
      if (!side) continue
      // The staged copy cannot be read on its own, and the working file may differ from it:
      // showing that file as "after" would be a wrong answer, so staged files get sizes only.
      if (staged.value && side === after) continue
      if (previewKind(side.path) === 'image') inspector.ensureExtraction(side)
      if (hasMediaInfo(side.path)) inspector.ensureInfo(side)
    }
  }

  watch([from, to], () => {
    selectedPath.value = null
    void compare()
  })
  watch(
    () => repo.path,
    () => {
      token++
      commits.value = []
      commitsLoaded.value = false
      commitsError.value = null
      from.value = ''
      to.value = WORKING
      report.value = null
      error.value = null
      selectedPath.value = null
    },
  )

  return { commits, commitsLoaded, commitsError, from, to, working, staged, report, files, file, sides, loading, error, selectedPath, loadCommits, compare, select }
})
