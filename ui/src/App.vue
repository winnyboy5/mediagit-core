<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import { api } from '@/api'
import Toast from '@/components/Toast.vue'
import ActivityBar from '@/shell/ActivityBar.vue'
import ActivityDrawer from '@/shell/ActivityDrawer.vue'
import Inspector from '@/shell/Inspector.vue'
import Sidebar from '@/shell/Sidebar.vue'
import { useRepoStore } from '@/stores/repo'
import { useShellStore } from '@/stores/shell'

const route = useRoute()
const router = useRouter()
const repo = useRepoStore()
const shell = useShellStore()
shell.watchViewport()

const hasChrome = computed(() => route.meta.chrome !== 'none')
const showInspector = computed(() => !!route.meta.inspector && shell.inspectorVisible)

// The overlay inspector and the activity drawer float over the view; a scrim dims it and a click dismisses.
const scrimOn = computed(() => shell.activityOpen || (shell.narrow && showInspector.value))
function closeOverlays() {
  shell.activityOpen = false
  shell.overlayOpen = false
}

const VIEWS = ['/changes', '/history', '/branches', '/locks', '/sync', '/merge', '/compare', '/tags']
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && shell.narrow && shell.overlayOpen && !shell.activityOpen) shell.overlayOpen = false
  if (!e.ctrlKey || e.altKey || e.shiftKey || !repo.path) return
  const target = VIEWS[Number(e.key) - 1]
  if (target) {
    e.preventDefault()
    void router.push(target)
  }
}

// Status goes stale when files change outside the app: refresh on window
// focus and on the watcher's `repo-changed` event, coalesced.
let timer: ReturnType<typeof setTimeout> | undefined
function refreshSoon() {
  if (!repo.path || route.meta.chrome === 'none') return
  clearTimeout(timer)
  timer = setTimeout(() => void repo.refresh(), 150)
}
let unlisten: (() => void) | undefined
onMounted(async () => {
  window.addEventListener('keydown', onKey)
  window.addEventListener('focus', refreshSoon)
  if (repo.path && !repo.status && route.meta.chrome !== 'none') void repo.refresh()
  try {
    unlisten = await api.onRepoChanged(refreshSoon)
  } catch {
    // No event bus outside Tauri; focus refresh still works.
  }
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey)
  window.removeEventListener('focus', refreshSoon)
  clearTimeout(timer)
  unlisten?.()
})
</script>

<template>
  <div class="app">
    <div class="body">
      <Sidebar v-if="hasChrome" />
      <main id="main" class="center"><RouterView /></main>
      <div v-if="scrimOn" class="scrim" aria-hidden="true" data-testid="scrim" @click="closeOverlays" />
      <Inspector v-if="showInspector" :class="{ overlay: shell.narrow }" />
    </div>
    <ActivityBar />
    <ActivityDrawer />
    <Toast />
  </div>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.body {
  position: relative;
  display: flex;
  flex: 1;
  min-height: 0;
}
.center {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.scrim {
  position: absolute;
  z-index: 20;
  inset: 0;
  background: var(--scrim);
}
.overlay {
  position: absolute;
  z-index: 30;
  top: 0;
  right: 0;
  bottom: 0;
  box-shadow: var(--shadow-3);
}
</style>
