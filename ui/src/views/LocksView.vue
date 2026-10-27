<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Lock, LockOpen, RefreshCw } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import FileIcon from '@/components/FileIcon.vue'
import IconButton from '@/components/IconButton.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TextField from '@/components/TextField.vue'
import { timeAgo } from '@/lib/persist'
import { filterLocks, useLocksStore, type LockFilter } from '@/stores/locks'
import type { LockReport } from '@/api'

const locks = useLocksStore()
onMounted(() => void locks.refresh())

const FILTERS = [
  { value: 'all', label: 'All' },
  { value: 'mine', label: 'Mine' },
  { value: 'others', label: 'Others' },
]
const filter = ref<LockFilter>('all')
const shown = computed(() => filterLocks(locks.locks, filter.value, locks.me))
const firstLoad = computed(() => !locks.loaded && !locks.error)
const isMine = (l: LockReport) => locks.me !== null && l.owner === locks.me

// Lock a file -----------------------------------------------------------
const lockOpen = ref(false)
const lockPath = ref('')
const lockError = computed(() => {
  const p = lockPath.value.trim()
  if (!p) return undefined
  if (p.startsWith('/') || p.startsWith('-') || p.split('/').includes('..')) return 'Use a path inside the project, like art/hero.psd'
  return locks.byPath.has(p) ? 'This file is already locked' : undefined
})
function openLock() {
  locks.actionError = null
  lockOpen.value = true
}
async function lockFile() {
  const p = lockPath.value.trim().replace(/\\/g, '/')
  if (!p || lockError.value || locks.busy) return
  if (await locks.lock(p)) {
    lockOpen.value = false
    lockPath.value = ''
  }
}

// Force unlock ----------------------------------------------------------
// `forcing` outlives the dialog: the confirm click closes it before our handler runs.
const forcing = ref<LockReport | null>(null)
const forceOpen = ref(false)
function askForce(l: LockReport) {
  forcing.value = l
  forceOpen.value = true
}
</script>

<template>
  <section class="locks" :aria-busy="!!locks.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Locks</h1>
        <p>who is working on which file</p>
      </div>
      <div class="tools">
        <SegmentedControl v-model="filter" label="Whose locks" :options="FILTERS" />
        <IconButton label="Refresh locks" variant="secondary" :disabled="locks.loading" @click="locks.refresh()"><RefreshCw :size="18" /></IconButton>
        <Button variant="primary" :disabled="!!locks.busy" @click="openLock"><Lock :size="16" aria-hidden="true" /> Lock a file</Button>
      </div>
    </header>

    <div v-if="locks.actionError" class="banner" role="alert">
      <code class="mono">{{ locks.actionError.code }}</code>
      <span>{{ locks.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="locks.actionError = null">Dismiss</Button>
    </div>

    <div v-if="firstLoad" class="pad"><SkeletonRows :count="5" /></div>
    <div v-else-if="locks.error && !locks.loaded" class="pad">
      <ErrorState :code="locks.error.code" :message="locks.error.message" @retry="locks.refresh()" />
    </div>
    <div v-else-if="!locks.locks.length" class="pad">
      <EmptyState title="Nothing is locked" description="Lock a file before you edit it so nobody else changes it at the same time.">
        <template #icon><LockOpen :size="24" /></template>
        <Button variant="primary" @click="openLock">Lock a file</Button>
      </EmptyState>
    </div>
    <div v-else-if="!shown.length" class="pad">
      <EmptyState
        :title="filter === 'mine' ? 'You have no locks' : 'Nobody else has locks'"
        :description="filter === 'mine' && locks.me === null ? 'MediaGit learns your lock name the first time you lock a file here.' : 'Choose All to see every lock.'"
      />
    </div>

    <div v-else class="panel">
      <table>
        <caption class="sr-only">File locks</caption>
        <thead>
          <tr>
            <th scope="col">File</th>
            <th scope="col">Locked by</th>
            <th scope="col">Since</th>
            <th scope="col"><span class="sr-only">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="l in shown" :key="l.lock_id" :data-lock="l.path">
            <td class="file"><FileIcon :path="l.path" /><span class="mono" :title="l.path">{{ l.path }}</span></td>
            <td>
              {{ l.owner }}<span v-if="isMine(l)" class="chip">You</span>
            </td>
            <td class="since">{{ timeAgo(l.created_at * 1000) }}</td>
            <td class="act">
              <Button v-if="isMine(l)" size="sm" :loading="locks.busy === `unlock:${l.lock_id}`" :disabled="!!locks.busy" @click="locks.unlock(l, false)">Unlock</Button>
              <Button v-else size="sm" variant="danger" :disabled="!!locks.busy" @click="askForce(l)">Force unlock…</Button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <Dialog v-model:open="lockOpen" title="Lock a file" description="Others can still look at the file, but they cannot save changes to it until you unlock it.">
      <form id="lock-file" class="form" @submit.prevent="lockFile">
        <TextField v-model="lockPath" label="File path" placeholder="art/hero.psd" :error="lockError" hint="Relative to the project folder" mono />
        <p v-if="locks.actionError && lockOpen" class="derr" role="alert">{{ locks.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="lockOpen = false">Cancel</Button>
        <Button type="submit" form="lock-file" variant="primary" :disabled="!lockPath.trim() || !!lockError || !!locks.busy" :loading="!!locks.busy">Lock file</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="forceOpen"
      :title="`Force unlock ${forcing?.path ?? ''}?`"
      :description="`${forcing?.owner ?? 'Someone'} locked this file ${forcing ? timeAgo(forcing.created_at * 1000) : ''}. If you take the lock away, they may be editing it right now and can lose changes they have not saved, or overwrite yours. This needs permission to manage the project.`"
      confirm-label="Force unlock"
      destructive
      @confirm="forcing && locks.unlock(forcing, true)"
    />
  </section>
</template>

<style scoped>
.locks {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
}
.head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3) var(--space-4);
}
.ttl {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
}
h1 {
  margin: 0;
  font-size: var(--fs-2xl);
}
.ttl p {
  margin: 0;
  color: var(--text-muted);
}
.tools {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.pad {
  overflow-y: auto;
}
.banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
}
.banner span {
  flex: 1;
}
.panel {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
table {
  width: 100%;
  border-collapse: collapse;
}
th {
  position: sticky;
  top: 0;
  padding: var(--space-2) var(--space-5);
  border-bottom: 1px solid var(--border);
  background: var(--surface-2);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-align: left;
  text-transform: uppercase;
}
td {
  height: 56px;
  padding: var(--space-2) var(--space-5);
  border-bottom: 1px solid var(--border);
}
tr:last-child td {
  border-bottom: 0;
}
.file {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}
.file span.mono {
  overflow-wrap: anywhere;
}
.since {
  color: var(--text-muted);
  white-space: nowrap;
}
.act {
  text-align: right;
  white-space: nowrap;
}
.chip {
  margin-left: var(--space-2);
  padding: 2px var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--accent-soft);
  color: var(--text);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.form {
  display: grid;
  gap: var(--space-3);
}
.derr {
  margin: 0;
  color: var(--danger);
  font-size: var(--fs-sm);
}
</style>
