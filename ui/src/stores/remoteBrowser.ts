import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type LsTreeEntryReport, type LsTreeReport } from '@/api'
import { previewKind } from '@/lib/inspect'
import { downloadTarget, joinOut, sortEntries } from '@/lib/remoteBrowse'
import { useOpsStore } from './ops'
import { useRepoStore } from './repo'
import { useSyncStore } from './sync'

/** The server's default branch, when no ref is chosen. */
export const DEFAULT_REF = 'default'

export interface Preview {
  status: 'loading' | 'ready' | 'error'
  path?: string
  pos: number
  len: number | null
  error?: ApiError
}

/** Remote Browser: one folder level at a time from a remote's ref, a preview, and single-file download. */
export const useRemoteBrowserStore = defineStore('remoteBrowser', () => {
  const repo = useRepoStore()
  const sync = useSyncStore()
  const ops = useOpsStore()

  const remoteName = ref<string | null>(null)
  const ref_ = ref(DEFAULT_REF)
  const path = ref('')
  const listing = shallowRef<LsTreeReport | null>(null)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  const selected = shallowRef<LsTreeEntryReport | null>(null)
  const preview = shallowRef<Preview | null>(null)
  const downloadError = ref<ApiError | null>(null)
  const downloading = ref(false)
  let token = 0
  let previewOp: string | null = null
  let seq = 0

  const remote = computed(() => sync.remotes.find((r) => r.name === (remoteName.value ?? sync.remote)) ?? null)
  const entries = computed(() => sortEntries(listing.value?.entries ?? []))
  const gitRef = computed(() => (ref_.value === DEFAULT_REF ? null : ref_.value))

  function cancelPreview() {
    if (previewOp) void api.cancelOp(previewOp).catch(() => {})
    previewOp = null
  }

  function select(entry: LsTreeEntryReport | null) {
    cancelPreview()
    selected.value = entry
    preview.value = null
    downloadError.value = null
    const r = remote.value
    const root = repo.path
    if (!entry || entry.kind !== 'file' || !r || !root || previewKind(entry.path) === 'none') return
    const id = `preview-${++seq}`
    previewOp = id
    preview.value = { status: 'loading', pos: 0, len: entry.size }
    api
      .downloadPreview(root, downloadTarget(r, entry.path), gitRef.value, id, (b) => {
        if (selected.value === entry) preview.value = { status: 'loading', pos: b.pos, len: b.len ?? entry.size }
      })
      .then((x) => {
        if (selected.value === entry) preview.value = { status: 'ready', path: x.path, pos: x.size, len: x.size }
      })
      .catch((e) => {
        const err = toApiError(e)
        if (selected.value === entry && err.code !== 'cancelled') preview.value = { status: 'error', pos: 0, len: null, error: err }
      })
  }

  async function open(dir: string) {
    const r = remote.value
    const root = repo.path
    if (!r || !root) return
    const mine = ++token
    loading.value = true
    error.value = null
    select(null)
    try {
      const l = await api.remoteLsTree(root, r.name, dir || null, gitRef.value)
      if (mine !== token) return
      listing.value = l
      path.value = l.path
    } catch (e) {
      if (mine === token) error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  /** Reload from the repository root (remote or version changed). */
  const load = () => open('')

  /** Save the selected file into `folder`; one cancellable op with a bytes bar. */
  async function download(folder: string) {
    const entry = selected.value
    const r = remote.value
    const root = repo.path
    if (!entry || !r || !root || downloading.value) return false
    downloading.value = true
    downloadError.value = null
    let id = ''
    try {
      await ops.track(
        { kind: 'download', label: `Download ${entry.name}`, cancel: () => api.cancelOp(id), retry: () => void download(folder) },
        (report, opId) => {
          id = opId
          return api.download(root, downloadTarget(r, entry.path), gitRef.value, joinOut(folder, entry.name), opId, report)
        },
        (d) => `Saved ${d.bytes} bytes to ${d.output}`,
      )
      return true
    } catch (e) {
      const err = toApiError(e)
      if (err.code !== 'cancelled') downloadError.value = err
      return false
    } finally {
      downloading.value = false
    }
  }

  /** The download in flight, for its progress bar. */
  const active = computed(() => ops.running.find((o) => o.kind === 'download'))

  watch(
    () => repo.path,
    () => {
      token++
      cancelPreview()
      remoteName.value = null
      ref_.value = DEFAULT_REF
      path.value = ''
      listing.value = null
      error.value = null
      selected.value = null
      preview.value = null
      downloadError.value = null
    },
  )

  return { remoteName, ref: ref_, path, listing, entries, loading, error, selected, preview, downloadError, downloading, remote, gitRef, active, select, open, load, download }
})
