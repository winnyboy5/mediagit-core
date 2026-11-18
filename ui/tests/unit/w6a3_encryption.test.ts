import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'

const api = vi.hoisted(() => ({ authRun: vi.fn(), keyStatus: vi.fn(), allowRepo: vi.fn() }))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))
vi.mock('vue-router', () => ({ onBeforeRouteLeave: vi.fn(), useRouter: () => ({ push: vi.fn() }) }))

import SecurityView from '@/views/SecurityView.vue'
import { useRepoStore } from '@/stores/repo'

enableAutoUnmount(afterEach)
beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.authRun.mockResolvedValue({ identity: null })
  api.keyStatus.mockResolvedValue({ action: 'status', enabled: false, source: null, wrapped_key: null, recovery_slot: null, recovery_code: null })
})

async function view(hasCommits: boolean) {
  const repo = useRepoStore()
  repo.path = '/repos/hero'
  repo.status = { root: '/repos/hero', branch: { name: 'main', detached: false, detached_oid: null, has_commits: hasCommits, upstream: null }, staged: [], modified: [], deleted: [], untracked: [] } as never
  const w = mount(SecurityView)
  await flushPromises()
  return w
}

describe('BUG-037: encryption on a repository that already has versions', () => {
  it('offers the button on an empty repository', async () => {
    const w = await view(false)
    expect(w.text()).toContain('Turn on encryption')
  })

  it('shows one plain sentence instead of the button once a version exists', async () => {
    const w = await view(true)
    expect(w.findAll('button').some((b) => b.text().includes('Turn on encryption'))).toBe(false)
    expect(w.text()).toContain('Encryption must be turned on before the first version is saved')
  })
})
