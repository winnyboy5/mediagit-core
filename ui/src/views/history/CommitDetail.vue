<script setup lang="ts">
// The selected version: message, who and when, the branches/tags on it, what it
// changed, and every file as it was at that point.
import { computed, ref, watch } from 'vue'
import { GitMerge } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import BranchPill from '@/components/BranchPill.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import Tabs from '@/components/Tabs.vue'
import TagPill from '@/components/TagPill.vue'
import type { FileStatus } from '@/components/StatusBadge.vue'
import type { FileEntry } from '@/lib/fileTree'
import { formatWhen, shortOid } from '@/lib/history'
import { useBranchesStore } from '@/stores/branches'
import { useHistoryStore } from '@/stores/history'
import { useRepoStore } from '@/stores/repo'
import Button from '@/components/Button.vue'
import { useUndoStore } from '@/stores/undo'
import UndoDialogs from './UndoDialogs.vue'
import UndoMenu from './UndoMenu.vue'
import VersionFiles from './VersionFiles.vue'

const history = useHistoryStore()
const branches = useBranchesStore()
const repo = useRepoStore()
const undo = useUndoStore()

const commit = computed(() => history.selected)
const rest = computed(() => {
  const m = commit.value?.message ?? ''
  const i = m.indexOf('\n')
  return i < 0 ? '' : m.slice(i + 1).trim()
})
const pills = computed(() => (commit.value ? (history.decorations.get(commit.value.oid) ?? []) : []))
const current = computed(() => repo.status?.branch.name ?? null)

const tab = ref('changes')
const TABS = computed(() => [
  { value: 'changes', label: `Changes in this version${history.detail ? ` · ${history.detail.files.length}` : ''}` },
  { value: 'tree', label: 'Files at this version' },
])
watch([tab, () => history.selectedOid], () => {
  if (tab.value === 'tree') void history.loadTree()
})

const changed = computed<FileEntry[]>(() =>
  (history.detail?.files ?? []).map((f) => ({ path: f.path, status: f.status as FileStatus, size: f.new_size ?? f.old_size })),
)
// A tree entry has no change status; 'added' is a placeholder the plain list never shows.
const atVersion = computed<FileEntry[]>(() => (history.tree ?? []).map((f) => ({ path: f.path, status: 'added', size: f.size })))

function pick(f: { path: string; size: number | null; status: FileStatus }) {
  history.selectFile({ path: f.path, size: f.size, deleted: tab.value === 'changes' && f.status === 'deleted' })
}

// `switching` outlives the dialog: the confirm click closes it before our handler runs.
const switching = ref<string | null>(null)
const switchOpen = ref(false)
function askSwitch(name: string) {
  switching.value = name
  switchOpen.value = true
}
</script>

<template>
  <section class="detail" aria-labelledby="ver-title" data-testid="commit-detail">
    <EmptyState v-if="!commit" style="grid-column: 1 / -1" title="Pick a version" description="Select a version in the list to see what it changed." />

    <template v-else>
      <header>
        <h2 id="ver-title">{{ commit.summary || '(no message)' }}</h2>
        <p class="meta">{{ commit.author.name }} · {{ formatWhen(commit.time) }}</p>
        <p class="meta">
          <span class="mono">{{ shortOid(commit.oid) }}</span>
          <span v-if="commit.parents.length > 1" class="merge"><GitMerge :size="14" aria-hidden="true" /> Merge of {{ commit.parents.length }} versions</span>
        </p>
        <div class="acts">
          <UndoMenu :rev="commit.oid" :summary="commit.summary" />
        </div>
        <div v-if="history.file" class="picked" data-testid="picked-file">
          <span class="mono pf">{{ history.file.path }}</span>
          <Button size="sm" :disabled="history.file.deleted || undo.busy" @click="undo.ask({ kind: 'restore', rev: commit.oid, path: history.file.path })">
            Restore this file…
          </Button>
        </div>
        <ul v-if="pills.length" class="pills" aria-label="Names pointing at this version">
          <li v-for="p in pills" :key="p.kind + p.name">
            <TagPill v-if="p.kind === 'tag'" :name="p.name" />
            <BranchPill v-else-if="p.kind === 'remote' || p.name === current" :name="p.name" :current="p.name === current" />
            <button v-else type="button" class="pill-btn" :aria-label="`Switch to ${p.name}`" @click="askSwitch(p.name)">
              <BranchPill :name="p.name" />
            </button>
          </li>
        </ul>
        <pre v-if="rest" class="body">{{ rest }}</pre>
        <UndoDialogs />
      </header>

      <Tabs v-model="tab" class="tabs" label="Version contents" :items="TABS">
        <template #default="{ value }">
          <template v-if="value === 'changes'">
            <div v-if="history.detailLoading" class="pad"><SkeletonRows :count="4" /></div>
            <div v-else-if="history.detailError" class="pad">
              <ErrorState :code="history.detailError.code" :message="history.detailError.message" @retry="history.select(history.selectedOid)" />
            </div>
            <p v-else-if="!changed.length" class="none">This version changed no files.</p>
            <VersionFiles v-else :entries="changed" group="changed" label="Files this version changed" @pick="pick" />
          </template>
          <template v-else>
            <div v-if="history.treeLoading || (!history.tree && !history.treeError)" class="pad"><SkeletonRows :count="4" /></div>
            <div v-else-if="history.treeError" class="pad">
              <ErrorState :code="history.treeError.code" :message="history.treeError.message" @retry="history.loadTree()" />
            </div>
            <p v-else-if="!atVersion.length" class="none">This version has no files.</p>
            <VersionFiles v-else :entries="atVersion" group="tree" label="Files at this version" plain @pick="pick" />
          </template>
        </template>
      </Tabs>
    </template>

    <AlertDialog
      v-model:open="switchOpen"
      :title="`Switch to ${switching ?? ''}?`"
      description="Your files change to match that branch. If you have unsaved changes the switch is refused and nothing is lost."
      confirm-label="Switch branch"
      @confirm="switching && branches.switchTo(switching)"
    />
  </section>
</template>

<style scoped>
.acts {
  display: flex;
  gap: var(--space-2);
}
.picked {
  display: grid;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--surface-2);
}
.pf {
  overflow-wrap: anywhere;
  font-size: var(--fs-sm);
}
.detail {
  display: grid;
  grid-template-columns: minmax(220px, 34%) minmax(0, 1fr);
  min-height: 0;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
header {
  display: grid;
  align-content: start;
  gap: var(--space-2);
  min-height: 0;
  padding: var(--space-4) var(--space-5);
  overflow-y: auto;
  border-right: 1px solid var(--border);
}
h2 {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
}
.meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.merge {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
}
.pills {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
}
.pill-btn {
  padding: 0;
  border: 0;
  border-radius: var(--radius-pill);
  background: none;
  cursor: pointer;
}
.pill-btn:focus-visible {
  outline: var(--focus-ring);
  outline-offset: var(--focus-offset);
}
.body {
  margin: 0;
  font: inherit;
  font-size: var(--fs-sm);
  white-space: pre-wrap;
}
.tabs {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
.tabs :deep(.panel[data-state='active']) {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
.pad {
  padding: var(--space-3) var(--space-4);
  overflow-y: auto;
}
.none {
  margin: 0;
  padding: var(--space-4) var(--space-5);
  color: var(--text-muted);
}
</style>
