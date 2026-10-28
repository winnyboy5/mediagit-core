import { defineStore } from 'pinia'
import { ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type StatsReport } from '@/api'
import { useRepoStore } from './repo'
import { useUnlockStore } from './unlock'

/** Storage view: repository statistics. */
export const useStorageStore = defineStore('storage', () => {
  const repo = useRepoStore()
  const unlock = useUnlockStore()
  const stats = shallowRef<StatsReport | null>(null)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  let token = 0

  async function refresh() {
    const root = repo.path
    if (!root) return
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      const r = await unlock.withUnlock((p) => api.stats(root, p))
      if (mine === token) stats.value = r
    } catch (e) {
      if (mine === token) error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  watch(
    () => repo.path,
    () => {
      token++
      stats.value = null
      error.value = null
    },
  )

  return { stats, loading, error, refresh }
})
