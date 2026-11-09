import { defineStore } from 'pinia'
import { ref, shallowRef, watch } from 'vue'
import { api, type FsckCliReport, type FsckOptions, type GcOptions, type GcReport, type VerifyOptions, type VerifyRangeReport } from '@/api'
import { load, save } from '@/lib/persist'
import { useAction } from './action'
import { useRepoStore } from './repo'
import { useUnlockStore } from './unlock'

const KEY = 'mg.maint.last'
export type Task = 'gc' | 'fsck' | 'verify'

export interface Run<T> {
  report: T
  at: number
  /** gc only: the figures are what a real run would do. */
  preview?: boolean
  /** fsck only: `preview` = what a repair would change; `done` = a repair ran. */
  repair?: 'preview' | 'done'
}

const isRange = (r: FsckCliReport | VerifyRangeReport): r is VerifyRangeReport => 'commits' in r

/** Maintenance: clean up (gc), check (fsck) and verify, one at a time, each with its last report. */
export const useMaintenanceStore = defineStore('maintenance', () => {
  const repo = useRepoStore()
  const unlock = useUnlockStore()
  const { busy, error, running, run } = useAction(async () => {})

  const gc = shallowRef<Run<GcReport> | null>(null)
  const fsck = shallowRef<Run<FsckCliReport> | null>(null)
  const verify = shallowRef<Run<FsckCliReport> | null>(null)
  /** `verify` of one version or a range reports versions, not objects. */
  const verifyRange = shallowRef<Run<VerifyRangeReport> | null>(null)
  /** Which task is running, for its card. */
  const task = ref<Task | null>(null)
  /** When each task last ran, per repository. */
  const last = ref<Record<string, Partial<Record<Task, number>>>>(load('mg.maint.last', {}))

  function stamp(t: Task) {
    if (!repo.path) return
    last.value = { ...last.value, [repo.path]: { ...last.value[repo.path], [t]: Date.now() } }
    save(KEY, last.value)
  }

  async function exec<T>(t: Task, label: string, fn: Parameters<typeof run<T>>[1], summarize: (r: T) => string, warn: (r: T) => boolean) {
    if (busy.value) return null
    task.value = t
    const r = await run(label, fn, summarize, warn, false, true)
    task.value = null
    if (r) stamp(t)
    return r
  }

  /** `dryRun` reports what would go; a real run must be `confirmed`. */
  async function runGc(dryRun: boolean, confirmed: boolean, options: GcOptions = {}) {
    const r = await exec(
      'gc',
      dryRun ? 'Preview clean-up' : 'Clean up storage',
      (root, report) => unlock.withUnlock((p) => api.gc(root, dryRun, confirmed, options, p, report)),
      (g) => (g.errors.length ? `Finished with ${g.errors.length} errors` : `${g.objects_deleted} objects ${dryRun ? 'could go' : 'removed'}`),
      (g) => g.errors.length > 0,
    )
    if (r) gc.value = { report: r.value, at: Date.now(), preview: dryRun }
    return !!r
  }

  /** A repair changes the repository: the caller sets `options.confirmed` once the person agreed. */
  async function runFsck(options: FsckOptions = {}) {
    const r = await exec(
      'fsck',
      options.repair ? (options.dryRun ? 'Preview repair' : 'Repair repository') : options.deep ? 'Deep integrity check' : 'Integrity check',
      (root, report) => unlock.withUnlock((p) => api.fsck(root, options, p, report)),
      (x) => (x.healthy ? `${x.objects_checked} objects checked, no problems` : 'Problems found'),
      (x) => !x.healthy,
    )
    if (r) fsck.value = { report: r.value, at: Date.now(), repair: options.repair ? (options.dryRun ? 'preview' : 'done') : undefined }
    return !!r
  }

  /** The whole repository, or one version or range (`options.commit` / `start` / `end`). */
  async function runVerify(options: VerifyOptions = {}) {
    const r = await exec<FsckCliReport | VerifyRangeReport>(
      'verify',
      'Verify repository',
      (root, report) => unlock.withUnlock((p) => api.verify(root, options, p, report)),
      (x) => (isRange(x) ? (x.errors ? 'Problems found' : 'Every version verified') : x.healthy ? 'Every object verified' : 'Problems found'),
      (x) => (isRange(x) ? x.errors > 0 : !x.healthy),
    )
    if (r) {
      const at = Date.now()
      if (isRange(r.value)) {
        verifyRange.value = { report: r.value, at }
        verify.value = null
      } else {
        verify.value = { report: r.value, at }
        verifyRange.value = null
      }
    }
    return !!r
  }

  watch(
    () => repo.path,
    () => {
      gc.value = fsck.value = verify.value = verifyRange.value = null
      error.value = null
    },
  )

  return { gc, fsck, verify, verifyRange, task, last, busy, error, running, runGc, runFsck, runVerify }
})
