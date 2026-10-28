import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { Progress } from '@/api'
import { useToast } from '@/components/useToast'
import { BAR_DELAY_MS, etaOf, primaryBar, rateOf, useOpsStore } from '@/stores/ops'

const P = (pos: number, len: number | null = 1000, id = 1, over: Partial<Progress> = {}): Progress => ({
  id,
  msg: 'Uploading',
  pos,
  len,
  unit: 'bytes',
  done: false,
  ...over,
})

let ops: ReturnType<typeof useOpsStore>
beforeEach(() => {
  setActivePinia(createPinia())
  ops = useOpsStore()
  ops.reset()
})

const bar = (id: string, barId = 1) => ops.ops.find((o) => o.id === id)!.bars.get(barId)!

describe('rate and ETA', () => {
  it('shows nothing until two samples at least 1 s apart exist', () => {
    const id = ops.begin({ kind: 'push', label: 'Push' }, 0)
    ops.progress(id, P(100), 0)
    expect(rateOf(bar(id))).toBeNull()
    ops.progress(id, P(200), 900) // second sample, but only 900 ms after the first
    expect(rateOf(bar(id))).toBeNull()
    expect(etaOf(bar(id))).toBeNull()
    ops.progress(id, P(300), 1000)
    expect(rateOf(bar(id))).toBe(200) // 200 B over 1 s
    expect(etaOf(bar(id))).toBe(3.5)
  })

  it('never invents an ETA without a length', () => {
    const id = ops.begin({ kind: 'push', label: 'Push' }, 0)
    ops.progress(id, P(0, null), 0)
    ops.progress(id, P(500, null), 2000)
    expect(rateOf(bar(id))).toBe(250)
    expect(etaOf(bar(id))).toBeNull()
  })

  it('has no rate for item bars', () => {
    const id = ops.begin({ kind: 'add', label: 'Add' }, 0)
    ops.progress(id, P(0, 10, 1, { unit: 'items' }), 0)
    ops.progress(id, P(5, 10, 1, { unit: 'items' }), 3000)
    expect(rateOf(bar(id))).toBeNull()
  })

  it('handles lumpy progress: a stall gives rate 0 and no ETA, a lump recovers', () => {
    const id = ops.begin({ kind: 'push', label: 'Push' }, 0)
    ops.progress(id, P(500), 0)
    ops.progress(id, P(500), 2000)
    expect(rateOf(bar(id))).toBe(0)
    expect(etaOf(bar(id))).toBeNull() // cannot divide by a stall
    ops.progress(id, P(900), 4000) // credit arrives in one lump
    expect(rateOf(bar(id))).toBe(100)
    expect(etaOf(bar(id))).toBe(1)
  })

  it('coalesces updates faster than 250 ms so history stays bounded', () => {
    const id = ops.begin({ kind: 'push', label: 'Push' }, 0)
    for (let t = 0; t < 5000; t += 50) ops.progress(id, P(t), t)
    expect(bar(id).samples.length).toBeLessThanOrEqual(22)
    expect(rateOf(bar(id))).toBeCloseTo(1000, -1)
  })

  it('ignores progress for unknown or finished ops', () => {
    ops.progress('nope', P(1), 0)
    const id = ops.begin({ kind: 'push', label: 'Push' }, 0)
    ops.finish(id, 'ok', 10)
    ops.progress(id, P(5), 20)
    expect(ops.ops[0]!.bars.size).toBe(0)
  })
})

describe('multi-bar ops', () => {
  it('tracks each tracker id separately and picks the unfinished bytes bar as primary', () => {
    const id = ops.begin({ kind: 'push', label: 'Push' }, 0)
    ops.progress(id, P(10, 10, 1, { msg: 'Chunking', unit: 'items', done: true }), 0)
    ops.progress(id, P(100, 1000, 2, { msg: 'Upload' }), 0)
    const op = ops.ops[0]!
    expect(op.bars.size).toBe(2)
    expect(primaryBar(op)!.id).toBe(2)
    expect(op.phase).toBe('Upload')
  })

  it('marks all bars done when the op finishes', () => {
    const id = ops.begin({ kind: 'clone', label: 'Clone' }, 0)
    ops.progress(id, P(1, 10), 0)
    ops.finish(id, 'Cloned', 5)
    expect(ops.ops[0]!.bars.get(1)!.done).toBe(true)
    expect(ops.ops[0]!.summary).toBe('Cloned')
  })
})

describe('anti-flicker', () => {
  it('current is hidden until the op has run 400 ms', () => {
    const id = ops.begin({ kind: 'push', label: 'Push' }, 1000)
    expect(ops.current).toBeUndefined()
    ops.progress(id, P(1), 1000 + BAR_DELAY_MS - 1)
    expect(ops.current).toBeUndefined()
    ops.progress(id, P(2), 1000 + BAR_DELAY_MS)
    expect(ops.current?.id).toBe(id)
  })

  it('a short op never becomes current', () => {
    const id = ops.begin({ kind: 'commit', label: 'Commit' }, 0)
    ops.finish(id, undefined, 100)
    expect(ops.current).toBeUndefined()
    expect(ops.finished).toHaveLength(1)
  })
})

describe('cancel visibility', () => {
  it('only push/pull/fetch/clone keep a cancel hook', () => {
    const cancel = vi.fn()
    for (const kind of ['push', 'pull', 'fetch', 'clone', 'add', 'commit', 'init'] as const) {
      const id = ops.begin({ kind, label: kind, cancel }, 0)
      expect(!!ops.ops.find((o) => o.id === id)!.cancel).toBe(['push', 'pull', 'fetch', 'clone'].includes(kind))
    }
  })

  it('cancel calls the hook once and is a no-op for non-cancellable or finished ops', async () => {
    const cancel = vi.fn()
    const a = ops.begin({ kind: 'add', label: 'Add', cancel }, 0)
    await ops.cancel(a)
    const p = ops.begin({ kind: 'push', label: 'Push', cancel }, 0)
    await ops.cancel(p)
    expect(cancel).toHaveBeenCalledTimes(1)
    ops.finish(p, undefined, 1)
    await ops.cancel(p)
    expect(cancel).toHaveBeenCalledTimes(1)
  })

  it('a cancelled error code lands as cancelled, not failed', () => {
    const id = ops.begin({ kind: 'clone', label: 'Clone' }, 0)
    ops.fail(id, { code: 'cancelled', message: 'Cancelled' }, 1)
    expect(ops.ops[0]!.status).toBe('cancelled')
  })
})

describe('track', () => {
  it('records success with a summary', async () => {
    const r = await ops.track({ kind: 'commit', label: 'Save' }, async () => 7, (n) => `n=${n}`)
    expect(r).toBe(7)
    expect(ops.finished[0]).toMatchObject({ status: 'done', summary: 'n=7' })
  })

  it('records failure with code and message, and retry re-runs the work', async () => {
    let calls = 0
    const fn = async () => {
      calls++
      if (calls === 1) throw { code: 'json_unsupported', message: 'not yet' }
      return 'ok'
    }
    await expect(ops.track({ kind: 'add', label: 'Add' }, fn)).rejects.toMatchObject({ code: 'json_unsupported' })
    const failed = ops.finished[0]!
    expect(failed).toMatchObject({ status: 'failed', error: { code: 'json_unsupported', message: 'not yet' } })
    failed.retry!()
    await vi.waitFor(() => expect(ops.ops.some((o) => o.status === 'done')).toBe(true))
    expect(calls).toBe(2)
  })

  it('keeps only the last 20 finished ops', () => {
    for (let i = 0; i < 25; i++) ops.finish(ops.begin({ kind: 'add', label: `a${i}` }, i), undefined, i + 1)
    expect(ops.finished).toHaveLength(20)
    expect(ops.finished[0]!.label).toBe('a24')
  })
})

describe('queue and toasts', () => {
  it('queued ops do not run, tick, or show a bar until started', () => {
    const id = ops.queue({ kind: 'push', label: 'Push' }, 0)
    expect(ops.queued.map((o) => o.id)).toEqual([id])
    expect(ops.running).toHaveLength(0)
    expect(ops.current).toBeUndefined()
    ops.start(id, 5000)
    expect(ops.queued).toHaveLength(0)
    expect(ops.ops[0]!.startedAt).toBe(5000)
    expect(ops.running).toHaveLength(1)
  })

  it('finishing pushes a success toast with the summary', () => {
    const toast = useToast()
    toast.items.value.forEach((t) => toast.dismiss(t.id))
    const before = toast.items.value.length
    const id = ops.begin({ kind: 'push', label: 'Push origin' }, 0)
    ops.finish(id, 'Pushed 14 files', 10)
    const last = toast.items.value[toast.items.value.length - 1]!
    expect(toast.items.value.length).toBe(before + 1)
    expect(last).toMatchObject({ kind: 'success', title: 'Push origin', description: 'Pushed 14 files' })
  })
})

describe('taskbar', () => {
  it('stays untouched for short ops, follows the primary bar after 2 s, clears at the end', () => {
    const id = ops.begin({ kind: 'push', label: 'Push' }, 0)
    ops.progress(id, P(500), 1500)
    expect(ops.taskbarValue).toBeUndefined()
    ops.progress(id, P(500), 2500)
    expect(ops.taskbarValue).toBe(50)
    ops.progress(id, P(0, null), 3000)
    expect(ops.taskbarValue).toBe(-1) // unknown length: indeterminate, not a made-up percentage
    ops.finish(id, undefined, 3500)
    expect(ops.taskbarValue).toBeNull()
  })

  it('never touches the taskbar for an op that finishes quickly', () => {
    const id = ops.begin({ kind: 'commit', label: 'Save' }, 0)
    ops.finish(id, undefined, 500)
    expect(ops.taskbarValue).toBeUndefined()
  })
})
