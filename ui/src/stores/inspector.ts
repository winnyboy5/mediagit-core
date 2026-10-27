import { defineStore } from 'pinia'
import { reactive, watch } from 'vue'
import { api, toApiError, type ApiError, type LogCommitReport, type MediaInfo } from '@/api'
import { joinPath, type InspectFile } from '@/lib/inspect'
import { useRepoStore } from './repo'

type Status = 'loading' | 'ready' | 'error'

export interface Extraction {
  status: Status
  /** Local file the webview can load. */
  path?: string
  error?: ApiError
  pos: number
  len: number | null
}

export interface Loadable<T> {
  status: Status
  value?: T
  error?: ApiError
}

/** Working copy and every version are separate entries. */
export const fileKey = (f: Pick<InspectFile, 'path' | 'rev'>) => `${f.rev ?? 'work'}:${f.path}`

/**
 * Per-file data for the inspector: the blob on disk (extracted once per version),
 * media metadata, and the file's own history. Components call `ensure*` when the
 * selected file changes and read the maps; entries are memoized for the session.
 */
export const useInspectorStore = defineStore('inspector', () => {
  const repo = useRepoStore()
  const extractions = reactive(new Map<string, Extraction>())
  const infos = reactive(new Map<string, Loadable<MediaInfo>>())
  const histories = reactive(new Map<string, Loadable<LogCommitReport[]>>())
  // Not reactive: only `ensureInfo` waits on these.
  const blobs = new Map<string, Promise<string>>()

  function ensureExtraction(file: InspectFile) {
    const root = repo.path
    const key = fileKey(file)
    if (!root || extractions.has(key)) return
    if (file.rev === null) {
      extractions.set(key, { status: 'ready', path: joinPath(root, file.path), pos: 0, len: null })
      blobs.set(key, Promise.resolve(file.path))
      return
    }
    extractions.set(key, { status: 'loading', pos: 0, len: file.size })
    const entry = extractions.get(key)!
    const job = api
      .extract(root, file.rev, file.path, file.size, (p) => {
        entry.pos = p.pos
        entry.len = p.len ?? file.size
      })
      .then((r) => {
        entry.path = r.path
        entry.status = 'ready'
        return r.path
      })
      .catch((e) => {
        entry.error = toApiError(e)
        entry.status = 'error'
        throw e
      })
    job.catch(() => {})
    blobs.set(key, job)
  }

  function retryExtraction(file: InspectFile) {
    const key = fileKey(file)
    extractions.delete(key)
    infos.delete(key)
    blobs.delete(key)
    ensureExtraction(file)
  }

  /** Media metadata; for a version it waits for the extraction. */
  function ensureInfo(file: InspectFile) {
    const root = repo.path
    const key = fileKey(file)
    if (!root || infos.has(key)) return
    infos.set(key, { status: 'loading' })
    const entry = infos.get(key)!
    if (file.rev !== null) ensureExtraction(file)
    ;(blobs.get(key) ?? Promise.resolve(file.path))
      .then((p) => api.mediaInfo(root, p))
      .then((v) => {
        entry.value = v
        entry.status = 'ready'
      })
      .catch((e) => {
        entry.error = toApiError(e)
        entry.status = 'error'
      })
  }

  /** The last 10 versions that touched the file. */
  function ensureHistory(file: InspectFile) {
    const root = repo.path
    if (!root || histories.has(file.path)) return
    histories.set(file.path, { status: 'loading' })
    const entry = histories.get(file.path)!
    api
      .log(root, 0, 10, false, file.path)
      .then((r) => {
        entry.value = r.commits
        entry.status = 'ready'
      })
      .catch((e) => {
        entry.error = toApiError(e)
        entry.status = 'error'
      })
  }

  /** Forget everything (repo switched). */
  function clear() {
    extractions.clear()
    infos.clear()
    histories.clear()
    blobs.clear()
  }

  // The working copy changes under us: its metadata and every file history go stale.
  watch(
    () => repo.status,
    () => {
      for (const k of [...infos.keys()]) if (k.startsWith('work:')) infos.delete(k)
      for (const k of [...extractions.keys()]) if (k.startsWith('work:')) extractions.delete(k)
      histories.clear()
    },
  )
  watch(() => repo.path, clear)

  return { extractions, infos, histories, ensureExtraction, retryExtraction, ensureInfo, ensureHistory, clear }
})
