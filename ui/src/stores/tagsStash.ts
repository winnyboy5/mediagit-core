import { defineStore } from 'pinia'
import { ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type StashEntryReport, type TagReport } from '@/api'
import { useAction } from './action'
import { useRepoStore } from './repo'

/** Tags and the stash. */
export const useTagsStashStore = defineStore('tagsStash', () => {
  const repo = useRepoStore()

  const tags = shallowRef<TagReport[]>([])
  const stashes = shallowRef<StashEntryReport[]>([])
  const tagsLoaded = ref(false)
  const stashLoaded = ref(false)
  const loading = ref(false)
  const tagsError = ref<ApiError | null>(null)
  const stashError = ref<ApiError | null>(null)
  let token = 0

  async function refresh() {
    if (!repo.path) return
    const mine = ++token
    const root = repo.path
    loading.value = true
    tagsError.value = null
    stashError.value = null
    await Promise.all([
      api.tagList(root).then(
        (r) => {
          if (mine !== token) return
          tags.value = r.tags
          tagsLoaded.value = true
        },
        (e) => {
          if (mine === token) tagsError.value = toApiError(e)
        },
      ),
      api.stashList(root).then(
        (r) => {
          if (mine !== token) return
          stashes.value = r.stashes
          stashLoaded.value = true
        },
        (e) => {
          if (mine === token) stashError.value = toApiError(e)
        },
      ),
    ])
    if (mine === token) loading.value = false
  }

  const { busy, error: actionError, running, run } = useAction(refresh)

  const createTag = (name: string, message: string) =>
    run(`Create tag ${name}`, (root) => api.tagCreate(root, name, null, message.trim() || null), () => `Tagged the latest version ${name}`)
  const deleteTag = (name: string) =>
    run(`Delete tag ${name}`, (root) => api.tagDelete(root, name), (r) => (r.missing.length ? 'Tag was already gone' : `Deleted tag ${name}`))

  const save = (message: string, includeUntracked: boolean) =>
    run(
      'Stash changes',
      (root, report) => api.stashSave(root, message.trim() || null, includeUntracked, report),
      (r) => (r.oid ? 'Changes set aside' : 'Nothing to stash'),
    )
  const apply = (i: number) => run(`Apply stash ${i}`, (root, report) => api.stashApply(root, i, report), (r) => `Applied to ${r.files_updated ?? 0} files`)
  const pop = (i: number) => run(`Pop stash ${i}`, (root, report) => api.stashPop(root, i, report), (r) => `Applied to ${r.files_updated ?? 0} files and removed`)
  const drop = (i: number) => run(`Drop stash ${i}`, (root) => api.stashDrop(root, i), () => 'Stash removed')
  const clear = () => run('Clear all stashes', (root) => api.stashClear(root), () => 'All stashes removed')

  watch(
    () => repo.path,
    () => {
      token++
      tags.value = []
      stashes.value = []
      tagsLoaded.value = false
      stashLoaded.value = false
      tagsError.value = null
      stashError.value = null
      actionError.value = null
    },
  )

  return { tags, stashes, tagsLoaded, stashLoaded, loading, tagsError, stashError, actionError, busy, running, refresh, createTag, deleteTag, save, apply, pop, drop, clear }
})
