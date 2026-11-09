import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type BisectReport, type CherryPickReport, type LogCommitReport, type RebaseReport, type ReflogEntryReport, type ReflogReport } from '@/api'
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

/** Rebase: `branch` (default: the current one), `keepEmpty`. Cherry-pick: `noCommit`, `appendMessage`. */
export interface SequencerExtra {
  branch?: string
  keepEmpty?: boolean
  noCommit?: boolean
  appendMessage?: boolean
}

/** The recent versions the pickers offer. */
export const useVersionsStore = defineStore('versions', () => {
  const repo = useRepoStore()
  const list = shallowRef<LogCommitReport[]>([])
  /** Every branch's versions, for pickers that copy from another branch. */
  const allList = shallowRef<LogCommitReport[]>([])
  const loaded = ref(false)
  const error = ref<ApiError | null>(null)

  async function load() {
    const root = repo.path
    if (!root) return
    error.value = null
    try {
      const [r, all] = await Promise.all([api.log(root, 0, 100, false), api.log(root, 0, 200, true)])
      if (root !== repo.path) return
      list.value = r.commits
      allList.value = all.commits
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
      allList.value = []
      loaded.value = false
      error.value = null
    },
  )
  return { list, allList, loaded, error, load, summaryOf }
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
    /** Stopped on clashes is not a success: it needs the person, so the toast warns. */
    const stopped = (x: RebaseReport | CherryPickReport) => x.status === 'conflict'
    const stepOf = (r: RebaseReport | CherryPickReport): Step => ({ status: r.status, conflicts: r.conflicts, conflictCommit: r.conflict_commit })

    async function start(target: string, extra: SequencerExtra = {}) {
      const label = command === 'rebase' ? `Rebase onto ${target}` : `Copy ${target.slice(0, 7)}`
      const r = await run<RebaseReport | CherryPickReport>(
        label,
        (root, report) =>
          command === 'rebase'
            ? api.rebaseStart(root, target, report, { branch: extra.branch || undefined, keepEmpty: extra.keepEmpty || undefined })
            : api.pickStart(root, [target], report, { noCommit: extra.noCommit || undefined, appendMessage: extra.appendMessage || undefined }),
        (x) => stepSummary(stepOf(x)),
        stopped,
      )
      if (r) apply(stepOf(r.value))
      return !!r
    }

    async function control(action: 'continue' | 'abort' | 'skip') {
      const r = await run<RebaseReport | CherryPickReport>(
        `${action === 'abort' ? 'Cancel' : action === 'skip' ? 'Skip step of' : 'Continue'} ${command}`,
        (root, report) => api.sequencerControl(root, command, action, report),
        (x) => stepSummary(stepOf(x)),
        stopped,
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

  const start = (bad: string, good: string | null, restart = false) => exec('Start bisect', (root, p) => api.bisectStart(root, bad, good, p, restart))
  /** `commit` marks that version instead of the one being tested. */
  const mark = (verdict: 'good' | 'bad' | 'skip', commit: string | null = null) => exec(`Mark ${verdict}`, (root, p) => api.bisectMark(root, verdict, commit, p))
  /** `commit` moves the branch there instead of back to where the bisect began. */
  const reset = (commit?: string) => exec('End bisect', (root, p) => api.bisectReset(root, p, commit))
  const replay = (logfile: string) => exec('Replay bisect log', (root, p) => api.bisectReplay(root, logfile, p))

  watch(
    () => repo.path,
    () => {
      report.value = null
      error.value = null
    },
  )
  return { report, active, busy, error, running, start, mark, reset, replay }
})

/** The "every ref" choice in the undo log's picker (`reflog show --all`). */
export const ALL_REFS = '*all-refs'

/** The reflog of one ref, newest first. */
export const useReflogStore = defineStore('reflog', () => {
  const repo = useRepoStore()
  const reference = ref('HEAD')
  const entries = shallowRef<ReflogEntryReport[]>([])
  const loaded = ref(false)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  /** Every ref's newest entries, when `reference` is the all-refs choice. */
  const groups = shallowRef<ReflogReport[]>([])
  let token = 0

  async function refresh() {
    const root = repo.path
    if (!root) return
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      if (reference.value === ALL_REFS) {
        const r = await api.reflogAll(root, 20)
        if (mine !== token) return
        groups.value = r.refs
        entries.value = []
        loaded.value = true
        return
      }
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
      groups.value = []
      loaded.value = false
      error.value = null
    },
  )

  // Cleaning up undo history cannot itself be undone: the view asks first.
  const action = useAction(refresh)
  /** Forget where `ref` has been. */
  const forget = (ref: string) => action.run(`Clear undo history of ${ref}`, (root) => api.reflogDelete(root, ref), () => 'Undo history cleared')
  /** Keep the newest `keep` entries of `ref` (null = every ref). */
  const trim = (ref: string | null, keep: number) =>
    action.run(`Trim undo history${ref ? ` of ${ref}` : ''}`, (root) => api.reflogExpire(root, ref, keep), (r) => `Removed ${r.expired} ${r.expired === 1 ? 'entry' : 'entries'}`)
  return { reference, entries, groups, loaded, loading, error, refresh, forget, trim, busy: action.busy, actionError: action.error }
})
