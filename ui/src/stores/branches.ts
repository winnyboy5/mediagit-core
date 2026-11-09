import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type BranchCreateOpts, type BranchDeleteOpts, type BranchEntryReport, type BranchSwitchOpts, type Progress } from '@/api'
import { useOpsStore } from './ops'
import { useRepoStore } from './repo'

/** Branches view state: list plus create / switch / rename / delete. */
export const useBranchesStore = defineStore('branches', () => {
  const repo = useRepoStore()
  const ops = useOpsStore()

  const list = shallowRef<BranchEntryReport[]>([])
  /** Remote-tracking branches (`origin/feature`): what the last fetch saw. */
  const remotes = shallowRef<BranchEntryReport[]>([])
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
      // Remote-tracking branches are a bonus: a failure here must not hide the local list.
      const rr = await api.branchList(repo.path, true).catch(() => null)
      if (mine === token) remotes.value = rr ? rr.branches.filter((b) => b.remote) : []
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
      await ops.track({ kind: 'other', label, retry: () => void act(label, run, done, isSwitch) }, (report, id) => {
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

  const create = (name: string, opts?: BranchCreateOpts) => act(`Create branch ${name}`, (root) => api.branchCreate(root, name, opts), `Created branch ${name}`)
  const switchTo = (name: string, opts?: BranchSwitchOpts) =>
    act(`Switch to ${name}`, (root, report) => api.branchSwitch(root, name, report, opts), `Switched to ${name}`, true)
  /** Make a local branch that follows a remote-tracking one (`origin/feature`) and switch to it. */
  const checkoutRemote = (remoteName: string) => switchTo(remoteName, { create: true, track: true })
  const remove = (name: string, opts?: BranchDeleteOpts) =>
    act(`Delete branch ${name}`, (root) => api.branchDelete(root, name, opts), `Deleted branch ${name}`)
  /** Forget a remote-tracking branch on this computer; nothing changes on the server. */
  const forgetRemote = (name: string) => remove(name, { remote: true })
  const rename = (from: string, to: string, overwrite = false) =>
    act(`Rename ${from}`, (root) => api.branchRename(root, from, to, overwrite), `Renamed ${from} to ${to}`)
  const protect = (name: string, requireReviews: boolean) =>
    act(`Protect ${name}`, (root) => api.branchProtect(root, name, requireReviews, false), `Protected ${name}`)
  const unprotect = (name: string) => act(`Remove protection from ${name}`, (root) => api.branchProtect(root, name, false, true), `${name} is no longer protected`)

  /** One branch's details (`branch show`). */
  const detail = shallowRef<BranchEntryReport | null>(null)
  async function show(name: string) {
    if (!repo.path) return
    detail.value = null
    actionError.value = null
    try {
      detail.value = await api.branchShow(repo.path, name)
    } catch (e) {
      actionError.value = toApiError(e)
    }
  }

  watch(
    () => repo.path,
    () => {
      token++
      list.value = []
      remotes.value = []
      detail.value = null
      current.value = null
      loaded.value = false
      error.value = null
      actionError.value = null
    },
  )

  return { list, remotes, current, loaded, loading, error, actionError, busy, switching, detail, refresh, create, switchTo, checkoutRemote, remove, forgetRemote, rename, protect, unprotect, show }
})
