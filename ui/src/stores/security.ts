import { defineStore } from 'pinia'
import { ref, shallowRef, watch } from 'vue'
import { api, toApiError, type ApiError, type AuthReport, type KeyReport } from '@/api'
import { useAction } from './action'
import { useRepoStore } from './repo'

/**
 * Security: the signed-in account and the repository's encryption key.
 *
 * Secrets (password, passphrase, recovery code) are arguments of one call each: they are not
 * stored here, not in any op label or summary, and not persisted. The only secret this store
 * ever holds is the one-time `recoveryCode`, in memory, until the person confirms they saved it.
 */
export const useSecurityStore = defineStore('security', () => {
  const repo = useRepoStore()
  const auth = shallowRef<AuthReport | null>(null)
  const authError = ref<ApiError | null>(null)
  const key = shallowRef<KeyReport | null>(null)
  const keyError = ref<ApiError | null>(null)
  const loading = ref(false)
  /** Shown once after `key init`; cleared by `dismissRecovery`. Never persisted or logged. */
  const recoveryCode = ref<string | null>(null)
  /** How the new key unlocks on this machine (keychain, passphrase...), shown with the code. Not a secret. */
  const keySource = ref<string | null>(null)
  let token = 0

  async function refresh() {
    const root = repo.path
    if (!root) return
    const mine = ++token
    loading.value = true
    authError.value = keyError.value = null
    await Promise.all([
      api.authRun(root, 'status', null).then(
        (r) => mine === token && (auth.value = r),
        (e) => mine === token && ((auth.value = null), (authError.value = toApiError(e))),
      ),
      api.keyStatus(root).then(
        (r) => mine === token && (key.value = r),
        (e) => mine === token && ((key.value = null), (keyError.value = toApiError(e))),
      ),
    ])
    if (mine === token) loading.value = false
  }

  const { busy, error: actionError, run } = useAction(refresh)

  /** The password is passed on and forgotten; the caller clears its own field. */
  const login = (server: string, username: string, password: string) =>
    run('Sign in', (root) => api.authLogin(root, server || null, username || null, password), () => 'Signed in', undefined, true)
  const logout = (all: boolean) => run(all ? 'Sign out everywhere' : 'Sign out', (root) => api.authRun(root, 'logout', null, all), () => 'Signed out')

  async function enableEncryption(passphrase: string) {
    const r = await run('Turn on encryption', (root) => api.keyInit(root, passphrase || null), (k) => (k.source ? `Encryption is on, unlocks with ${k.source}` : 'Encryption is on'), undefined, true)
    if (r?.value.recovery_code) {
      recoveryCode.value = r.value.recovery_code
      keySource.value = r.value.source
    }
    return !!r
  }
  const recover = (code: string, passphrase: string) =>
    run('Recover key', (root) => api.keyRecover(root, code, passphrase || null), (k) => (k.source ? `Key recovered, unlocks with ${k.source}` : 'Key recovered'), undefined, true)

  function dismissRecovery() {
    recoveryCode.value = null
    keySource.value = null
  }

  watch(
    () => repo.path,
    () => {
      token++
      auth.value = key.value = null
      authError.value = keyError.value = actionError.value = null
      recoveryCode.value = null
      keySource.value = null
    },
  )

  return { auth, authError, key, keyError, loading, recoveryCode, keySource, busy, actionError, refresh, login, logout, enableEncryption, recover, dismissRecovery }
})
