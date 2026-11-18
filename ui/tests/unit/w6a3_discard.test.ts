import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'

vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api: {} }))

import UndoDialogs from '@/views/history/UndoDialogs.vue'
import { useRepoStore } from '@/stores/repo'
import { useUndoStore } from '@/stores/undo'

enableAutoUnmount(afterEach)
beforeEach(() => setActivePinia(createPinia()))

describe('BUG-017: the Discard dialog names the files', () => {
  it('lists changed files and calls out new files that will be deleted', async () => {
    const repo = useRepoStore()
    repo.path = '/repos/hero'
    repo.status = {
      root: '/repos/hero',
      branch: { name: 'main' },
      staged: [
        { path: 'new-included.txt', kind: 'added' },
        { path: 'staged-edit.psd', kind: 'modified' },
      ],
      modified: ['edited.psd'],
      deleted: ['gone.psd'],
      untracked: ['new-untracked.txt'],
      summary: { staged: 2, modified: 1, deleted: 1, untracked: 1 },
    } as never
    const w = mount(UndoDialogs, { attachTo: document.body })
    useUndoStore().ask({ kind: 'reset', mode: 'hard', rev: 'HEAD' })
    await flushPromises()
    const text = document.body.textContent ?? ''
    expect(text).toContain('1 new file will be deleted')
    expect(text).toContain('new-included.txt')
    expect(text).toContain('edited.psd')
    expect(text).toContain('gone.psd')
    expect(text).not.toContain('new-untracked.txt')
    expect(text).toContain('1 new file you have not included is kept as it is')
    w.unmount()
  })
})

describe('IMP-031: discard speaks of the latest version, not HEAD', () => {
  it('names the operation and the result in plain words', async () => {
    const { api } = await import('@/api')
    ;(api as Record<string, unknown>).resetHard = vi.fn().mockResolvedValue({})
    useRepoStore().path = '/repos/hero'
    const undo = useUndoStore()
    undo.ask({ kind: 'reset', mode: 'hard', rev: 'HEAD' })
    await undo.confirm()
    const { useToast } = await import('@/components/useToast')
    const text = JSON.stringify(useToast().items)
    expect(text).toContain('Discarded unsaved changes. Files match the latest version.')
    expect(text).not.toContain('HEAD')
  })
})
