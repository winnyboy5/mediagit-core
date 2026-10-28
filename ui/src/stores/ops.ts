import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import type { ApiError, Progress } from '@/api'
import { toApiError } from '@/api'
import { useToast } from '@/components/useToast'

export type OpKind = 'push' | 'pull' | 'fetch' | 'clone' | 'add' | 'commit' | 'init' | 'download' | 'other'
export type OpStatus = 'queued' | 'running' | 'done' | 'failed' | 'cancelled'

/** Only these can be killed safely (kill-safety gate G3, incl. pull since issues 93 and 95). */
export const CANCELLABLE: ReadonlySet<OpKind> = new Set(['push', 'pull', 'fetch', 'clone', 'download'])
/** Bars appear only for ops running longer than this (anti-flicker). */
export const BAR_DELAY_MS = 400
/** Taskbar progress only for ops running longer than this. */
export const TASKBAR_DELAY_MS = 2000
/** Rate/ETA need two samples at least this far apart. */
export const MIN_SAMPLE_SPAN_MS = 1000
const SAMPLE_WINDOW_MS = 30_000
const MIN_SAMPLE_GAP_MS = 250
const HISTORY_KEEP = 20

interface Sample {
  t: number
  pos: number
}

export interface Bar {
  id: number
  msg: string
  pos: number
  len: number | null
  unit: 'bytes' | 'items'
  done: boolean
  samples: Sample[]
}

export interface Op {
  id: string
  kind: OpKind
  label: string
  phase: string
  bars: Map<number, Bar>
  startedAt: number
  finishedAt?: number
  status: OpStatus
  error?: ApiError
  summary?: string
  /** Re-run a failed op; set by `track`. */
  retry?: () => void
  /** Ask the backend to stop; supplied by the caller. */
  cancel?: () => Promise<unknown> | void
}

export interface OpSpec {
  kind: OpKind
  label: string
  /** Backend cancel hook; only honoured for CANCELLABLE kinds. */
  cancel?: Op['cancel']
  /** Store-level action Retry runs (keeps its busy guard and refresh); defaults to re-running `fn`. */
  retry?: () => void
  /** The call carries a secret: keep no closure that could re-run it (and so hold the secret). */
  noRetry?: boolean
}

/** Bytes/s over the sample window, or null until two samples >= 1 s apart exist. */
export function rateOf(bar: Bar): number | null {
  if (bar.unit !== 'bytes' || bar.samples.length < 2) return null
  const first = bar.samples[0]!
  const last = bar.samples[bar.samples.length - 1]!
  const span = last.t - first.t
  if (span < MIN_SAMPLE_SPAN_MS) return null
  const delta = last.pos - first.pos
  return delta < 0 ? null : (delta / span) * 1000
}

/** Seconds remaining, or null when length or rate is unknown (or rate is 0). */
export function etaOf(bar: Bar): number | null {
  const rate = rateOf(bar)
  if (rate == null || rate <= 0 || bar.len == null) return null
  return Math.max(0, bar.len - bar.pos) / rate
}

/** The bar that represents the op: first unfinished bytes bar, else first unfinished, else last. */
export function primaryBar(op: Op): Bar | undefined {
  const bars = [...op.bars.values()]
  return bars.find((b) => !b.done && b.unit === 'bytes') ?? bars.find((b) => !b.done) ?? bars[bars.length - 1]
}

function applySample(bar: Bar, pos: number, now: number) {
  const last = bar.samples[bar.samples.length - 1]
  // Coalesce bursts into the newest sample, but keep the first as the window anchor.
  if (last && bar.samples.length > 1 && now - last.t < MIN_SAMPLE_GAP_MS) {
    last.t = now
    last.pos = pos
  } else bar.samples.push({ t: now, pos })
  while (bar.samples.length > 2 && now - bar.samples[0]!.t > SAMPLE_WINDOW_MS) bar.samples.shift()
}

export const useOpsStore = defineStore('ops', () => {
  const ops = ref<Op[]>([])
  const toast = useToast()
  /** Reactive clock for anti-flicker; ticks only while something runs. */
  const now = ref(Date.now())
  const taskbarValue = shallowRef<number | null | undefined>(undefined)
  let timer: ReturnType<typeof setInterval> | undefined
  let seq = 0

  const running = computed(() => ops.value.filter((o) => o.status === 'running'))
  const finished = computed(() =>
    ops.value
      .filter((o) => o.status !== 'running' && o.status !== 'queued')
      .sort((a, b) => (b.finishedAt ?? 0) - (a.finishedAt ?? 0))
      .slice(0, HISTORY_KEEP),
  )
  const queued = computed(() => ops.value.filter((o) => o.status === 'queued'))
  /** Newest running op whose bar is past the anti-flicker delay. */
  const current = computed(() => [...running.value].reverse().find((o) => now.value - o.startedAt >= BAR_DELAY_MS))

  const get = (id: string) => ops.value.find((o) => o.id === id)

  function tick() {
    now.value = Date.now()
    syncTaskbar()
    if (!running.value.length && timer) {
      clearInterval(timer)
      timer = undefined
    }
  }

  function begin(spec: OpSpec, at = Date.now(), status: 'running' | 'queued' = 'running'): string {
    const id = `op-${++seq}`
    ops.value.push({
      id,
      kind: spec.kind,
      label: spec.label,
      phase: '',
      bars: new Map(),
      startedAt: at,
      status,
      cancel: CANCELLABLE.has(spec.kind) ? spec.cancel : undefined,
    })
    now.value = at
    if (status === 'running' && !timer) timer = setInterval(tick, 250)
    return id
  }

  /** An op waiting behind another (per-repo write queue). */
  const queue = (spec: OpSpec, at = Date.now()) => begin(spec, at, 'queued')

  /** Promote a queued op to running; its clock starts now. */
  function start(id: string, at = Date.now()) {
    const op = get(id)
    if (!op || op.status !== 'queued') return
    op.status = 'running'
    op.startedAt = at
    now.value = at
    if (!timer) timer = setInterval(tick, 250)
  }

  function progress(id: string, p: Progress, at = Date.now()) {
    const op = get(id)
    if (!op || op.status !== 'running') return
    let bar = op.bars.get(p.id)
    if (!bar) {
      bar = { id: p.id, msg: p.msg, pos: p.pos, len: p.len, unit: p.unit, done: p.done, samples: [] }
      op.bars.set(p.id, bar)
    }
    bar.msg = p.msg
    bar.pos = p.pos
    bar.len = p.len
    bar.unit = p.unit
    bar.done = p.done
    applySample(bar, p.pos, at)
    if (!p.done) op.phase = p.msg
    now.value = at
    syncTaskbar()
  }

  function end(op: Op, status: OpStatus, at: number) {
    op.status = status
    op.finishedAt = at
    op.cancel = undefined
    now.value = at
    syncTaskbar()
  }

  function finish(id: string, summary?: string, at = Date.now(), warn = false) {
    const op = get(id)
    if (!op || op.status !== 'running') return
    for (const b of op.bars.values()) b.done = true
    op.summary = summary
    end(op, 'done', at)
    toast.push({ kind: warn ? 'warning' : 'success', title: op.label, description: summary })
  }

  function fail(id: string, error: ApiError, at = Date.now()) {
    const op = get(id)
    if (!op || op.status !== 'running') return
    op.error = error
    end(op, error.code === 'cancelled' ? 'cancelled' : 'failed', at)
  }

  /** Request cancel: the backend kills the child and the pending run rejects with `cancelled`. */
  async function cancel(id: string) {
    const op = get(id)
    if (!op || op.status !== 'running' || !CANCELLABLE.has(op.kind) || !op.cancel) return
    await op.cancel()
  }

  /** Run `fn` as a tracked op: progress in, summary or error out; failed ops get `retry`. */
  async function track<T>(
    spec: OpSpec,
    fn: (report: (p: Progress) => void, id: string) => Promise<T>,
    summarize?: (result: T) => string | undefined,
    warn?: (result: T) => boolean,
  ): Promise<T> {
    const id = begin(spec)
    try {
      const result = await fn((p) => progress(id, p), id)
      finish(id, summarize?.(result), Date.now(), warn?.(result))
      return result
    } catch (e) {
      fail(id, toApiError(e))
      const op = get(id)
      if (op && op.status === 'failed' && !spec.noRetry) {
        op.retry =
          spec.retry ??
          (() => {
            track(spec, fn, summarize, warn).catch(() => {})
          })
      }
      throw e
    }
  }

  /** Taskbar progress: undefined = untouched, null = cleared, -1 = indeterminate, 0..100 = value. */
  function syncTaskbar() {
    const longRunning = running.value.filter((o) => now.value - o.startedAt > TASKBAR_DELAY_MS)
    let next: number | null | undefined
    if (!longRunning.length) next = taskbarValue.value === undefined ? undefined : null
    else {
      const bar = primaryBar(longRunning[longRunning.length - 1]!)
      next = bar?.len ? Math.round(Math.min(1, bar.pos / bar.len) * 100) : -1
    }
    if (next === taskbarValue.value) return
    taskbarValue.value = next
    if (next !== undefined) void setTaskbar(next)
  }

  function reset() {
    ops.value = []
    if (timer) clearInterval(timer)
    timer = undefined
    taskbarValue.value = undefined
  }

  return { ops, now, running, queued, finished, current, taskbarValue, begin, queue, start, progress, finish, fail, cancel, track, reset }
})

/** Native taskbar progress; a no-op outside Tauri. */
async function setTaskbar(value: number | null) {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return
  try {
    const { getCurrentWindow, ProgressBarStatus } = await import('@tauri-apps/api/window')
    const w = getCurrentWindow()
    if (value === null) await w.setProgressBar({ status: ProgressBarStatus.None })
    else if (value < 0) await w.setProgressBar({ status: ProgressBarStatus.Indeterminate })
    else await w.setProgressBar({ status: ProgressBarStatus.Normal, progress: value })
  } catch {
    // Taskbar progress is cosmetic; never fail an operation over it.
  }
}
