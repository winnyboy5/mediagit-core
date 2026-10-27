import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type FetchReport, type PullReport, type PushReport, type RemoteReport } from '@/api'
import { load, save } from '@/lib/persist'
import { summarizeTransfer } from '@/lib/sync'
import { useOpsStore, type OpKind } from './ops'
import { useRepoStore } from './repo'

const KEY = 'mg.lastsync'
export type TransferKind = Extract<OpKind, 'fetch' | 'pull' | 'push'>
const TRANSFERS: readonly OpKind[] = ['fetch', 'pull', 'push']
const LABEL: Record<TransferKind, string> = { fetch: 'Fetch', pull: 'Get latest', push: 'Send' }

/** Remotes plus fetch / pull / push, all tracked as cancellable ops. */
export const useSyncStore = defineStore('sync', () => {
  const repo = useRepoStore()
  const ops = useOpsStore()

  const remotes = shallowRef<RemoteReport[]>([])
  const loaded = ref(false)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  const actionError = ref<ApiError | null>(null)
  const chosen = ref<string | null>(null)
  const lastSync = ref<Record<string, number>>(load<Record<string, number>>(KEY, {}))
  const editing = ref(false)
  let token = 0

  /** Remote used for transfers: the user's pick, else the upstream's, else `origin`, else the first. */
  const remote = computed(() => {
    const names = remotes.value.map((r) => r.name)
    const upstream = repo.status?.branch.upstream?.name.split('/')[0]
    return [chosen.value, upstream, 'origin'].find((n) => n && names.includes(n)) ?? names[0] ?? null
  })
  const active = computed(() => ops.running.find((o) => TRANSFERS.includes(o.kind)))
  const last = computed(() => (repo.path ? (lastSync.value[repo.path] ?? null) : null))
  const busy = computed(() => editing.value || !!active.value)

  async function refresh() {
    if (!repo.path) return
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      const r = await api.remoteList(repo.path)
      if (mine !== token) return
      remotes.value = r.remotes
      loaded.value = true
    } catch (e) {
      if (mine === token) error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  async function edit(label: string, run: (root: string) => Promise<{ remotes: RemoteReport[] }>) {
    if (!repo.path || busy.value) return false
    const root = repo.path
    editing.value = true
    actionError.value = null
    try {
      const r = await ops.track({ kind: 'other', label }, () => run(root))
      remotes.value = r.remotes
      return true
    } catch (e) {
      actionError.value = toApiError(e)
      return false
    } finally {
      editing.value = false
    }
  }

  const addRemote = (name: string, url: string) => edit(`Add remote ${name}`, (root) => api.remoteAdd(root, name, url))
  const removeRemote = (name: string) => edit(`Remove remote ${name}`, (root) => api.remoteRemove(root, name))

  async function run(kind: TransferKind) {
    if (!repo.path || busy.value) return false
    const root = repo.path
    const target = remote.value
    actionError.value = null
    let id = ''
    const cancel = () => api.cancelOp(id)
    try {
      await ops.track<FetchReport | PullReport | PushReport>(
        { kind, label: `${LABEL[kind]}${target ? ` ${target}` : ''}`, cancel, retry: () => void run(kind) },
        (report, opId) => {
          id = opId
          return kind === 'fetch'
            ? api.fetch(root, target, opId, report)
            : kind === 'pull'
              ? api.pull(root, target, opId, report)
              : api.push(root, target, opId, report)
        },
        (r) => summarizeTransfer(kind as never, r as never),
      )
      lastSync.value = { ...lastSync.value, [root]: Date.now() }
      save(KEY, lastSync.value)
      return true
    } catch (e) {
      const err = toApiError(e)
      if (err.code !== 'cancelled') actionError.value = err
      return false
    } finally {
      await repo.refresh()
    }
  }

  watch(
    () => repo.path,
    () => {
      token++
      remotes.value = []
      loaded.value = false
      error.value = null
      actionError.value = null
      chosen.value = null
    },
  )

  return { remotes, remote, chosen, loaded, loading, error, actionError, active, busy, last, refresh, addRemote, removeRemote, run }
})
