<script setup lang="ts">
// The selected version: message, who and when, the branches/tags on it, what it
// changed, and every file as it was at that point.
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { GitCompare, GitMerge, Tag } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import BranchPill from '@/components/BranchPill.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TagPill from '@/components/TagPill.vue'
import TextField from '@/components/TextField.vue'
import { refNameError } from '@/lib/inspect'
import { useCompareStore } from '@/stores/compare'
import { useTagsStashStore } from '@/stores/tagsStash'
import ChangeTiles from './ChangeTiles.vue'
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

// "Files at this version" is a disclosure: its tree loads when opened, and again for each version picked while open.
const treeOpen = ref(false)
function onToggle(e: Event) {
  treeOpen.value = (e.target as HTMLDetailsElement).open
  if (treeOpen.value) void history.loadTree()
}
watch(
  () => history.selectedOid,
  () => treeOpen.value && void history.loadTree(),
)

const router = useRouter()
const compare = useCompareStore()
function compareWithPrevious() {
  const c = commit.value
  if (!c?.parents.length) return
  compare.from = c.parents[0]!
  compare.to = c.oid
  void router.push('/history/compare')
}

// Tag this version ------------------------------------------------------------
const tags = useTagsStashStore()
const tagOpen = ref(false)
const tagName = ref('')
const tagError = computed(() =>
  tagName.value ? (refNameError(tagName.value) ?? (tags.tags.some((t) => t.name === tagName.value) ? 'A tag with this name exists' : undefined)) : undefined,
)
function openTag() {
  tags.actionError = null
  tagName.value = ''
  tagOpen.value = true
  if (!tags.tagsLoaded) void tags.refresh()
}
async function doTag() {
  const c = commit.value
  if (!c || !tagName.value || tagError.value) return
  if (await tags.createTag(tagName.value, { commit: c.oid, message: '', tagger: null, email: null, force: false, confirmed: false })) tagOpen.value = false
}

const changed = computed<FileEntry[]>(() =>
  (history.detail?.files ?? []).map((f) => ({ path: f.path, status: f.status as FileStatus, size: f.new_size ?? f.old_size })),
)
// A tree entry has no change status; 'added' is a placeholder the plain list never shows.
const atVersion = computed<FileEntry[]>(() => (history.tree ?? []).map((f) => ({ path: f.path, status: 'added', size: f.size })))

function pick(f: { path: string; size: number | null; status: FileStatus }) {
  history.selectFile({ path: f.path, size: f.size, deleted: f.status === 'deleted' })
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
    <EmptyState v-if="!commit" title="Pick a version" description="Select a version in the list to see what it changed." />

    <template v-else>
      <header>
        <h2 id="ver-title">{{ commit.summary || '(no message)' }}</h2>
        <p class="meta">{{ commit.author.name }} · {{ formatWhen(commit.time) }}</p>
        <p class="meta">
          <span class="mono">{{ shortOid(commit.oid) }}</span>
          <span v-if="commit.parents.length > 1" class="merge"><GitMerge :size="14" aria-hidden="true" /> Merge of {{ commit.parents.length }} versions</span>
        </p>
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
        <div class="acts" role="group" aria-label="This version">
          <Button size="sm" :disabled="!commit.parents.length" @click="compareWithPrevious"><GitCompare :size="14" aria-hidden="true" /> Compare with previous</Button>
          <Button size="sm" @click="openTag"><Tag :size="14" aria-hidden="true" /> Tag this version</Button>
          <UndoMenu :rev="commit.oid" :summary="commit.summary" />
        </div>
        <UndoDialogs />
      </header>

      <section class="sect" aria-labelledby="ver-changes">
        <h3 id="ver-changes">Changes in this version <span v-if="history.detail" class="n">{{ history.detail.files.length }}</span></h3>
        <div v-if="history.detailLoading" class="pad"><SkeletonRows :count="3" /></div>
        <div v-else-if="history.detailError" class="pad">
          <ErrorState :code="history.detailError.code" :message="history.detailError.message" @retry="history.select(history.selectedOid)" />
        </div>
        <p v-else-if="!changed.length" class="none">This version changed no files.</p>
        <ChangeTiles v-else :entries="changed" :rev="commit.oid" label="Files this version changed" @pick="pick" />
      </section>

      <div v-if="history.file" class="picked" data-testid="picked-file">
        <span class="mono pf">{{ history.file.path }}</span>
        <Button size="sm" :disabled="history.file.deleted || undo.busy" @click="undo.ask({ kind: 'restore', rev: commit.oid, path: history.file.path })">
          Restore this file…
        </Button>
      </div>

      <MoreOptions label="Files at this version" class="all" @toggle="onToggle">
        <div v-if="history.treeLoading || (!history.tree && !history.treeError)" class="pad"><SkeletonRows :count="4" /></div>
        <div v-else-if="history.treeError" class="pad">
          <ErrorState :code="history.treeError.code" :message="history.treeError.message" @retry="history.loadTree()" />
        </div>
        <p v-else-if="!atVersion.length" class="none">This version has no files.</p>
        <VersionFiles v-else :entries="atVersion" group="tree" label="Files at this version" plain @pick="pick" />
      </MoreOptions>
    </template>

    <Dialog v-model:open="tagOpen" title="Tag this version" description="A tag gives this version a name you can find later, like v1.0 delivery.">
      <form id="tag-version" class="form" @submit.prevent="doTag">
        <TextField v-model="tagName" label="Tag name" placeholder="e.g. v1.0" :error="tagError" />
        <p v-if="tags.actionError && tagOpen" class="err" role="alert">{{ tags.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="tagOpen = false">Cancel</Button>
        <Button type="submit" form="tag-version" variant="primary" :disabled="!tagName || !!tagError || tags.busy" :loading="tags.busy">Tag this version</Button>
      </template>
    </Dialog>

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
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-top: var(--space-2);
}
.sect {
  display: grid;
  gap: var(--space-3);
  padding: 0 var(--space-5);
}
h3 {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
}
.n {
  color: var(--text-muted);
  font-weight: 400;
}
.all {
  margin: 0 var(--space-5);
}
.err {
  margin: 0;
  color: var(--danger);
  font-size: var(--fs-sm);
}
.form {
  display: grid;
  gap: var(--space-3);
}
.picked {
  margin: 0 var(--space-5);
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
  /* the right-hand column of the Versions page */
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  overflow-y: auto;
  padding-bottom: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
header {
  display: grid;
  align-content: start;
  gap: var(--space-2);
  padding: var(--space-4) var(--space-5) 0;
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
