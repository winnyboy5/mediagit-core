<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import { api } from '@/api'
import Toast from '@/components/Toast.vue'
import ActivityDrawer from '@/shell/ActivityDrawer.vue'
import UnlockDialog from '@/shell/UnlockDialog.vue'
import Inspector from '@/shell/Inspector.vue'
import { useInspectedFiles } from '@/shell/inspected'
import { HUBS, hubById, hubHome, sectionOf } from '@/shell/hubs'
import ActivityPill from '@/shell/ActivityPill.vue'
import Sidebar from '@/shell/Sidebar.vue'
import { useRepoStore } from '@/stores/repo'
import { useLocksStore } from '@/stores/locks'
import { useShellStore } from '@/stores/shell'
import { useTagsStashStore } from '@/stores/tagsStash'

const route = useRoute()
const router = useRouter()
const repo = useRepoStore()
const shell = useShellStore()
shell.watchViewport()

const hasChrome = computed(() => route.meta.chrome !== 'none')

// Section counts (Tags, Stash, Locks) need their lists even before those pages are opened.
const tagsStash = useTagsStashStore()
const locks = useLocksStore()
watch(
  () => (hasChrome.value ? repo.path : null),
  (p) => {
    if (!p) return
    void tagsStash.refresh()
    void locks.refresh()
  },
  { immediate: true },
)
const hub = computed(() => hubById(route.meta.hub))
const inspected = useInspectedFiles()
// Wide: the details panel is part of the layout; it appears once something is picked and gives its width back when nothing is. Narrow: an overlay on request.
const showInspector = computed(
  () =>
    !!hub.value &&
    !!sectionOf(hub.value, route.params.section as string | undefined).inspector &&
    (shell.narrow ? shell.overlayOpen : inspected.value.length > 0),
)

// The overlay inspector and the activity drawer float over the view; a scrim dims it and a click dismisses.
const scrimOn = computed(() => shell.activityOpen || (shell.narrow && showInspector.value))
function closeOverlays() {
  shell.activityOpen = false
  shell.overlayOpen = false
}

// Ctrl+1..4 hubs, Ctrl+, Settings, Alt+1..9 sections of the current hub.
const SHORTCUT_HUBS = HUBS.filter((h) => h.id !== 'settings')
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && shell.narrow && shell.overlayOpen && !shell.activityOpen) shell.overlayOpen = false
  if (!repo.path || e.shiftKey) return
  let target: string | undefined
  if (e.ctrlKey && !e.altKey) {
    const h = e.key === ',' ? HUBS.find((x) => x.id === 'settings') : SHORTCUT_HUBS[Number(e.key) - 1]
    if (h) target = hubHome(h)
  } else if (e.altKey && !e.ctrlKey && hub.value) {
    const sec = hub.value.sections[Number(e.key) - 1]
    if (sec) target = `/${hub.value.id}/${sec.id}`
  }
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
      <main id="main" class="center">
        <!-- Welcome has no hub header: a running clone or init still needs to be seen. -->
        <div v-if="!hasChrome" class="floatpill"><ActivityPill /></div>
        <RouterView />
      </main>
      <div v-if="scrimOn" class="scrim" aria-hidden="true" data-testid="scrim" @click="closeOverlays" />
      <Inspector v-if="showInspector" :class="{ overlay: shell.narrow }" />
    </div>
    <ActivityDrawer />
    <UnlockDialog />
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
.floatpill {
  position: absolute;
  z-index: 10;
  top: var(--space-4);
  right: var(--space-4);
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
