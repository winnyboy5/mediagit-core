import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'
import { api, toApiError, type ApiError, type StatusReport } from '@/api'
import { forget, load, save } from '@/lib/persist'
import { useRecentStore } from './recent'
import { useUnlockStore } from './unlock'

const KEY = 'mg.repo'

/** The open repository and its working-copy status. */
export const useRepoStore = defineStore('repo', () => {
  const path = ref<string | null>(load<string | null>(KEY, null))
  // Shallow: a 100k-file status must not become 100k reactive proxies.
  const status = shallowRef<StatusReport | null>(null)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  let token = 0
  // The backend keeps one repo folder in the asset scope; opening another replaces it.
  let allowed: string | null = null

  function allow(p: string) {
    if (allowed === p) return
    allowed = p
    api.allowRepo(p).catch(() => {
      if (allowed === p) allowed = null
    })
  }

  /** Tell the desktop side; a failure must never block opening or closing. */
  async function remember(p: string | null) {
    try {
      await api.lastRepoSet(p)
    } catch {
      // Not in the desktop app, or the app data folder is not writable.
    }
  }

  /**
   * Startup: prefer the desktop side's record over localStorage, which
   * WebView2 may not have flushed when the app was killed (R-03). Never
   * rejects; skipped if a repository was opened meanwhile.
   */
  async function restoreLast() {
    const before = path.value
    let last: string | null = null
    try {
      // The app mounts after this: never let a stuck call keep the window blank.
      last = await Promise.race([api.lastRepoGet(), new Promise<null>((r) => setTimeout(() => r(null), 1000))])
    } catch {
      return
    }
    if (last && path.value === before && last !== before) {
      path.value = last
      save(KEY, last)
    }
  }

  /** Leaving a repository forgets its session passphrase. */
  function lock(p: string) {
    useUnlockStore().lock(p)
  }

  async function refresh() {
    if (!path.value) return
    const mine = ++token
    loading.value = true
    error.value = null
    try {
      const s = await api.repoStatus(path.value)
      if (mine !== token) return
      status.value = s
      // Previews load working files through the asset protocol, which needs the folder allowed.
      allow(path.value)
    } catch (e) {
      if (mine !== token) return
      status.value = null
      error.value = toApiError(e)
    } finally {
      if (mine === token) loading.value = false
    }
  }

  /** Open a repository; returns false (and stays closed) when it cannot be read. */
  async function open(dir: string): Promise<boolean> {
    // Opening again resumes the passphrase prompt, and the status call below may need it.
    useUnlockStore().resume(dir)
    // Validate before touching `path`: other stores watch it and reset on change.
    let s: StatusReport
    try {
      s = await api.repoStatus(dir)
    } catch (e) {
      error.value = toApiError(e)
      return false
    }
    token++ // supersede any refresh of the previous repo
    // The picked folder may be inside the repository; the app works on its root.
    const root = s.root
    if (path.value && path.value !== root) lock(path.value)
    useUnlockStore().resume(root)
    path.value = root
    status.value = s
    error.value = null
    loading.value = false
    allow(root)
    save(KEY, root)
    await remember(root)
    useRecentStore().add(root, (status.value as StatusReport | null)?.branch.name)
    return true
  }

  function close() {
    token++
    if (path.value) lock(path.value)
    path.value = null
    status.value = null
    error.value = null
    loading.value = false
    forget(KEY)
    void remember(null)
  }

  return { path, status, loading, error, open, refresh, close, restoreLast }
})
