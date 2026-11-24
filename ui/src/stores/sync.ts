import { defineStore } from 'pinia'
import { computed, reactive, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type FetchReport, type MergeReport, type PullReport, type PushReport, type RemoteReport, type RemoteShowReport } from '@/api'
import { load, save } from '@/lib/persist'
import { mergeSummary } from '@/lib/merge'
import { emptyForm, pushIsRisky, summarizeTransfer, transferOpts } from '@/lib/sync'
import { router } from '@/router'
import { useMergeStore } from './merge'
import { useOpsStore, type OpKind } from './ops'
import { useRepoStore } from './repo'

const KEY = 'mg.lastsync'
export type TransferKind = Extract<OpKind, 'fetch' | 'pull' | 'push'>
const TRANSFERS: readonly OpKind[] = ['fetch', 'pull', 'push']
/** A pull that hit a clash reports the same shape as a merge that stopped. */
const stoppedOnConflict = (r: unknown): r is MergeReport => (r as MergeReport).status === 'conflict'
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
  /** Said after a Cancel, so the panel does not just vanish. Cleared by the next run. */
  const stopped = ref<string | null>(null)
  const chosen = ref<string | null>(null)
  const lastSync = ref<Record<string, number>>(load<Record<string, number>>(KEY, {}))
  const editing = ref(false)
  /** Options for the next fetch / pull / push (More options). */
  const form = reactive(emptyForm())
  const detail = ref<RemoteShowReport | null>(null)
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
      token++ // a refresh still in flight must not put back an older list
      remotes.value = r.remotes
      return true
    } catch (e) {
      actionError.value = toApiError(e)
      // A failure can still change the list: `remote add --fetch` keeps the
      // remote when only the fetch fails.
      void refresh()
      return false
    } finally {
      editing.value = false
    }
  }

  const addRemote = (name: string, url: string, opts: { fetch?: boolean; token?: string } = {}) =>
    edit(`Add remote ${name}`, (root) => api.remoteAdd(root, name, url, opts))
  const removeRemote = (name: string) => edit(`Remove remote ${name}`, (root) => api.remoteRemove(root, name))

  /** Rename / change address: the CLI returns no remote set, so read it again. */
  const change = (label: string, run: (root: string) => Promise<unknown>) =>
    edit(label, async (root) => {
      await run(root)
      return api.remoteList(root)
    })
  const renameRemote = (from: string, to: string) => change(`Rename remote ${from}`, (root) => api.remoteRename(root, from, to))
  const setRemoteUrl = (name: string, url: string, push: boolean) =>
    change(`Change address of ${name}`, (root) => api.remoteSetUrl(root, name, url, push))

  /** Ask the server about a remote; `detail` is null while it loads and after a failure. */
  async function showRemote(name: string) {
    if (!repo.path) return false
    const root = repo.path
    detail.value = null
    actionError.value = null
    try {
      detail.value = await ops.track({ kind: 'other', label: `Check remote ${name}` }, () => api.remoteShow(root, name), () => `Checked ${name}`)
      return true
    } catch (e) {
      actionError.value = toApiError(e)
      return false
    }
  }

  /** Whether the next push needs the person to agree first (overwrite or delete on the remote). */
  const pushNeedsConfirm = computed(() => pushIsRisky(form.push))
  /** The overwrite / delete confirmation lives on the Sync page; the header's Send asks for it there. */
  const confirmPushOpen = ref(false)
  const pushDeleteNeedsName = computed(() => form.push.delete && !form.push.refspecs.trim())

  async function run(kind: TransferKind, confirmed = false) {
    if (!repo.path || busy.value) return false
    const root = repo.path
    const target = remote.value
    // Frozen now: the form may change while the transfer runs, and a retry repeats what was asked.
    const opts = transferOpts(kind as 'push', form, confirmed)
    actionError.value = null
    stopped.value = null
    let id = ''
    const cancel = () => api.cancelOp(id)
    try {
      const done = await ops.track<FetchReport | PullReport | PushReport | MergeReport>(
        { kind, label: `${LABEL[kind]}${target ? ` ${target}` : ''}`, cancel, retry: () => void run(kind, confirmed) },
        (report, opId) => {
          id = opId
          return kind === 'fetch'
            ? api.fetch(root, target, opId, report, opts as never)
            : kind === 'pull'
              ? api.pull(root, target, opId, report, opts as never)
              : api.push(root, target, opId, report, opts as never)
        },
        (r) => (stoppedOnConflict(r) ? mergeSummary(r) : summarizeTransfer(kind as never, r as never)),
        stoppedOnConflict,
      )
      // A pull that meets a clash stops like Merge does: send the person to resolve it.
      if (stoppedOnConflict(done)) {
        useMergeStore().apply(done, null, null)
        void router.push('/branches/merge')
      }
      lastSync.value = { ...lastSync.value, [root]: Date.now() }
      save(KEY, lastSync.value)
      return true
    } catch (e) {
      const err = toApiError(e)
      if (err.code !== 'cancelled') actionError.value = err
      else stopped.value = `${LABEL[kind]} cancelled. Nothing is lost; run it again to continue.`
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
      stopped.value = null
      chosen.value = null
    },
  )

  return { remotes, remote, chosen, loaded, loading, error, actionError, stopped, active, busy, last, refresh, form, detail, pushNeedsConfirm, confirmPushOpen, pushDeleteNeedsName, addRemote, removeRemote, renameRemote, setRemoteUrl, showRemote, run }
})
