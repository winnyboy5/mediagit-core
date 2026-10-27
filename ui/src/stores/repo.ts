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
  const allowed = new Set<string>()

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
      if (!allowed.has(path.value)) {
        allowed.add(path.value)
        api.allowRepo(path.value).catch(() => allowed.delete(path.value!))
      }
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
    const previous = { path: path.value, status: status.value }
    path.value = dir
    status.value = null
    await refresh()
    if (error.value) {
      const failure = error.value
      path.value = previous.path
      status.value = previous.status
      error.value = failure
      return false
    }
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
