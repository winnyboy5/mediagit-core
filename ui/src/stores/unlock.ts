import { defineStore } from 'pinia'
import { reactive, ref, watch } from 'vue'
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
  /**
   * Repositories whose prompt the person cancelled. Requests the person did not just make (commands
   * from the api layer, a view's refresh on mount) do not ask again: they fail as locked and the
   * view offers Unlock. A button (unlock, run, recalculate), opening the repo again, or a restart prompts.
   */
  const declined = reactive(new Set<string>())
  /** Set when the previous answer did not work. */
  const retry = ref(false)
  let waiting: ((answer: string | null) => void) | null = null
  /** Prompts run one at a time: a second asker waits its turn instead of replacing the first. */
  let turn: Promise<unknown> = Promise.resolve()

  /** Set while a one-time recovery code is on screen: the prompt waits so it never covers the code. */
  const hold = ref(false)
  const released = () => new Promise<void>((done) => {
    if (!hold.value) return done()
    const stop = watch(hold, (h) => !h && (stop(), done()), { flush: 'sync' })
  })

  function ask(again: boolean): Promise<string | null> {
    const asked = turn.then(released).then(
      () =>
        new Promise<string | null>((resolve) => {
          retry.value = again
          open.value = true
          waiting = resolve
        }),
    )
    turn = asked
    return asked
  }

  function answer(passphrase: string | null) {
    open.value = false
    const w = waiting
    waiting = null
    w?.(passphrase)
  }

  /**
   * Run `fn`; if it needs the passphrase, ask and run it again (until it works or is cancelled).
   * Pass `implicitFor` (the repository) for a call nobody clicked, such as a refresh on mount: after
   * a Cancel it fails as `needs_input` instead of asking again.
   */
  async function withUnlock<T>(fn: (passphrase?: string) => Promise<T>, implicitFor?: string): Promise<T> {
    let pass: string | undefined
    for (;;) {
      try {
        return await fn(pass)
      } catch (e) {
        if (toApiError(e).code !== 'needs_input') throw e
        if (implicitFor && declined.has(implicitFor)) throw e
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
   * This is a person asking to unlock (a button): it prompts even after an earlier Cancel.
   */
  function unlockRepo(repo: string): Promise<boolean> {
    declined.delete(repo)
    const existing = asking.get(repo)
    if (existing) return existing
    // A command just needed the passphrase, so whatever we believed before is stale.
    unlocked.delete(repo)
    locked.add(repo)
    const job = (async () => {
      let again = false
      for (;;) {
        const pass = await ask(again)
        if (pass === null) {
          declined.add(repo)
          return false
        }
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

  /**
   * A command needs the passphrase. After an earlier Cancel for this repository it does not ask
   * again: `'declined'` lets the command fail with its own `needs_input`, which the view shows.
   */
  function unlockForCommand(repo: string): Promise<boolean | 'declined'> {
    return declined.has(repo) ? Promise.resolve('declined') : unlockRepo(repo)
  }

  /** The repository was opened or switched to: prompting resumes. */
  function resume(repo: string) {
    declined.delete(repo)
  }

  /** Forget the session passphrase for `repo`; its next encrypted read asks again. */
  function lock(repo: string) {
    declined.delete(repo)
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

  return { open, hold, retry, unlocked, locked, markLocked, resume, answer, unlockForCommand, withUnlock, unlockRepo, lock }
})
