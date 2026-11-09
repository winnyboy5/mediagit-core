<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ArrowDown, ArrowUp, Ellipsis, GitBranch, Plus, Trash2 } from 'lucide-vue-next'
import DropdownMenu, { type MenuItem } from '@/components/DropdownMenu.vue'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import Select from '@/components/Select.vue'
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

function openCreate() {
  branches.actionError = null
  createOpen.value = true
}

// More options for a new branch.
const startPoint = ref('')
const upstream = ref('')
const tracking = ref('auto') // 'auto' is a named choice: the select cannot hold an empty value
const TRACKING = [
  { value: 'auto', label: 'Automatic' },
  { value: 'track', label: 'Follow the branch it starts from (--track)' },
  { value: 'none', label: 'Do not follow anything (--no-track)' },
]
const startError = computed(() => (startPoint.value ? (refNameError(startPoint.value) ? 'Use a branch, tag or version id' : undefined) : undefined))
const upstreamError = computed(() => (upstream.value && !upstream.value.includes('/') ? 'Write it as remote/branch, like origin/main' : upstream.value ? (refNameError(upstream.value) ?? undefined) : undefined))
const canCreate = computed(() => !!newName.value && !newError.value && !startError.value && !upstreamError.value && !branches.busy)

async function create(thenSwitch: boolean) {
  if (!canCreate.value) return
  const name = newName.value
  const opts = {
    startPoint: startPoint.value.trim() || undefined,
    setUpstream: upstream.value.trim() || undefined,
    tracking: tracking.value === 'auto' ? undefined : (tracking.value as 'track' | 'none'),
  }
  if (!(await branches.create(name, opts))) return
  createOpen.value = false
  newName.value = startPoint.value = upstream.value = ''
  tracking.value = 'auto'
  if (thenSwitch) await branches.switchTo(name)
}

// Switch -------------------------------------------------------------------
// A switch that unsaved changes block can be forced; that throws those changes away.
const blocked = ref<string | null>(null)
const forceOpen = ref(false)
async function doSwitch(name: string) {
  blocked.value = (await branches.switchTo(name)) ? null : name
}
const forceSwitch = async () => {
  const name = blocked.value
  if (name && (await branches.switchTo(name, { force: true, confirmed: true }))) blocked.value = null
}

// Remote-tracking branches ---------------------------------------------------
const forgetting = ref<string | null>(null)
const forgetOpen = ref(false)

// Details ---------------------------------------------------------------------
const detailOpen = ref(false)
function openDetails(name: string) {
  detailOpen.value = true
  void branches.show(name)
}

// Protection ------------------------------------------------------------------
const protecting = ref<string | null>(null)
const protectOpen = ref(false)
const requireReviews = ref(false)
const unprotectOpen = ref(false)
function askProtect(name: string, isProtected: boolean) {
  protecting.value = name
  branches.actionError = null
  if (isProtected) unprotectOpen.value = true
  else {
    requireReviews.value = false
    protectOpen.value = true
  }
}
async function protect() {
  if (protecting.value && (await branches.protect(protecting.value, requireReviews.value === true))) protectOpen.value = false
}

// Rename ----------------------------------------------------------------
const renaming = ref<string | null>(null)
const renameTo = ref('')
const renameOpen = computed({ get: () => renaming.value !== null, set: (o) => !o && (renaming.value = null) })
const overwrite = ref(false)
const overwriteOpen = ref(false)
const exists = computed(() => branches.list.some((b) => b.name === renameTo.value))
const renameError = computed(() => {
  if (!renameTo.value) return undefined
  if (renameTo.value === renaming.value) return 'Choose a different name'
  return refNameError(renameTo.value) ?? (exists.value && overwrite.value !== true ? 'A branch with this name exists' : undefined)
})
function startRename(name: string) {
  branches.actionError = null
  renameTo.value = name
  overwrite.value = false
  renaming.value = name
}
async function doRename() {
  const from = renaming.value
  if (!from || !renameTo.value || renameError.value || branches.busy) return
  if (await branches.rename(from, renameTo.value, overwrite.value === true && exists.value)) renaming.value = null
}
// Replacing a branch loses it: ask first, saying which one.
function rename() {
  if (overwrite.value === true && exists.value) overwriteOpen.value = true
  else void doRename()
}

// Row menu: one "…" instead of four icon buttons.
type Row = (typeof branches.list)[number]
const branchMenu = (b: Row): MenuItem[] => [
  { id: 'details', label: 'Details…' },
  { id: 'protect', label: b.protected ? 'Remove protection…' : 'Protect branch…', disabled: branches.busy },
  { id: 'rename', label: 'Rename…', disabled: branches.busy },
  {
    id: 'delete',
    label: b.name === branches.current ? 'Delete (switch to another branch first)' : b.protected ? 'Delete (protected)' : 'Delete…',
    danger: true,
    separated: true,
    disabled: branches.busy || b.name === branches.current || b.protected,
  },
]
function onBranchMenu(id: string, b: Row) {
  if (id === 'details') openDetails(b.name)
  else if (id === 'protect') askProtect(b.name, b.protected)
  else if (id === 'rename') startRename(b.name)
  else askDelete(b.name)
}

// Delete ----------------------------------------------------------------
// `deleting` outlives the dialog: the confirm click closes it before our handler runs.
const deleting = ref<string | null>(null)
const deleteOpen = ref(false)
const deleteAnyway = ref(false)
function askDelete(name: string) {
  deleting.value = name
  deleteAnyway.value = false
  deleteOpen.value = true
}
// Safe by default: the CLI refuses unless the branch's work is merged somewhere else.
function doDelete() {
  const name = deleting.value
  if (!name) return
  deleteOpen.value = false
  void branches.remove(name, deleteAnyway.value === true ? { force: true, confirmed: true } : { mergedOnly: true })
}
</script>

<template>
  <section class="branches" :aria-busy="branches.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">Branches</h2>
      </div>
      <div class="tools">
        <Button variant="secondary" :disabled="branches.busy" @click="openCreate"><Plus :size="16" aria-hidden="true" /> New branch</Button>
      </div>
    </header>

    <div v-if="branches.actionError" class="banner" role="alert">
      <code class="mono">{{ branches.actionError.code }}</code>
      <span>{{ branches.actionError.message }}</span>
      <Button v-if="blocked" size="sm" variant="danger" :disabled="branches.busy" @click="forceOpen = true">Switch anyway…</Button>
      <Button size="sm" variant="ghost" @click="branches.actionError = null; blocked = null">Dismiss</Button>
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
            <strong>{{ b.name }}</strong>
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
            <Button v-if="b.name !== branches.current" size="sm" :disabled="branches.busy" @click="doSwitch(b.name)">Switch to</Button>
            <DropdownMenu :items="branchMenu(b)" @select="(id) => onBranchMenu(id, b)">
              <IconButton :label="`Actions for ${b.name}`" size="sm"><Ellipsis :size="16" /></IconButton>
            </DropdownMenu>
          </div>
        </li>
      </ul>

      <section v-if="branches.remotes.length" class="remotes" aria-labelledby="remote-h">
        <h2 id="remote-h">On the server <span class="muted">as of your last fetch</span></h2>
        <ul class="rows" aria-label="Remote branches">
          <li v-for="r in branches.remotes" :key="r.name" class="row remote" :data-remote="r.name">
            <GitBranch :size="18" class="gi" aria-hidden="true" />
            <strong class="nm">{{ r.name }}</strong>
            <span class="oid mono">{{ r.oid ? shortOid(r.oid) : '' }}</span>
            <div class="acts">
              <Button size="sm" :disabled="branches.busy" @click="branches.checkoutRemote(r.name)">Start working on it</Button>
              <Button size="sm" variant="ghost" :aria-label="`Forget ${r.name} on this computer`" :disabled="branches.busy" @click="((forgetting = r.name), (forgetOpen = true))"><Trash2 :size="16" aria-hidden="true" /> Forget</Button>
            </div>
          </li>
        </ul>
      </section>

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
        <TextField v-model="newName" label="Branch name" placeholder="e.g. grade-pass" :error="newError" hint="Letters, numbers, dashes and slashes" mono />
        <MoreOptions>
          <TextField v-model="startPoint" label="Start from" placeholder="e.g. main, v1.0 or a version id" hint="Leave blank to start from where you are." flag="--start-point" :error="startError" mono />
          <TextField v-model="upstream" label="Follow this server branch" placeholder="e.g. origin/main" hint="Pull and push will use this branch by default." flag="--set-upstream" :error="upstreamError" mono />
          <Select v-model="tracking" label="Following" :options="TRACKING" />
        </MoreOptions>
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
        <MoreOptions>
          <OptionCheck v-model="overwrite" danger label="Replace the branch that already has this name" flag="--force" hint="That branch is removed. Its work stays only if another branch reaches it." />
        </MoreOptions>
        <p v-if="branches.actionError && renameOpen" class="derr" role="alert">{{ branches.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="renameOpen = false">Cancel</Button>
        <Button type="submit" form="rename-branch" variant="primary" :disabled="!renameTo || !!renameError || branches.busy" :loading="branches.busy">Rename</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="deleteOpen" :title="`Delete ${deleting ?? ''}?`" :description="deleteAnyway === true ? 'Work that only this branch has will be lost from History. It can only be recovered from the activity log or by its version id.' : 'The branch name is removed. It is only deleted if its work is already part of another branch.'">
      <MoreOptions>
        <OptionCheck v-model="deleteAnyway" danger label="Delete even if its work is not merged anywhere" flag="-D" hint="Versions that only this branch reaches stay in the project until it is cleaned up, but you will not find them in History." />
      </MoreOptions>
      <template #footer>
        <Button @click="deleteOpen = false">Cancel</Button>
        <Button variant="danger" @click="doDelete">Delete branch</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="overwriteOpen"
      :title="`Replace ${renameTo}?`"
      :description="`A branch called ${renameTo} already exists. It is removed and ${renaming ?? 'this branch'} takes its name.`"
      confirm-label="Replace branch"
      destructive
      @confirm="doRename"
    />

    <AlertDialog
      v-model:open="forceOpen"
      :title="`Switch to ${blocked ?? ''} anyway?`"
      description="Unsaved changes that stand in the way are thrown away and cannot be brought back."
      confirm-label="Discard changes and switch"
      destructive
      @confirm="forceSwitch"
    />

    <AlertDialog
      v-model:open="forgetOpen"
      :title="`Forget ${forgetting ?? ''}?`"
      description="Only the copy on this computer is removed. The branch stays on the server and comes back the next time you fetch."
      confirm-label="Forget branch"
      @confirm="forgetting && branches.forgetRemote(forgetting)"
    />

    <Dialog v-model:open="protectOpen" :title="`Protect ${protecting ?? ''}`" description="A protected branch cannot be deleted or have its history overwritten.">
      <OptionCheck v-model="requireReviews" label="Ask for a review first" flag="--require-reviews" hint="Changes need one approval before they are merged" />
      <p v-if="branches.actionError && protectOpen" class="derr" role="alert">{{ branches.actionError.message }}</p>
      <template #footer>
        <Button @click="protectOpen = false">Cancel</Button>
        <Button variant="primary" :disabled="branches.busy" @click="protect">Protect branch</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="unprotectOpen"
      :title="`Remove protection from ${protecting ?? ''}?`"
      description="Anyone can then delete this branch or overwrite its history."
      confirm-label="Remove protection"
      destructive
      @confirm="protecting && branches.unprotect(protecting)"
    />

    <Dialog v-model:open="detailOpen" :title="branches.detail ? branches.detail.name : 'Branch details'" description="What the project knows about this branch.">
      <dl v-if="branches.detail" class="kv" data-testid="branch-detail">
        <dt>Latest version</dt><dd class="mono">{{ branches.detail.oid ? shortOid(branches.detail.oid) : 'none yet' }}</dd>
        <dt>Follows</dt><dd class="mono">{{ branches.detail.upstream ?? 'nothing' }}</dd>
        <dt>Ahead / behind</dt><dd>{{ branches.detail.upstream ? `${branches.detail.ahead ?? 0} ahead, ${branches.detail.behind ?? 0} behind` : 'not shared yet' }}</dd>
        <dt>Protected</dt><dd>{{ branches.detail.protected ? 'Yes' : 'No' }}</dd>
      </dl>
      <p v-else class="muted" role="status">Loading…</p>
      <template #footer><Button @click="detailOpen = false">Close</Button></template>
    </Dialog>
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
.pt {
  margin: 0;
  font-size: var(--fs-xl);
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
  grid-template-columns: 18px minmax(0, 1fr) 112px 64px 240px;
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
.remotes {
  border-top: 1px solid var(--border);
}
.remotes h2 {
  margin: 0;
  padding: var(--space-3) var(--space-5) 0;
  font-size: var(--fs-md);
}
.muted {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  font-weight: 400;
}
.row.remote {
  grid-template-columns: 18px minmax(0, 1fr) 64px auto;
}
.kv {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-2) var(--space-5);
  margin: 0;
}
.kv dt {
  color: var(--text-muted);
}
.kv dd {
  margin: 0;
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
