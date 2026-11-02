import { defineStore } from 'pinia'
import { reactive, ref } from 'vue'
import { api, toApiError } from '@/api'

/**
 * The encryption passphrase prompt. A command that reaches a passphrase prompt fails with
 * `needs_input`; `withUnlock` asks the person, then runs the command again with the answer.
 *
 * The passphrase lives only in a local variable of one `withUnlock` call and in the dialog's
 * own field (which clears itself on submit). It is never stored, persisted or logged.
 */
export const useUnlockStore = defineStore('unlock', () => {
  const open = ref(false)
  /** Repositories unlocked for this session (the backend holds the passphrase; this is only the fact). */
  const unlocked = reactive(new Set<string>())
  /** Repositories a command found locked (`needs_input`) and that are not unlocked now. */
  const locked = reactive(new Set<string>())
  /** Set when the previous answer did not work. */
  const retry = ref(false)
  let waiting: ((answer: string | null) => void) | null = null

  function ask(again: boolean): Promise<string | null> {
    retry.value = again
    open.value = true
    return new Promise((resolve) => (waiting = resolve))
  }

  function answer(passphrase: string | null) {
    open.value = false
    const w = waiting
    waiting = null
    w?.(passphrase)
  }

  /** Run `fn`; if it needs the passphrase, ask and run it again (until it works or is cancelled). */
  async function withUnlock<T>(fn: (passphrase?: string) => Promise<T>): Promise<T> {
    let pass: string | undefined
    for (;;) {
      try {
        return await fn(pass)
      } catch (e) {
        if (toApiError(e).code !== 'needs_input') throw e
        const next = await ask(pass !== undefined)
        if (next === null) throw { code: 'cancelled', message: 'Cancelled' }
        pass = next
      }
    }
  }

  /** Prompts being answered, by repository: commands that fail together share one dialog. */
  const asking = new Map<string, Promise<boolean>>()

  /**
   * Unlock `repo` for the session: ask until the backend accepts the passphrase or the person
   * cancels. The passphrase goes straight to `unlock_repo` and nowhere else.
   */
  function unlockRepo(repo: string): Promise<boolean> {
    const existing = asking.get(repo)
    if (existing) return existing
    // A command just needed the passphrase, so whatever we believed before is stale.
    unlocked.delete(repo)
    locked.add(repo)
    const job = (async () => {
      let again = false
      for (;;) {
        const pass = await ask(again)
        if (pass === null) return false
        try {
          await api.unlockRepo(repo, pass)
          unlocked.add(repo)
          locked.delete(repo)
          return true
        } catch (e) {
          // Only a rejected passphrase is asked about again; any other failure is the real answer.
          if (toApiError(e).code !== 'wrong_passphrase') throw e
          again = true
        }
      }
    })().finally(() => asking.delete(repo))
    asking.set(repo, job)
    return job
  }

  /** Forget the session passphrase for `repo`; its next encrypted read asks again. */
  function lock(repo: string) {
    unlocked.delete(repo)
    locked.add(repo)
    void Promise.resolve()
      .then(() => api.lockRepo(repo))
      .catch(() => {})
  }

  /** The backend still wants the passphrase after an unlock: the "unlocked" fact was wrong. */
  function markLocked(repo: string) {
    unlocked.delete(repo)
    locked.add(repo)
  }

  return { open, retry, unlocked, locked, markLocked, answer, withUnlock, unlockRepo, lock }
})
