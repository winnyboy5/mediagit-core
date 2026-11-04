<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { FolderOpen, FolderPlus, GitFork, Server, X } from 'lucide-vue-next'
import Wordmark from '@/components/Wordmark.vue'
import { toApiError, type ApiError } from '@/api'
import { api } from '@/api'
import BranchPill from '@/components/BranchPill.vue'
import Button from '@/components/Button.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import Kbd from '@/components/Kbd.vue'
import { baseName, timeAgo } from '@/lib/persist'
import { pickFolder, useOpenRepo } from '@/shell/useOpenRepo'
import { useOpsStore } from '@/stores/ops'
import { useRecentStore } from '@/stores/recent'
import CloneDialog from './welcome/CloneDialog.vue'

const router = useRouter()
const recent = useRecentStore()
const ops = useOpsStore()
const { openPath, pickAndOpen } = useOpenRepo()

const error = ref<ApiError | null>(null)
const retry = ref<(() => void) | null>(null)
const cloneOpen = ref(false)
const opening = ref(false)
const version = ref('')

const initBusy = computed(() => ops.running.some((o) => o.kind === 'init'))

async function guarded(run: () => Promise<ApiError | null>, again: () => void) {
  error.value = null
  opening.value = true
  const err = await run()
  opening.value = false
  if (err) {
    error.value = err
    retry.value = again
  }
}

const openFolder = () => guarded(pickAndOpen, openFolder)
const openRecent = (p: string) => guarded(() => openPath(p), () => void openRecent(p))

async function newRepo() {
  error.value = null
  let dir: string | null
  try {
    dir = await pickFolder('Choose a folder for the new repository')
  } catch (e) {
    error.value = toApiError(e)
    return
  }
  if (dir) await initIn(dir)
}

async function initIn(dir: string) {
  try {
    await ops.track({ kind: 'init', label: `Create repository ${baseName(dir)}` }, () => api.init(dir), () => dir)
  } catch (e) {
    error.value = toApiError(e)
    retry.value = () => void initIn(dir)
    return
  }
  await guarded(() => openPath(dir), () => void openPath(dir))
}

function onKey(e: KeyboardEvent) {
  if (e.ctrlKey && e.key.toLowerCase() === 'o') {
    e.preventDefault()
    void openFolder()
  }
}
onMounted(async () => {
  window.addEventListener('keydown', onKey)
  try {
    const info = await api.engineInfo()
    version.value = `mediagit ${info.version} · engine contract v${info.contract}`
  } catch {
    // The startup gate already reports engine problems.
  }
})
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="welcome">
    <header>
      <Wordmark :height="32" />
      <h1>Welcome</h1>
      <p class="sub">Open or create a repository</p>
    </header>

    <div class="actions">
      <Button variant="primary" :loading="opening" @click="openFolder"><FolderOpen :size="18" aria-hidden="true" />Open folder…</Button>
      <Button @click="cloneOpen = true"><GitFork :size="18" aria-hidden="true" />Clone…</Button>
      <Button :loading="initBusy" @click="newRepo"><FolderPlus :size="18" aria-hidden="true" />New repository (init)</Button>
      <Button variant="ghost" @click="router.push('/server')"><Server :size="18" aria-hidden="true" />Host a server…</Button>
    </div>

    <ErrorState v-if="error" class="fail" :code="error.code" :message="error.message" @retry="retry?.()" />

    <section class="recent" aria-labelledby="recent-h">
      <h2 id="recent-h">Recent repositories</h2>
      <ul v-if="recent.list.length" data-testid="recent-list">
        <li v-for="r in recent.list" :key="r.path">
          <button type="button" class="open" @click="openRecent(r.path)">
            <span class="who">
              <strong>{{ r.name }}</strong>
              <span class="path mono">{{ r.path }}</span>
            </span>
            <BranchPill v-if="r.branch" :name="r.branch" />
            <span class="ago">{{ timeAgo(r.openedAt) }}</span>
          </button>
          <IconButton :label="`Remove ${r.name} from recent`" size="sm" @click="recent.remove(r.path)"><X :size="16" /></IconButton>
        </li>
      </ul>
      <p v-else class="none">No repositories yet. Open a folder, clone one, or create a new one.</p>
    </section>

    <p class="foot mono">{{ version }}<template v-if="version"> · </template><Kbd>Ctrl</Kbd>+<Kbd>O</Kbd> to open</p>

    <CloneDialog v-model:open="cloneOpen" />
  </div>
</template>

<style scoped>
header {
  display: grid;
  justify-items: start;
  gap: var(--space-3);
}
.welcome {
  display: grid;
  align-content: start;
  gap: var(--space-5);
  width: min(760px, 100%);
  margin: 0 auto;
  padding: var(--space-8) var(--space-6);
  overflow-y: auto;
}
h1 {
  margin: 0;
  font-size: var(--fs-2xl);
}
.sub {
  margin: 0;
  color: var(--text-muted);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}
.fail {
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
h2 {
  margin: 0;
  padding: var(--space-3) var(--space-4);
  border-bottom: 1px solid var(--border);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.recent {
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
ul {
  margin: 0;
  padding: 0;
  list-style: none;
}
li {
  display: flex;
  align-items: center;
  padding-right: var(--space-2);
}
li + li {
  border-top: 1px solid var(--border);
}
.open {
  display: flex;
  flex: 1;
  align-items: center;
  gap: var(--space-4);
  min-width: 0;
  padding: var(--space-3) var(--space-4);
  border: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.open:hover {
  background: var(--surface-2);
}
.who {
  display: grid;
  flex: 1;
  min-width: 0;
}
.path {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ago {
  min-width: 64px;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  text-align: right;
}
.none {
  margin: 0;
  padding: var(--space-6) var(--space-4);
  color: var(--text-muted);
}
.foot {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
