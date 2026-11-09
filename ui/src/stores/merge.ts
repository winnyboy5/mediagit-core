import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, type MergeOpts, type MergeReport, type MergeStrategy } from '@/api'
import { mergeSummary, toConflicts, type Conflict, type Side } from '@/lib/merge'
import { useAction } from './action'
import { useBranchesStore } from './branches'
import { useRepoStore } from './repo'

/** Merge & conflicts: start a merge, choose a version per conflicted file, continue or abort. */
export const useMergeStore = defineStore('merge', () => {
  const repo = useRepoStore()
  const branches = useBranchesStore()
  const { busy, error, running, run } = useAction(() => branches.refresh())

  /** A merge stopped on conflicts and is waiting for decisions. */
  const active = ref(false)
  const conflicts = shallowRef<Conflict[]>([])
  const theirsName = ref<string | null>(null)
  /** Commit of the branch being merged in; needed to take "their" version of a file. */
  const theirsOid = ref<string | null>(null)
  const last = ref<MergeReport | null>(null)

  const resolved = computed(() => conflicts.value.filter((c) => c.resolved).length)
  const canContinue = computed(() => active.value && resolved.value === conflicts.value.length)

  function apply(r: MergeReport, name: string | null, oid: string | null) {
    last.value = r
    if (r.status === 'conflict') {
      active.value = true
      conflicts.value = toConflicts(r.conflicts)
      if (name) {
        theirsName.value = name
        theirsOid.value = oid
      }
    } else {
      active.value = false
      conflicts.value = []
      theirsName.value = null
      theirsOid.value = null
    }
  }

  const isStopped = (r: MergeReport) => r.status === 'conflict'

  async function start(name: string, strategy: MergeStrategy, opts: MergeOpts = {}) {
    const oid = branches.list.find((b) => b.name === name)?.oid ?? null
    const r = await run(`Merge ${name}`, (root, report) => api.merge(root, name, strategy, opts, report), mergeSummary, isStopped)
    if (r) apply(r.value, name, oid)
    return !!r
  }

  async function resolve(path: string, side: Side) {
    const oid = theirsOid.value
    if (side === 'theirs' && !oid) return false
    const label = side === 'ours' ? 'mine' : 'theirs'
    const r = await run(
      `Keep ${label} of ${path}`,
      (root) => api.mergeResolve(root, path, side, oid ?? ''),
      () => `Kept ${label}`,
    )
    if (r) conflicts.value = conflicts.value.map((c) => (c.path === path ? { ...c, resolved: side } : c))
    return !!r
  }

  async function control(action: 'continue' | 'abort') {
    const label = action === 'continue' ? 'Finish merge' : 'Cancel merge'
    const r = await run(label, (root, report) => api.mergeControl(root, action, report), mergeSummary, isStopped)
    if (r) apply(r.value, null, null)
    return !!r
  }

  watch(
    () => repo.path,
    () => {
      active.value = false
      conflicts.value = []
      theirsName.value = null
      theirsOid.value = null
      last.value = null
      error.value = null
    },
  )

  return { active, conflicts, theirsName, theirsOid, last, resolved, canContinue, busy, error, running, start, resolve, continue: () => control('continue'), abort: () => control('abort') }
})
