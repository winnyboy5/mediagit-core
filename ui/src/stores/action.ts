import { computed, ref } from 'vue'
import { toApiError, type ApiError, type Progress } from '@/api'
import { useOpsStore } from './ops'
import { useRepoStore } from './repo'

/**
 * One write at a time for a view's store: a busy flag, the last failure, and the running
 * op (for its progress bar). `run` resolves to the result, or null when it failed (the
 * error is in `error`). `after` refreshes the view's own data; the repo status always refreshes.
 */
export function useAction(after: () => Promise<unknown>) {
  const repo = useRepoStore()
  const ops = useOpsStore()
  const busy = ref(false)
  const error = ref<ApiError | null>(null)
  const opId = ref<string | null>(null)
  const running = computed(() => ops.ops.find((o) => o.id === opId.value && o.status === 'running'))

  async function run<T>(
    label: string,
    fn: (root: string, report: (p: Progress) => void) => Promise<T>,
    summarize: (r: T) => string | undefined,
    warn?: (r: T) => boolean,
    /** The call carries a secret: no Retry closure may keep it alive in the activity store. */
    secret = false,
    /** The view shows the result itself: no toast. */
    quiet = false,
  ): Promise<{ value: T } | null> {
    if (!repo.path || busy.value) return null
    const root = repo.path
    busy.value = true
    error.value = null
    try {
      const value = await ops.track(
        { kind: 'other', label, retry: secret ? undefined : () => void run(label, fn, summarize, warn, false, quiet), noRetry: secret, quiet },
        (report, id) => {
          opId.value = id
          return fn(root, report)
        },
        summarize,
        warn,
      )
      return { value }
    } catch (e) {
      error.value = toApiError(e)
      return null
    } finally {
      busy.value = false
      opId.value = null
      await Promise.all([after(), repo.refresh()])
    }
  }

  return { busy, error, running, run }
}
