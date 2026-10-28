<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { GitMerge } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import type { MergeStrategy } from '@/api'
import { conflictReason, missingSide } from '@/lib/merge'
import { useBranchesStore } from '@/stores/branches'
import { useMergeStore } from '@/stores/merge'
import { primaryBar } from '@/stores/ops'

const merge = useMergeStore()
const branches = useBranchesStore()
onMounted(() => void branches.refresh())

const firstLoad = computed(() => !branches.loaded && !branches.error)
const bar = computed(() => (merge.running ? primaryBar(merge.running) : undefined))

// Start ------------------------------------------------------------------
const STRATEGIES = [
  { value: 'recursive', label: 'Ask me about each conflicting file' },
  { value: 'ours', label: 'Keep my version of every conflict' },
  { value: 'theirs', label: 'Keep their version of every conflict' },
]
const startOpen = ref(false)
const pick = ref('')
const strategy = ref<string>('recursive')
const noFf = ref(false)
const others = computed(() => branches.list.filter((b) => b.name !== branches.current).map((b) => ({ value: b.name, label: b.name })))

function openStart() {
  merge.error = null
  pick.value = others.value[0]?.value ?? ''
  startOpen.value = true
}
async function start() {
  if (!pick.value || merge.busy) return
  // Close first: progress and any failure show on the page, not behind the dialog.
  startOpen.value = false
  await merge.start(pick.value, strategy.value as MergeStrategy, noFf.value === true)
}

// Cancel -----------------------------------------------------------------
const abortOpen = ref(false)
const REASONS: Record<string, string> = { ours: 'You deleted this file, so there is no version of yours to keep.', theirs: 'They deleted this file, so there is no version of theirs to keep.' }
const theirsMissing = computed(() => !merge.theirsOid)
</script>

<template>
  <section class="merge" :aria-busy="merge.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Merge &amp; conflicts</h1>
        <p>combine branches, resolve clashes</p>
      </div>
      <div class="tools">
        <template v-if="merge.active">
          <Button variant="danger" :disabled="merge.busy" @click="abortOpen = true">Cancel merge</Button>
          <Button variant="primary" :disabled="!merge.canContinue || merge.busy" :loading="merge.busy" @click="merge.continue()">Finish merge</Button>
        </template>
        <Button v-else variant="primary" :disabled="merge.busy || firstLoad || !others.length" @click="openStart">
          <GitMerge :size="16" aria-hidden="true" /> Start a merge
        </Button>
      </div>
    </header>

    <div v-if="merge.error" class="banner" role="alert">
      <code class="mono">{{ merge.error.code }}</code>
      <span>{{ merge.error.message }}</span>
      <Button size="sm" variant="ghost" @click="merge.error = null">Dismiss</Button>
    </div>

    <div v-if="firstLoad" class="pad"><SkeletonRows :count="4" /></div>
    <div v-else-if="branches.error && !branches.loaded" class="pad">
      <ErrorState :code="branches.error.code" :message="branches.error.message" @retry="branches.refresh()" />
    </div>

    <div v-else-if="merge.active" class="panel">
      <div class="summary" data-testid="merge-summary">
        <strong>Merging {{ merge.theirsName ?? 'another branch' }} into {{ branches.current ?? 'this branch' }}</strong>
        <span class="muted">{{ merge.conflicts.length - merge.resolved }} of {{ merge.conflicts.length }} files need a decision · {{ merge.resolved }} resolved</span>
        <ProgressBar label="Conflicts resolved" :value="merge.resolved" :max="merge.conflicts.length || 1" />
      </div>
      <p v-if="theirsMissing" class="note">This merge was started elsewhere, so only your version can be chosen here. Cancel the merge and start it again to pick theirs.</p>
      <ul class="rows" aria-label="Conflicting files">
        <li v-for="c in merge.conflicts" :key="c.path" class="row" :data-file="c.path">
          <StatusBadge status="conflict" />
          <div class="nm">
            <span class="mono path">{{ c.path }}</span>
            <span class="muted">{{ conflictReason(c.kind) }}</span>
            <span v-if="missingSide(c.kind)" class="muted">{{ REASONS[missingSide(c.kind)!] }}</span>
          </div>
          <span class="kept">
            <span v-if="c.resolved" class="chip done">
              {{ c.resolved === 'ours' ? 'Kept mine' : c.resolved === 'theirs' ? 'Kept theirs' : 'Resolved' }}
            </span>
          </span>
          <div class="acts" role="group" :aria-label="`Choose a version of ${c.path}`">
            <Button
              size="sm"
              :variant="c.resolved === 'ours' ? 'primary' : 'secondary'"
              :aria-pressed="c.resolved === 'ours'"
              :disabled="merge.busy || missingSide(c.kind) === 'ours'"
              @click="merge.resolve(c.path, 'ours')"
            >
              Keep mine
            </Button>
            <Button
              size="sm"
              :variant="c.resolved === 'theirs' ? 'primary' : 'secondary'"
              :aria-pressed="c.resolved === 'theirs'"
              :disabled="merge.busy || theirsMissing || missingSide(c.kind) === 'theirs'"
              @click="merge.resolve(c.path, 'theirs')"
            >
              Keep theirs
            </Button>
          </div>
        </li>
      </ul>

      <div v-if="merge.running" class="overlay" role="status" aria-live="polite" data-testid="merge-progress">
        <div class="card">
          <strong>{{ merge.running.label }}</strong>
          <span class="phase">{{ merge.running.phase || 'Starting…' }}</span>
          <ProgressBar label="Updating files" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
          <span v-if="bar" class="mono count">{{ bar.pos }}<template v-if="bar.len != null"> / {{ bar.len }}</template> files</span>
        </div>
      </div>
    </div>

    <div v-else class="pad">
      <p v-if="merge.last" class="done-note" role="status" data-testid="merge-result">
        <GitMerge :size="16" aria-hidden="true" /> {{ merge.last.status === 'aborted' ? 'Merge cancelled. Nothing was changed.' : `Done: ${merge.last.status.replace(/_/g, ' ')}.` }}
      </p>
      <EmptyState title="No merge in progress" description="Pick a branch to merge into the one you are on.">
        <template #icon><GitMerge :size="24" /></template>
        <Button variant="primary" :disabled="merge.busy || !others.length" @click="openStart">Start a merge</Button>
        <p v-if="!others.length" class="muted">There is no other branch to merge yet.</p>
      </EmptyState>
      <!-- The status report does not say whether a merge is under way, so these stay behind a disclosure. -->
      <details class="resume">
        <summary class="muted">Merge started in another tool?</summary>
        <div class="resume-acts">
          <Button size="sm" :disabled="merge.busy" @click="merge.continue()">Finish merge</Button>
          <Button size="sm" variant="ghost" :disabled="merge.busy" @click="abortOpen = true">Cancel merge</Button>
        </div>
      </details>
      <div v-if="merge.running" class="busy" role="status" data-testid="merge-progress">
        <strong>{{ merge.running.label }}</strong>
        <ProgressBar label="Updating files" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
      </div>
    </div>

    <Dialog v-model:open="startOpen" title="Start a merge" description="Bring another branch's work into the branch you are on.">
      <form id="start-merge" class="form" @submit.prevent="start">
        <Select v-model="pick" label="Merge this branch" :options="others" />
        <Select v-model="strategy" label="When files clash" :options="STRATEGIES" />
        <Checkbox v-model="noFf" label="Always create a merge version" />
      </form>
      <template #footer>
        <Button @click="startOpen = false">Cancel</Button>
        <Button type="submit" form="start-merge" variant="primary" :disabled="!pick || merge.busy" :loading="merge.busy">Start merge</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="abortOpen"
      title="Cancel this merge?"
      description="Your choices are discarded and your files go back to how they were before the merge started."
      confirm-label="Cancel merge"
      destructive
      @confirm="merge.abort()"
    />
  </section>
</template>

<style scoped>
.merge {
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
.ttl p,
.muted {
  margin: 0;
  color: var(--text-muted);
}
.tools {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.pad {
  display: grid;
  gap: var(--space-4);
  align-content: start;
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
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.summary {
  display: grid;
  gap: var(--space-2);
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--border);
}
.note {
  margin: 0;
  padding: var(--space-3) var(--space-5);
  border-bottom: 1px solid var(--border);
  background: var(--surface-2);
  color: var(--text-muted);
}
.rows {
  margin: 0;
  padding: 0;
  list-style: none;
}
.row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) 96px auto;
  align-items: center;
  gap: var(--space-4);
  min-height: 56px;
  padding: var(--space-2) var(--space-5);
  border-bottom: 1px solid var(--border);
}
.row:last-child {
  border-bottom: 0;
}
.nm {
  display: grid;
  gap: 2px;
  min-width: 0;
  font-size: var(--fs-sm);
}
.path {
  overflow-wrap: anywhere;
  color: var(--text);
  font-size: var(--fs-md);
  font-weight: 600;
}
.chip {
  padding: 2px var(--space-2);
  border-radius: var(--radius-pill);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.kept {
  display: flex;
  justify-content: flex-start;
}
.chip.done {
  background: var(--success-soft);
  color: var(--success);
}
.acts {
  display: flex;
  gap: var(--space-2);
}
.done-note {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--success-soft);
}
.resume {
  justify-self: center;
  text-align: center;
}
.resume summary {
  cursor: pointer;
  font-size: var(--fs-sm);
}
.resume-acts {
  display: flex;
  justify-content: center;
  gap: var(--space-3);
  margin-top: var(--space-2);
}
.busy {
  display: grid;
  gap: var(--space-2);
  width: min(360px, 100%);
  margin: 0 auto;
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
  gap: var(--space-4);
}
</style>
