import { defineStore } from 'pinia'
import { computed, onScopeDispose, ref } from 'vue'
import { load, save } from '@/lib/persist'

/** Below this width the inspector is an overlay drawer instead of a column. */
export const NARROW_PX = 1200

/** Shell layout state: inspector, activity drawer, viewport class. */
export const useShellStore = defineStore('shell', () => {
  const inspectorOpen = ref<boolean>(load('mg.inspector', true))
  /** Narrow viewports show the inspector as an overlay, closed until asked for. */
  const overlayOpen = ref(false)
  const activityOpen = ref(false)
  const narrow = ref(typeof window !== 'undefined' && window.innerWidth < NARROW_PX)

  const inspectorVisible = computed(() => (narrow.value ? overlayOpen.value : inspectorOpen.value))

  function toggleInspector() {
    if (narrow.value) {
      overlayOpen.value = !overlayOpen.value
      return
    }
    inspectorOpen.value = !inspectorOpen.value
    save('mg.inspector', inspectorOpen.value)
  }

  /** Track the viewport; call once from the shell. */
  function watchViewport() {
    if (typeof window === 'undefined' || !window.matchMedia) return
    const mq = window.matchMedia(`(max-width: ${NARROW_PX - 1}px)`)
    const on = () => (narrow.value = mq.matches)
    on()
    mq.addEventListener('change', on)
    onScopeDispose(() => mq.removeEventListener('change', on))
  }

  return { inspectorOpen, overlayOpen, inspectorVisible, activityOpen, narrow, toggleInspector, watchViewport }
})
