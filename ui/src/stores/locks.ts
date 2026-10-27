import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type LockReport } from '@/api'
import { load, save } from '@/lib/persist'
import { useOpsStore } from './ops'
import { useRepoStore } from './repo'

const OWNER_KEY = 'mg.lock.owner'

export type LockFilter = 'all' | 'mine' | 'others'

/** Pure: which locks a filter shows. `me` is unknown until this app has created a lock. */
export function filterLocks(locks: readonly LockReport[], filter: LockFilter, me: string | null): LockReport[] {
  if (filter === 'all') return [...locks]
  return locks.filter((l) => (filter === 'mine') === (me !== null && l.owner === me))
}

/** File locks on the remote: list, lock, unlock. */
export const useLocksStore = defineStore('locks', () => {
  const repo = useRepoStore()
  const ops = useOpsStore()

  const locks = shallowRef<LockReport[]>([])
  const loaded = ref(false)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  const actionError = ref<ApiError | null>(null)
  const busy = ref<string | null>(null)
  /** The owner name locks made here are filed under; learned from the first lock this app creates. */
  const me = ref<string | null>(load<string | null>(OWNER_KEY, null))
  let token = 0

  const byPath = computed(() => new Map(locks.value.map((l) => [l.path, l])))

  async function refresh() {
    if (!repo.path) return
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      const r = await api.lockList(repo.path)
      if (mine !== token) return
      locks.value = [...r.locks].sort((a, b) => a.path.localeCompare(b.path))
      loaded.value = true
    } catch (e) {
      if (mine === token) error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  async function act(key: string, label: string, run: (root: string) => Promise<unknown>, done: string) {
    if (!repo.path || busy.value) return false
    const root = repo.path
    busy.value = key
    actionError.value = null
    try {
      await ops.track({ kind: 'other', label }, () => run(root), () => done)
      return true
    } catch (e) {
      actionError.value = toApiError(e)
      return false
    } finally {
      busy.value = null
      await refresh()
    }
  }

  async function lock(path: string) {
    return act(`lock:${path}`, `Lock ${path}`, async (root) => {
      const made = await api.lockCreate(root, path)
      me.value = made.owner
      save(OWNER_KEY, made.owner)
    }, `Locked ${path}`)
  }

  const unlock = (l: LockReport, force: boolean) =>
    act(`unlock:${l.lock_id}`, `${force ? 'Force unlock' : 'Unlock'} ${l.path}`, (root) => api.lockUnlock(root, l.lock_id, force), `Unlocked ${l.path}`)

  watch(
    () => repo.path,
    () => {
      token++
      locks.value = []
      loaded.value = false
      error.value = null
      actionError.value = null
    },
  )

  return { locks, byPath, loaded, loading, error, actionError, busy, me, refresh, lock, unlock }
})
