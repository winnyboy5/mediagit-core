import { defineStore } from 'pinia'
import { ref } from 'vue'
import { toApiError } from '@/api'

/**
 * The encryption passphrase prompt. A command that reaches a passphrase prompt fails with
 * `needs_input`; `withUnlock` asks the person, then runs the command again with the answer.
 *
 * The passphrase lives only in a local variable of one `withUnlock` call and in the dialog's
 * own field (which clears itself on submit). It is never stored, persisted or logged.
 */
export const useUnlockStore = defineStore('unlock', () => {
  const open = ref(false)
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

  return { open, retry, answer, withUnlock }
})
