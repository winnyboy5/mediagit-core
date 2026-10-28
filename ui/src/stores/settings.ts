import { defineStore } from 'pinia'
import { ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type ConfigEntry, type SparseReport } from '@/api'
import { load, save } from '@/lib/persist'
import { useAction } from './action'
import { useIdentityStore } from './identity'
import { useRepoStore } from './repo'

export type Theme = 'system' | 'light' | 'dark'
export type Density = 'comfortable' | 'compact'
const THEME_KEY = 'mg.theme'
const DENSITY_KEY = 'mg.density'

/** Put the chosen look on the page. `system` leaves it to the OS (no attribute). */
export function applyLook(theme: Theme, density: Density, root: HTMLElement = document.documentElement) {
  if (theme === 'system') root.removeAttribute('data-theme')
  else root.setAttribute('data-theme', theme)
  if (density === 'compact') root.setAttribute('data-density', 'compact')
  else root.removeAttribute('data-density')
}

/** How the app looks: per viewer, kept in the browser, applied at startup. */
export const useLookStore = defineStore('look', () => {
  const theme = ref<Theme>(load<Theme>(THEME_KEY, 'system'))
  const density = ref<Density>(load<Density>(DENSITY_KEY, 'comfortable'))
  watch(
    [theme, density],
    () => {
      save(THEME_KEY, theme.value)
      save(DENSITY_KEY, density.value)
      applyLook(theme.value, density.value)
    },
    { immediate: true },
  )
  return { theme, density }
})

/** Repository settings: author identity, other config keys, and sparse checkout. */
export const useRepoSettingsStore = defineStore('repoSettings', () => {
  const repo = useRepoStore()
  const identity = useIdentityStore()
  const entries = shallowRef<ConfigEntry[]>([])
  const sparse = shallowRef<SparseReport | null>(null)
  const loaded = ref(false)
  const loading = ref(false)
  const error = ref<ApiError | null>(null)
  let token = 0

  async function refresh() {
    const root = repo.path
    if (!root) return
    const mine = ++token
    loading.value = true
    error.value = null
    const [cfg, sp] = await Promise.allSettled([api.configList(root), api.sparseList(root)])
    if (mine !== token) return
    if (cfg.status === 'fulfilled') {
      entries.value = cfg.value.entries
      loaded.value = true
    } else error.value = toApiError(cfg.reason)
    sparse.value = sp.status === 'fulfilled' ? sp.value : null
    loading.value = false
    void identity.refresh()
  }

  const { busy, error: actionError, running, run } = useAction(refresh)

  const setKey = (key: string, value: string) => run(`Save ${key}`, (root) => api.configSet(root, key, value), () => 'Saved')
  const unsetKey = (key: string) => run(`Reset ${key}`, (root) => api.configUnset(root, key), () => 'Reset')
  const setAuthor = async (name: string, email: string) => (await setKey('author.name', name)) && (await setKey('author.email', email))

  async function applySparse(patterns: string[], patternMode: boolean) {
    const r = await run(
      'Choose which files to keep on disk',
      (root, report) => api.sparseSet(root, patterns, patternMode, report),
      (x) => `${x.files_added ?? 0} files added, ${x.files_removed ?? 0} removed`,
    )
    return !!r
  }
  async function disableSparse() {
    const r = await run('Keep every file on disk', (root, report) => api.sparseDisable(root, report), (x) => `${x.files_added ?? 0} files added`)
    return !!r
  }

  const value = (key: string) => entries.value.find((e) => e.key === key)?.value ?? null

  watch(
    () => repo.path,
    () => {
      token++
      entries.value = []
      sparse.value = null
      loaded.value = false
      error.value = null
      actionError.value = null
    },
  )

  return { entries, sparse, loaded, loading, error, actionError, busy, running, value, refresh, setKey, unsetKey, setAuthor, applySparse, disableSparse }
})
