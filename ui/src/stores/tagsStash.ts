import { defineStore } from 'pinia'
import { ref, shallowRef, watch } from 'vue'
import {
  api,
  toApiError,
  type ApiError,
  type StashEntryReport,
  type StashRestore,
  type TagOptions,
  type TagReport,
  type TagShowReport,
  type TagVerifyReport,
} from '@/api'
import { useAction } from './action'
import { useRepoStore } from './repo'

/** What the tag detail dialog shows: the tag's own data and, once asked, its signature check. */
export interface TagDetail {
  name: string
  show: TagShowReport | null
  verify: TagVerifyReport | null
  /** Why the signature check failed (an invalid signature is an error, not a report). */
  verifyError: ApiError | null
  error: ApiError | null
  loading: boolean
}

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
  /** A glob that narrows the tag list (`tag list PATTERN`); empty shows every tag. */
  const filter = ref('')
  const detail = shallowRef<TagDetail | null>(null)
  let token = 0

  async function refresh() {
    if (!repo.path) return
    const mine = ++token
    const root = repo.path
    loading.value = true
    tagsError.value = null
    stashError.value = null
    await Promise.all([
      api.tagList(root, filter.value.trim() || null).then(
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

  const createTag = (name: string, options: TagOptions = {}) =>
    run(
      `Create tag ${name}`,
      (root) => api.tagCreate(root, name, { ...options, message: options.message?.trim() || null }),
      () => (options.force ? `Replaced tag ${name}` : options.commit ? `Tagged ${name}` : `Tagged the latest version ${name}`),
    )
  const deleteTag = (name: string) =>
    run(`Delete tag ${name}`, (root) => api.tagDelete(root, name), (r) => (r.missing.length ? 'Tag was already gone' : `Deleted tag ${name}`))

  async function showTag(name: string) {
    const root = repo.path
    if (!root) return
    const d: TagDetail = { name, show: null, verify: null, verifyError: null, error: null, loading: true }
    detail.value = d
    try {
      d.show = await api.tagShow(root, name)
    } catch (e) {
      d.error = toApiError(e)
    }
    d.loading = false
    if (detail.value?.name === name) detail.value = { ...d }
  }
  async function verifyTag() {
    const root = repo.path
    const d = detail.value
    if (!root || !d) return
    detail.value = { ...d, verify: null, verifyError: null, loading: true }
    let next: TagDetail
    try {
      next = { ...d, verify: await api.tagVerify(root, d.name), verifyError: null, loading: false }
    } catch (e) {
      next = { ...d, verify: null, verifyError: toApiError(e), loading: false }
    }
    if (detail.value?.name === d.name) detail.value = next
  }
  const closeTag = () => (detail.value = null)

  /** `paths` empty = everything that changed; otherwise only those files ("Shelve selected files"). */
  const save = (message: string, includeUntracked: boolean, paths: string[] = []) =>
    run(
      paths.length ? 'Shelve selected files' : 'Stash changes',
      (root, report) => api.stashSave(root, message.trim() || null, includeUntracked, paths, report),
      (r) => (r.oid ? (paths.length ? `Set aside ${paths.length} file${paths.length === 1 ? '' : 's'}` : 'Changes set aside') : 'Nothing to stash'),
    )
  const apply = (i: number, opts: StashRestore = {}) =>
    run(`Apply stash ${i}`, (root, report) => api.stashApply(root, i, opts, report), (r) => `Applied to ${r.files_updated ?? 0} files`)
  const pop = (i: number, opts: StashRestore = {}) =>
    run(`Pop stash ${i}`, (root, report) => api.stashPop(root, i, opts, report), (r) => `Applied to ${r.files_updated ?? 0} files and removed`)
  const drop = (i: number) => run(`Drop stash ${i}`, (root) => api.stashDrop(root, i), () => 'Stash removed')
  const clear = () => run('Clear all stashes', (root) => api.stashClear(root), () => 'All stashes removed')

  /** One entry's own details (`stash show`). */
  async function showStash(i: number): Promise<StashEntryReport | null> {
    const root = repo.path
    if (!root) return null
    try {
      return (await api.stashShow(root, i)).stashes[0] ?? null
    } catch (e) {
      actionError.value = toApiError(e)
      return null
    }
  }

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
      filter.value = ''
      detail.value = null
    },
  )

  return {
    tags, stashes, tagsLoaded, stashLoaded, loading, tagsError, stashError, filter, detail, actionError, busy, running,
    refresh, createTag, deleteTag, showTag, verifyTag, closeTag, save, apply, pop, drop, clear, showStash,
  }
})
