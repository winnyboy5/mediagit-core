import { beforeEach, expect, it, vi } from 'vitest'
import { createPinia, getActivePinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({ pull: vi.fn(), repoStatus: vi.fn(), cancelOp: vi.fn(), mergeControl: vi.fn(), mergeResolve: vi.fn(), merge: vi.fn(), tagCreate: vi.fn(), tagList: vi.fn(), stashList: vi.fn(), log: vi.fn() }))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))
vi.mock('@/router', () => ({ router: { push: vi.fn() } }))

import { useMergeStore } from '@/stores/merge'
import { useRepoStore } from '@/stores/repo'
import { useSyncStore } from '@/stores/sync'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' } })
})

// QA S-10: Get latest conflict left "Keep their version" disabled (theirsOid null).
it('S-10: after Get latest conflicts, their commit comes from the repo operation', async () => {
  const repo = useRepoStore()
  repo.path = '/repos/hero'
  api.pull.mockResolvedValue({ action: 'merge', status: 'conflict', oid: null, conflicts: [{ path: 'n.txt', state: 'unresolved', kind: 'modify_modify' }] })
  await useSyncStore().run('pull')
  const merge = useMergeStore()
  expect(merge.theirsOid).toBeNull()
  repo.status = { branch: { name: 'main' }, operation: { theirs: 'abc123' } } as never
  expect(merge.theirsCommit).toBe('abc123')
  // The view must enable "Keep their version" from that fallback (this was the disabled button).
  const { flushPromises, mount } = await import('@vue/test-utils')
  const { createMemoryHistory, createRouter } = await import('vue-router')
  const { default: MergeView } = await import('@/views/MergeView.vue')
  const { useBranchesStore } = await import('@/stores/branches')
  useBranchesStore().loaded = true
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: MergeView }] })
  const w = mount(MergeView, { global: { plugins: [getActivePinia()!, router] } })
  await flushPromises()
  const keep = w.findAll('button').filter((b) => b.text().startsWith('Keep') && b.text().includes('their'))
  expect(keep.length).toBe(1)
  expect(keep[0]!.attributes('disabled')).toBeUndefined()
})

// QA B-04: merge --no-commit leaves a waiting merge with no conflicts; Finish must be offered.
it('B-04: a not_committed merge is active with nothing to decide and Finish runs continue', async () => {
  useRepoStore().path = '/repos/hero'
  const merge = useMergeStore()
  merge.apply({ action: 'merge', status: 'not_committed', oid: null, conflicts: [] } as never, 'feat', 'f00d')
  expect(merge.active).toBe(true)
  expect(merge.canContinue).toBe(true)
  api.mergeControl.mockResolvedValue({ action: 'merge', status: 'merged', oid: 'c0ffee', conflicts: [] })
  await merge.continue()
  expect(api.mergeControl).toHaveBeenCalledWith('/repos/hero', 'continue', expect.anything())
  expect(merge.active).toBe(false)
  expect(merge.theirsOid).toBeNull()
})

// QA T-01: switching settings section (a route update) must ask before dropping unsaved rows.
it('T-01: a section switch with an unsaved setting is held for the leave dialog', async () => {
  const { flushPromises, mount } = await import('@vue/test-utils')
  const { createMemoryHistory, createRouter, RouterView } = await import('vue-router')
  const { defineComponent, h } = await import('vue')
  const { default: SettingsView } = await import('@/views/SettingsView.vue')
  const { default: SettingRow } = await import('@/views/settings/SettingRow.vue')
  const { useRepoSettingsStore } = await import('@/stores/settings')
  const pinia = createPinia()
  setActivePinia(pinia)
  const rs = useRepoSettingsStore()
  rs.refresh = vi.fn()
  rs.loaded = true
  rs.schema = [{ key: 'x.y', group: 'x', type: 'string', secret: false, default: '', min: null, max: null } as never]
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/settings/:section', component: SettingsView }] })
  await router.push('/settings/a')
  const w = mount(defineComponent({ render: () => h(RouterView) }), { global: { plugins: [pinia, router] } })
  await flushPromises()
  w.findComponent(SettingRow).vm.$emit('dirty', 'x.y', true)
  void router.push('/settings/b')
  await flushPromises()
  expect(router.currentRoute.value.path).toBe('/settings/a')
})

// QA F-05: with nothing changed the save dock (and its Allow-empty option) must still be reachable.
it('F-05: the save dock shows when there are no changes', async () => {
  const { flushPromises, mount } = await import('@vue/test-utils')
  const { createMemoryHistory, createRouter } = await import('vue-router')
  const { default: ChangesView } = await import('@/views/ChangesView.vue')
  const pinia = createPinia()
  setActivePinia(pinia)
  useRepoStore().status = { branch: { name: 'main' }, staged: [], modified: [], untracked: [], deleted: [] } as never
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: ChangesView }] })
  const w = mount(ChangesView, { global: { plugins: [pinia, router] } })
  await flushPromises()
  expect(w.find('[data-testid="save-dock"]').exists()).toBe(true)
})

// QA H-06: a new tag must reach the version pill (history refs) without a manual reload.
it('H-06: creating a tag refreshes the history refs and keeps page and selection', async () => {
  const { useHistoryStore } = await import('@/stores/history')
  const { useTagsStashStore } = await import('@/stores/tagsStash')
  useRepoStore().path = '/repos/hero'
  api.tagCreate.mockResolvedValue({})
  api.tagList.mockResolvedValue({ tags: [] })
  api.stashList.mockResolvedValue({ stashes: [] })
  const h = useHistoryStore()
  h.loaded = true
  h.commits = [{ oid: 'a' }, { oid: 'b' }] as never
  h.selectedOid = 'b'
  api.log.mockResolvedValue({ commits: [{ oid: 'a' }], refs: [{ name: 'v3' }] })
  await useTagsStashStore().createTag('v3', { commit: 'abc' })
  expect(api.log).toHaveBeenCalled()
  expect(h.refs).toEqual([{ name: 'v3' }])
  expect(h.commits.length).toBe(2)
  expect(h.selectedOid).toBe('b')
})

// QA B-07: the stash list must re-read when the window regains focus (CLI stashes behind the app).
it('B-07: the stash page re-reads on window focus', async () => {
  const { flushPromises, mount } = await import('@vue/test-utils')
  const { default: TagsStashView } = await import('@/views/TagsStashView.vue')
  const pinia = createPinia()
  setActivePinia(pinia)
  useRepoStore().path = '/repos/hero'
  api.tagList.mockResolvedValue({ tags: [] })
  api.stashList.mockResolvedValue({ stashes: [] })
  mount(TagsStashView, { props: { only: 'stash' }, global: { plugins: [pinia] } })
  await flushPromises()
  const before = api.stashList.mock.calls.length
  window.dispatchEvent(new Event('focus'))
  await flushPromises()
  expect(api.stashList.mock.calls.length).toBeGreaterThan(before)
})

// QA review: squash + no-commit leaves no MERGE_HEAD, so it must not become an active merge.
it('B-04: squash with no-commit does not open a Finish-able merge', async () => {
  useRepoStore().path = '/repos/hero'
  api.merge.mockResolvedValue({ action: 'merge', status: 'not_committed', oid: null, conflicts: [] })
  const merge = useMergeStore()
  await merge.start('feat', 'recursive' as never, { squash: true, noCommit: true })
  expect(api.merge).toHaveBeenCalled()
  expect(merge.active).toBe(false)
})
