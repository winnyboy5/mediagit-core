import { beforeEach, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({ pull: vi.fn(), repoStatus: vi.fn(), cancelOp: vi.fn() }))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))
const push = vi.hoisted(() => vi.fn())
vi.mock('@/router', () => ({ router: { push } }))

import { useMergeStore } from '@/stores/merge'
import { useRepoStore } from '@/stores/repo'
import { useSyncStore } from '@/stores/sync'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' } })
})

it('Get latest that hits a clash opens the merge page with the conflicts', async () => {
  useRepoStore().path = '/repos/hero'
  api.pull.mockResolvedValue({
    action: 'merge',
    status: 'conflict',
    oid: null,
    conflicts: [{ path: 'notes.txt', state: 'unresolved', kind: 'modify_modify' }],
  })
  const ok = await useSyncStore().run('pull')
  expect(ok).toBe(true)
  expect(push).toHaveBeenCalledWith('/branches/merge')
  expect(useMergeStore().active).toBe(true)
  expect(useMergeStore().conflicts.map((c) => c.path)).toEqual(['notes.txt'])
})

it('a normal pull stays where it is', async () => {
  useRepoStore().path = '/repos/hero'
  api.pull.mockResolvedValue({ remote: 'origin', branch: 'main', result: 'fast_forward', old_head: null, new_head: 'ab', tracking_refs_updated: [], bytes_downloaded: 0, objects_received: 0 })
  await useSyncStore().run('pull')
  expect(push).not.toHaveBeenCalled()
  expect(useMergeStore().active).toBe(false)
})
