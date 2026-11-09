import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type ConfigEntry, type ConfigKeySchema, type SparseReport } from '@/api'
import { load, save } from '@/lib/persist'
import { useAction } from './action'
import { useIdentityStore } from './identity'
import { useRepoStore } from './repo'

/** One row of the settings form. `name` is the remote's name for a `remotes.<name>.*` key. */
export interface SettingField {
  spec: ConfigKeySchema
  key: string
  name: string | null
  group: string
  value: string | null
}

/** `pattern` segments (`<name>` = any one) against a concrete key's segments. */
const matches = (pattern: string[], segs: string[]) => pattern.length === segs.length && pattern.every((p, i) => p === '<name>' || p === segs[i])

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
  /** Every setting the CLI knows; the form is built from this, not from a fixed list. */
  const schema = shallowRef<ConfigKeySchema[]>([])
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
    const [cfg, sp, sc] = await Promise.allSettled([api.configList(root), api.sparseList(root), api.configSchema()])
    if (mine !== token) return
    if (cfg.status === 'fulfilled') {
      entries.value = cfg.value.entries
      loaded.value = true
    } else error.value = toApiError(cfg.reason)
    schema.value = sc.status === 'fulfilled' ? sc.value.keys : []
    sparse.value = sp.status === 'fulfilled' ? sp.value : null
    loading.value = false
    void identity.refresh()
  }

  const { busy, error: actionError, running, run } = useAction(refresh)

  /** A `secret` value goes to the CLI privately; it is not in the activity label, summary or retry. */
  const setKey = (key: string, value: string, secret = false) =>
    run(`Save ${key}`, (root) => api.configSet(root, key, value, secret), () => 'Saved', undefined, secret)
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

  /**
   * The form's rows: one per schema key, and for a `<name>` key one per name that exists
   * (the CLI lists those). The author keys have their own card and are left out.
   */
  const fields = computed<SettingField[]>(() => {
    const out: SettingField[] = []
    for (const spec of schema.value) {
      if (spec.key.startsWith('author.')) continue
      const parts = spec.key.split('.')
      const at = parts.indexOf('<name>')
      const keys = at < 0 ? [spec.key] : entries.value.map((e) => e.key).filter((k) => matches(parts, k.split('.')))
      for (const key of keys) {
        const name = at < 0 ? null : key.split('.')[at]!
        out.push({ spec, key, name, group: parts[0]!, value: value(key) })
      }
    }
    return out
  })

  watch(
    () => repo.path,
    () => {
      token++
      entries.value = []
      schema.value = []
      sparse.value = null
      loaded.value = false
      error.value = null
      actionError.value = null
    },
  )

  return { entries, schema, fields, sparse, loaded, loading, error, actionError, busy, running, value, refresh, setKey, unsetKey, setAuthor, applySparse, disableSparse }
})
