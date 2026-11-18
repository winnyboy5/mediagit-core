import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'

const api = vi.hoisted(() => ({
  tagList: vi.fn(),
  stashList: vi.fn(),
  stashSave: vi.fn(),
  allowRepo: vi.fn(),
  repoStatus: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import TagsStashView from '@/views/TagsStashView.vue'
import { useRepoStore } from '@/stores/repo'

enableAutoUnmount(afterEach)

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.tagList.mockResolvedValue({ tags: [] })
  api.stashList.mockResolvedValue({ stashes: [] })
  api.allowRepo.mockResolvedValue(undefined)
  api.stashSave.mockResolvedValue({ oid: 'aa' })
})

describe('BUG-027: stash has no "Include new files" choice', () => {
  const mountStash = () => {
    const repo = useRepoStore()
    repo.path = '/repos/hero'
    repo.status = {
      root: '/repos/hero',
      branch: { name: 'main' },
      staged: [],
      modified: [],
      deleted: [],
      untracked: ['new.psd'],
      summary: { staged: 0, modified: 0, deleted: 0, untracked: 1 },
    } as never
    return mount(TagsStashView, { props: { only: 'stash' } })
  }

  it('shows no checkbox for it, and new files are stashed because the CLI always takes them', async () => {
    const w = mountStash()
    await flushPromises()
    expect(w.text()).not.toContain('Include new files')
    const save = w.findAll('button').find((b) => b.text() === 'Stash my changes')!
    expect(save.attributes('disabled')).toBeUndefined()
    await w.get('form.save').trigger('submit')
    await flushPromises()
    expect(api.stashSave).toHaveBeenCalledWith('/repos/hero', null, true, [], expect.any(Function))
  })
})
