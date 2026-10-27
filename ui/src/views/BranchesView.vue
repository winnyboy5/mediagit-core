<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ArrowDown, ArrowUp, GitBranch, Pencil, Plus, RefreshCw, Trash2 } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TextField from '@/components/TextField.vue'
import { shortOid } from '@/lib/history'
import { refNameError } from '@/lib/inspect'
import { primaryBar } from '@/stores/ops'
import { useBranchesStore } from '@/stores/branches'

const branches = useBranchesStore()
onMounted(() => void branches.refresh())

const firstLoad = computed(() => !branches.loaded && !branches.error)

// The switch in flight: files written so far, from the CLI's own count.
const switchBar = computed(() => {
  const op = branches.switching
  return op ? primaryBar(op) : undefined
})

// Create ----------------------------------------------------------------
const createOpen = ref(false)
const newName = ref('')
const taken = computed(() => branches.list.some((b) => b.name === newName.value))
const newError = computed(() => (newName.value ? (refNameError(newName.value) ?? (taken.value ? 'A branch with this name exists' : undefined)) : undefined))
const canCreate = computed(() => !!newName.value && !newError.value && !branches.busy)

function openCreate() {
  branches.actionError = null
  createOpen.value = true
}

async function create(thenSwitch: boolean) {
  if (!canCreate.value) return
  const name = newName.value
  if (!(await branches.create(name))) return
  createOpen.value = false
  newName.value = ''
  if (thenSwitch) await branches.switchTo(name)
}

// Rename ----------------------------------------------------------------
const renaming = ref<string | null>(null)
const renameTo = ref('')
const renameOpen = computed({ get: () => renaming.value !== null, set: (o) => !o && (renaming.value = null) })
const renameError = computed(() => {
  if (!renameTo.value) return undefined
  if (renameTo.value === renaming.value) return 'Choose a different name'
  return refNameError(renameTo.value) ?? (branches.list.some((b) => b.name === renameTo.value) ? 'A branch with this name exists' : undefined)
})
function startRename(name: string) {
  branches.actionError = null
  renameTo.value = name
  renaming.value = name
}
async function rename() {
  const from = renaming.value
  if (!from || !renameTo.value || renameError.value || branches.busy) return
  if (await branches.rename(from, renameTo.value)) renaming.value = null
}

// Delete ----------------------------------------------------------------
// `deleting` outlives the dialog: the confirm click closes it before our handler runs.
const deleting = ref<string | null>(null)
const deleteOpen = ref(false)
function askDelete(name: string) {
  deleting.value = name
  deleteOpen.value = true
}
</script>

<template>
  <section class="branches" :aria-busy="branches.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Branches</h1>
        <p>separate lines of work in this project</p>
      </div>
      <div class="tools">
        <IconButton label="Refresh branches" variant="secondary" :disabled="branches.loading" @click="branches.refresh()"><RefreshCw :size="18" /></IconButton>
        <Button variant="primary" :disabled="branches.busy" @click="openCreate"><Plus :size="16" aria-hidden="true" /> New branch</Button>
      </div>
    </header>

    <div v-if="branches.actionError" class="banner" role="alert">
      <code class="mono">{{ branches.actionError.code }}</code>
      <span>{{ branches.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="branches.actionError = null">Dismiss</Button>
    </div>

    <div v-if="firstLoad" class="pad"><SkeletonRows :count="5" /></div>
    <div v-else-if="branches.error && !branches.loaded" class="pad">
      <ErrorState :code="branches.error.code" :message="branches.error.message" @retry="branches.refresh()" />
    </div>
    <div v-else-if="!branches.list.length" class="pad">
      <EmptyState title="No branches yet" description="Save your first version and the main branch appears here.">
        <template #icon><GitBranch :size="24" /></template>
      </EmptyState>
    </div>

    <div v-else class="panel">
      <ul class="rows" aria-label="Branches">
        <li v-for="b in branches.list" :key="b.name" class="row" :class="{ cur: b.name === branches.current }" :data-branch="b.name">
          <GitBranch :size="18" class="gi" aria-hidden="true" />
          <div class="nm">
            <strong class="mono">{{ b.name }}</strong>
            <span v-if="b.name === branches.current" class="chip now">Current</span>
            <span v-if="b.protected" class="chip prot">Protected</span>
          </div>
          <span v-if="b.upstream" class="sync mono" :aria-label="`${b.ahead ?? 0} ahead, ${b.behind ?? 0} behind ${b.upstream}`">
            <span class="up"><ArrowUp :size="14" aria-hidden="true" />{{ b.ahead ?? 0 }}</span>
            <span class="up"><ArrowDown :size="14" aria-hidden="true" />{{ b.behind ?? 0 }}</span>
          </span>
          <span v-else class="sync none">Not shared yet</span>
          <span class="oid mono">{{ b.oid ? shortOid(b.oid) : '' }}</span>
          <div class="acts">
            <Button v-if="b.name !== branches.current" size="sm" :disabled="branches.busy" @click="branches.switchTo(b.name)">Switch to</Button>
            <IconButton :label="`Rename ${b.name}`" size="sm" :disabled="branches.busy" @click="startRename(b.name)"><Pencil :size="16" /></IconButton>
            <IconButton
              :label="b.name === branches.current ? `Delete ${b.name} (switch to another branch first)` : b.protected ? `Delete ${b.name} (protected)` : `Delete ${b.name}`"
              size="sm"
              :disabled="branches.busy || b.name === branches.current || b.protected"
              @click="askDelete(b.name)"
            >
              <Trash2 :size="16" />
            </IconButton>
          </div>
        </li>
      </ul>

      <div v-if="branches.switching" class="overlay" role="status" aria-live="polite" data-testid="switch-progress">
        <div class="card">
          <strong>{{ branches.switching.label }}</strong>
          <span class="phase">{{ branches.switching.phase || 'Starting…' }}</span>
          <ProgressBar
            label="Updating files"
            :value="switchBar && switchBar.len != null ? switchBar.pos : null"
            :max="switchBar?.len ?? 100"
          />
          <span v-if="switchBar" class="mono count">
            {{ switchBar.pos }}<template v-if="switchBar.len != null"> / {{ switchBar.len }}</template> files
          </span>
        </div>
      </div>
    </div>

    <Dialog v-model:open="createOpen" title="New branch" description="A branch starts from the version you are on now.">
      <form id="create-branch" class="form" @submit.prevent="create(true)">
        <TextField v-model="newName" label="Branch name" placeholder="grade-pass" :error="newError" hint="Letters, numbers, dashes and slashes" mono />
        <p v-if="branches.actionError && createOpen" class="derr" role="alert">{{ branches.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="createOpen = false">Cancel</Button>
        <Button :disabled="!canCreate" :loading="branches.busy" @click="create(false)">Create</Button>
        <Button type="submit" form="create-branch" variant="primary" :disabled="!canCreate" :loading="branches.busy">Create and switch</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="renameOpen" :title="`Rename ${renaming ?? ''}`" description="Anyone who already uses this branch will need to switch to the new name.">
      <form id="rename-branch" class="form" @submit.prevent="rename">
        <TextField v-model="renameTo" label="New name" :error="renameError" mono />
        <p v-if="branches.actionError && renameOpen" class="derr" role="alert">{{ branches.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="renameOpen = false">Cancel</Button>
        <Button type="submit" form="rename-branch" variant="primary" :disabled="!renameTo || !!renameError || branches.busy" :loading="branches.busy">Rename</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="deleteOpen"
      :title="`Delete ${deleting ?? ''}?`"
      description="The branch name is removed. Versions that only this branch reaches stay in the project until it is cleaned up, but you will not find them in History."
      confirm-label="Delete branch"
      destructive
      @confirm="deleting && branches.remove(deleting)"
    />
  </section>
</template>

<style scoped>
.branches {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
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
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.rows {
  margin: 0;
  padding: 0;
  list-style: none;
}
.row {
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr) 112px 64px 176px;
  align-items: center;
  gap: var(--space-4);
  min-height: 56px;
  padding: var(--space-2) var(--space-5);
  border-bottom: 1px solid var(--border);
}
.row:last-child {
  border-bottom: 0;
}
.row.cur {
  background: var(--accent-soft);
}
.gi {
  flex: none;
  color: var(--text-muted);
}
.nm {
  display: flex;
  flex: 1;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
.nm strong {
  overflow-wrap: anywhere;
  font-weight: 600;
}
.chip {
  padding: 2px var(--space-2);
  border-radius: var(--radius-pill);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.now {
  background: var(--accent);
  color: var(--on-accent);
}
.prot {
  background: var(--locked-soft);
  color: var(--locked);
}
.sync {
  display: flex;
  gap: var(--space-3);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.sync.none {
  font-family: inherit;
}
.up {
  display: inline-flex;
  align-items: center;
  gap: 2px;
}
.oid {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.acts {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  justify-content: flex-end;
}
.overlay {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  background: var(--scrim);
}
.card {
  display: grid;
  gap: var(--space-3);
  width: min(360px, 90%);
  padding: var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  box-shadow: var(--shadow-3);
}
.phase,
.count {
  color: var(--text-muted);
  font-size: var(--fs-sm);
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
