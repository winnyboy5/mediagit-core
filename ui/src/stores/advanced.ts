import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type BisectReport, type CherryPickReport, type LogCommitReport, type RebaseReport, type ReflogEntryReport } from '@/api'
import type { Side } from '@/lib/merge'
import { useAction } from './action'
import { useBranchesStore } from './branches'
import { useRepoStore } from './repo'

/** A clash found while replaying versions; same shape as a merge conflict, decided per file. */
export interface Clash {
  path: string
  resolved: Side | null
}

/** One report from a sequencer command (`rebase`, `cherry-pick`), reduced to what the UI shows. */
interface Step {
  status: string
  conflicts: string[]
  conflictCommit: string | null
}

const SUMMARY: Record<string, string> = {
  rebased: 'Rebased',
  up_to_date: 'Already up to date',
  nothing_to_rebase: 'Nothing to rebase',
  aborted: 'Cancelled',
  picked: 'Copied',
  not_committed: 'Copied, not saved yet',
  completed: 'Done',
}

export function stepSummary(s: Step): string {
  if (s.status === 'conflict') return `Stopped: ${s.conflicts.length} ${s.conflicts.length === 1 ? 'file clashes' : 'files clash'}`
  return SUMMARY[s.status] ?? s.status
}

/** The recent versions the pickers offer. */
export const useVersionsStore = defineStore('versions', () => {
  const repo = useRepoStore()
  const list = shallowRef<LogCommitReport[]>([])
  const loaded = ref(false)
  const error = ref<ApiError | null>(null)

  async function load() {
    const root = repo.path
    if (!root) return
    error.value = null
    try {
      const r = await api.log(root, 0, 100, false)
      if (root !== repo.path) return
      list.value = r.commits
      loaded.value = true
    } catch (e) {
      error.value = toApiError(e)
    }
  }
  const summaryOf = (oid: string | null) => list.value.find((c) => c.oid === oid)?.summary ?? ''

  watch(
    () => repo.path,
    () => {
      list.value = []
      loaded.value = false
      error.value = null
    },
  )
  return { list, loaded, error, load, summaryOf }
})

/** Rebase and cherry-pick share one flow: start, maybe stop on clashes, then continue / skip / abort. */
function sequencer(command: 'rebase' | 'cherry-pick') {
  return () => {
    const repo = useRepoStore()
    const branches = useBranchesStore()
    const { busy, error, running, run } = useAction(() => branches.refresh())
    const active = ref(false)
    const clashes = shallowRef<Clash[]>([])
    const conflictCommit = ref<string | null>(null)
    const last = shallowRef<Step | null>(null)

    const resolved = computed(() => clashes.value.filter((c) => c.resolved).length)
    const canContinue = computed(() => active.value && resolved.value === clashes.value.length)

    function apply(s: Step) {
      last.value = s
      active.value = s.status === 'conflict'
      clashes.value = s.status === 'conflict' ? s.conflicts.map((path) => ({ path, resolved: null })) : []
      conflictCommit.value = s.status === 'conflict' ? s.conflictCommit : null
    }
    const stepOf = (r: RebaseReport | CherryPickReport): Step => ({ status: r.status, conflicts: r.conflicts, conflictCommit: r.conflict_commit })

    async function start(target: string) {
      const label = command === 'rebase' ? `Rebase onto ${target}` : `Copy ${target.slice(0, 7)}`
      const r = await run<RebaseReport | CherryPickReport>(
        label,
        (root, report) => (command === 'rebase' ? api.rebaseStart(root, target, report) : api.pickStart(root, [target], report)),
        (x) => stepSummary(stepOf(x)),
      )
      if (r) apply(stepOf(r.value))
      return !!r
    }

    async function control(action: 'continue' | 'abort' | 'skip') {
      const r = await run<RebaseReport | CherryPickReport>(
        `${action === 'abort' ? 'Cancel' : action === 'skip' ? 'Skip step of' : 'Continue'} ${command}`,
        (root, report) => api.sequencerControl(root, command, action, report),
        (x) => stepSummary(stepOf(x)),
      )
      if (r) apply(stepOf(r.value))
      return !!r
    }

    /** Take our version or the one being replayed; the replayed commit is "theirs". */
    async function resolve(path: string, side: Side) {
      const theirs = conflictCommit.value
      if (!theirs) return false
      const r = await run(`Keep ${side === 'ours' ? 'mine' : 'theirs'} of ${path}`, (root) => api.mergeResolve(root, path, side, theirs), () => 'Kept')
      if (r) clashes.value = clashes.value.map((c) => (c.path === path ? { ...c, resolved: side } : c))
      return !!r
    }

    watch(
      () => repo.path,
      () => {
        active.value = false
        clashes.value = []
        conflictCommit.value = null
        last.value = null
        error.value = null
      },
    )

    return { active, clashes, conflictCommit, last, resolved, canContinue, busy, error, running, start, control, resolve }
  }
}

export const useRebaseStore = defineStore('rebase', sequencer('rebase'))
export const usePickStore = defineStore('cherry-pick', sequencer('cherry-pick'))

const BISECT_SUMMARY: Record<string, string> = {
  waiting_for_good: 'Now mark a version that works',
  testing: 'Test the version that is checked out',
  complete: 'Found the first bad version',
  reset: 'Bisect ended',
}

/** Bisect: narrow down which version broke something, one verdict at a time. */
export const useBisectStore = defineStore('bisect', () => {
  const repo = useRepoStore()
  const { busy, error, running, run } = useAction(async () => {})
  const report = shallowRef<BisectReport | null>(null)

  const active = computed(() => !!report.value && ['waiting_for_good', 'testing', 'complete'].includes(report.value.status))
  const summary = (r: BisectReport) => BISECT_SUMMARY[r.status] ?? r.status

  async function exec(label: string, fn: Parameters<typeof run<BisectReport>>[1]) {
    const r = await run(label, fn, summary)
    if (r) report.value = r.value
    return !!r
  }

  const start = (bad: string, good: string | null) => exec('Start bisect', (root, p) => api.bisectStart(root, bad, good, p))
  const mark = (verdict: 'good' | 'bad' | 'skip') => exec(`Mark ${verdict}`, (root, p) => api.bisectMark(root, verdict, null, p))
  const reset = () => exec('End bisect', (root, p) => api.bisectReset(root, p))

  watch(
    () => repo.path,
    () => {
      report.value = null
      error.value = null
    },
  )
  return { report, active, busy, error, running, start, mark, reset }
})

/** The reflog of one ref, newest first. */
export const useReflogStore = defineStore('reflog', () => {
  const repo = useRepoStore()
  const reference = ref('HEAD')
  const entries = shallowRef<ReflogEntryReport[]>([])
  const loaded = ref(false)
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
      const r = await api.reflog(root, reference.value, 100)
      if (mine !== token) return
      entries.value = r.entries
      loaded.value = true
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
      reference.value = 'HEAD'
      entries.value = []
      loaded.value = false
      error.value = null
    },
  )
  return { reference, entries, loaded, loading, error, refresh }
})
