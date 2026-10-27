import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type BranchEntryReport, type Progress } from '@/api'
import { useOpsStore } from './ops'
import { useRepoStore } from './repo'

/** Branches view state: list plus create / switch / rename / delete. */
export const useBranchesStore = defineStore('branches', () => {
  const repo = useRepoStore()
  const ops = useOpsStore()

  const list = shallowRef<BranchEntryReport[]>([])
  const current = ref<string | null>(null)
  const loaded = ref(false)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  /** The last failed action (e.g. a dirty working tree refusing a switch). */
  const actionError = ref<ApiError | null>(null)
  const busy = ref(false)
  const switchOpId = ref<string | null>(null)
  let token = 0

  /** The switch in flight, for the progress overlay. */
  const switching = computed(() => ops.ops.find((o) => o.id === switchOpId.value && o.status === 'running'))

  async function refresh() {
    if (!repo.path) return
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      const r = await api.branchList(repo.path)
      if (mine !== token) return
      list.value = r.branches.filter((b) => !b.remote)
      current.value = r.current
      loaded.value = true
    } catch (e) {
      if (mine === token) error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  async function act(label: string, run: (root: string, report: (p: Progress) => void) => Promise<unknown>, done: string, isSwitch = false) {
    if (!repo.path || busy.value) return false
    const root = repo.path
    busy.value = true
    actionError.value = null
    try {
      await ops.track({ kind: 'other', label }, (report, id) => {
        if (isSwitch) switchOpId.value = id
        return run(root, report)
      }, () => done)
      return true
    } catch (e) {
      actionError.value = toApiError(e)
      return false
    } finally {
      busy.value = false
      switchOpId.value = null
      await Promise.all([refresh(), repo.refresh()])
    }
  }

  const create = (name: string) => act(`Create branch ${name}`, (root) => api.branchCreate(root, name), `Created branch ${name}`)
  const switchTo = (name: string) => act(`Switch to ${name}`, (root, report) => api.branchSwitch(root, name, report), `Switched to ${name}`, true)
  const remove = (name: string) => act(`Delete branch ${name}`, (root) => api.branchDelete(root, name), `Deleted branch ${name}`)
  const rename = (from: string, to: string) => act(`Rename ${from}`, (root) => api.branchRename(root, from, to), `Renamed ${from} to ${to}`)

  watch(
    () => repo.path,
    () => {
      token++
      list.value = []
      current.value = null
      loaded.value = false
      error.value = null
      actionError.value = null
    },
  )

  return { list, current, loaded, loading, error, actionError, busy, switching, refresh, create, switchTo, remove, rename }
})
