import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'
import { api, toApiError, type ApiError, type StatusReport } from '@/api'
import { forget, load, save } from '@/lib/persist'
import { useRecentStore } from './recent'

const KEY = 'mg.repo'

/** The open repository and its working-copy status. */
export const useRepoStore = defineStore('repo', () => {
  const path = ref<string | null>(load<string | null>(KEY, null))
  // Shallow: a 100k-file status must not become 100k reactive proxies.
  const status = shallowRef<StatusReport | null>(null)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  let token = 0
  // The backend keeps one repo folder in the asset scope; opening another replaces it.
  let allowed: string | null = null

  function allow(p: string) {
    if (allowed === p) return
    allowed = p
    api.allowRepo(p).catch(() => {
      if (allowed === p) allowed = null
    })
  }

  async function refresh() {
    if (!path.value) return
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      const s = await api.repoStatus(path.value)
      if (mine !== token) return
      status.value = s
      // Previews load working files through the asset protocol, which needs the folder allowed.
      allow(path.value)
    } catch (e) {
      if (mine !== token) return
      status.value = null
      error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  /** Open a repository; returns false (and stays closed) when it cannot be read. */
  async function open(dir: string): Promise<boolean> {
    // Validate before touching `path`: other stores watch it and reset on change.
    let s: StatusReport
    try {
      s = await api.repoStatus(dir)
    } catch (e) {
      error.value = toApiError(e)
      return false
    }
    token++ // supersede any refresh of the previous repo
    path.value = dir
    status.value = s
    error.value = null
    loading.value = false
    allow(dir)
    save(KEY, dir)
    useRecentStore().add(dir, (status.value as StatusReport | null)?.branch.name)
    return true
  }

  function close() {
    token++
    path.value = null
    status.value = null
    error.value = null
    loading.value = false
    forget(KEY)
  }

  return { path, status, loading, error, open, refresh, close }
})
