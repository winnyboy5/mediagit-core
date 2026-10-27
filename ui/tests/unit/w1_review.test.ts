import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  log: vi.fn(),
  show: vi.fn(),
  add: vi.fn(),
  commit: vi.fn(),
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { batchPaths, useChangesStore } from '@/stores/changes'
import { useHistoryStore } from '@/stores/history'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'

const STATUS = { branch: { name: 'main' }, staged: [], modified: ['a.psd'], deleted: [], untracked: [] }

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue(STATUS)
  api.allowRepo.mockResolvedValue(undefined)
  useOpsStore().reset()
})

describe('history store', () => {
  it('reload clears a loadMore it superseded', async () => {
    const repo = useRepoStore()
    repo.path = '/r'
    const h = useHistoryStore()
    api.show.mockResolvedValue({ oid: 'o0', parents: [], files: [] })
    api.log.mockResolvedValueOnce({ commits: Array.from({ length: 200 }, (_, i) => ({ oid: `o${i}`, parents: [], author: { name: 'a', email: 'a@x' }, time: 1, summary: 's', message: 's' })), refs: [] })
    await h.reload()
    let release!: (v: unknown) => void
    api.log.mockReturnValueOnce(new Promise((r) => (release = r)))
    void h.loadMore()
    expect(h.loadingMore).toBe(true)
    api.log.mockResolvedValueOnce({ commits: [], refs: [] })
    await h.reload()
    expect(h.loadingMore).toBe(false)
    release({ commits: [], refs: [] })
  })
})

describe('ops retry', () => {
  it('runs the store-level retry from the spec instead of re-running fn', async () => {
    const ops = useOpsStore()
    const fn = vi.fn().mockRejectedValue(new Error('boom'))
    const retry = vi.fn()
    await expect(ops.track({ kind: 'other', label: 'x', retry }, fn)).rejects.toThrow()
    ops.ops[0]!.retry!()
    expect(retry).toHaveBeenCalledTimes(1)
    expect(fn).toHaveBeenCalledTimes(1)
  })

  it('a failed stage retries through the store: guard released and status refreshed', async () => {
    const repo = useRepoStore()
    repo.path = '/r'
    const c = useChangesStore()
    api.add.mockRejectedValueOnce({ code: 'error', message: 'locked' })
    await c.stageAll()
    expect(c.busy).toBeNull()
    api.repoStatus.mockClear()
    api.add.mockResolvedValue(undefined)
    useOpsStore().ops.find((o) => o.status === 'failed')!.retry!()
    await vi.waitFor(() => expect(api.add).toHaveBeenCalledTimes(2))
    await vi.waitFor(() => expect(api.repoStatus).toHaveBeenCalled())
    expect(c.busy).toBeNull()
  })
})

describe('repo store', () => {
  it('does not move to a folder until it validates', async () => {
    const repo = useRepoStore()
    repo.path = '/a'
    let release!: (v: unknown) => void
    api.repoStatus.mockReturnValueOnce(new Promise((r) => (release = r)))
    const p = repo.open('/b')
    expect(repo.path).toBe('/a')
    release(STATUS)
    expect(await p).toBe(true)
    expect(repo.path).toBe('/b')
  })

  it('a failed open leaves the open repo untouched and reports the error', async () => {
    const repo = useRepoStore()
    repo.path = '/a'
    api.repoStatus.mockRejectedValueOnce({ code: 'invalid_argument', message: 'not a repo' })
    expect(await repo.open('/b')).toBe(false)
    expect(repo.path).toBe('/a')
    expect(repo.error?.code).toBe('invalid_argument')
  })
})

describe('add batching', () => {
  it('caps argv length as well as count', () => {
    const long = 'x'.repeat(5000)
    const batches = batchPaths(Array.from({ length: 20 }, () => long))
    for (const b of batches) expect(b.reduce((n, p) => n + p.length + 1, 0)).toBeLessThanOrEqual(24_000)
    expect(batches.flat()).toHaveLength(20)
    expect(batches.length).toBeGreaterThan(1)
  })

  it('keeps the count cap and never drops an oversized path', () => {
    expect(batchPaths(Array.from({ length: 450 }, (_, i) => `f${i}`)).map((b) => b.length)).toEqual([200, 200, 50])
    expect(batchPaths(['a', 'y'.repeat(30_000), 'b'])).toEqual([['a'], ['y'.repeat(30_000)], ['b']])
    expect(batchPaths([])).toEqual([])
  })
})
