import { onBeforeUnmount, onMounted } from 'vue'

/** Re-read a view's data when the window regains focus (replaces manual refresh buttons). */
export function useFocusRefresh(fn: () => unknown) {
  const run = () => void fn()
  onMounted(() => window.addEventListener('focus', run))
  onBeforeUnmount(() => window.removeEventListener('focus', run))
}
