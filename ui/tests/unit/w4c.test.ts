import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({ log: vi.fn(), diff: vi.fn() }))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { HUBS, inHub } from '@/shell/hubs'
import { useCompareStore } from '@/stores/compare'
import { useRepoStore } from '@/stores/repo'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
})

describe('rail active state', () => {
  const hub = (id: string) => HUBS.find((h) => h.id === id)!
  it('is active on every section of a hub, not only the first', () => {
    expect(inHub('/branches/branches', hub('branches'))).toBe(true)
    expect(inHub('/branches/merge', hub('branches'))).toBe(true)
    expect(inHub('/team/server/people', hub('team'))).toBe(true)
    expect(inHub('/history/tags', hub('branches'))).toBe(false)
    expect(inHub('/settings/storage', hub('settings'))).toBe(true)
  })
})

describe('compare store', () => {
  it('treats a null diff report as "nothing differs" instead of crashing', async () => {
    useRepoStore().path = '/repos/hero'
    api.log.mockResolvedValue({ commits: [{ oid: 'a'.repeat(40), summary: 'one' }] })
    api.diff.mockResolvedValue(null)
    const c = useCompareStore()
    await c.loadCommits()
    await c.compare()
    expect(c.error).toBeNull()
    expect(c.files).toEqual([])
  })
})
