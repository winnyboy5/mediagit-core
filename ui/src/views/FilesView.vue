<script setup lang="ts">
// All files: the whole working folder, with a status chip only on the files that changed.
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { FolderOpen, PanelRight, Search } from 'lucide-vue-next'
import { useFocusRefresh } from '@/lib/focusRefresh'
import Checkbox from '@/components/Checkbox.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { api } from '@/api'
import { useChangesStore } from '@/stores/changes'
import { useFilesStore } from '@/stores/files'
import { useRepoStore } from '@/stores/repo'
import { useShellStore } from '@/stores/shell'
import FileList from './changes/FileList.vue'
import SaveDock from './changes/SaveDock.vue'

const files = useFilesStore()
const changes = useChangesStore()
const shell = useShellStore()
const repo = useRepoStore()
// A sparse checkout leaves folders out on purpose; the list must not look like the whole project.
const partial = ref(false)

const find = ref<HTMLInputElement | null>(null)
// Ctrl+F is where people expect the search box.
function onFindKey(e: KeyboardEvent) {
  if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'f') {
    e.preventDefault()
    find.value?.focus()
    find.value?.select()
  }
}
onMounted(() => {
  if (!files.loaded) void files.refresh()
  if (repo.path) void api.sparseList(repo.path).then((r) => (partial.value = !!r?.enabled), () => {})
  window.addEventListener('keydown', onFindKey)
})
onBeforeUnmount(() => window.removeEventListener('keydown', onFindKey))

useFocusRefresh(() => files.refresh())

const MODES = [
  { value: 'tree', label: 'Tree' },
  { value: 'flat', label: 'Flat' },
]
const first = computed(() => !files.loaded && !files.error)
const none = computed(() => files.loaded && !files.entries.length)
const noMatch = computed(() => files.loaded && !!files.entries.length && !files.rows.length)
const summary = computed(() => {
  const n = files.entries.length
  const c = files.changedCount
  if (files.query.trim()) return `${files.matchCount.toLocaleString('en-US')} of ${n.toLocaleString('en-US')} ${n === 1 ? 'file' : 'files'}`
  return `${n.toLocaleString('en-US')} ${n === 1 ? 'file' : 'files'}${c ? `, ${c.toLocaleString('en-US')} changed` : ''}`
})

/** Space or the checkbox on a changed file: include it in the next version, or take it out. */
function check(path: string) {
  const e = files.entries.find((x) => x.path === path)
  if (e?.status) void changes.setIncluded([path], !e.included)
}
</script>

<template>
  <section class="explorer" :aria-busy="files.loading || undefined">
    <h2 class="sr-only">All files</h2>
    <header class="head">
      <label class="find">
        <Search :size="18" aria-hidden="true" />
        <input ref="find" v-model="files.query" type="search" class="q" placeholder="Find a file" aria-label="Find a file" autocomplete="off" spellcheck="false" />
      </label>
      <Checkbox v-model="files.showIgnored" label="Show ignored files" />
      <p class="sum" data-testid="files-summary">{{ summary }}</p>
      <p v-if="partial" class="sum" data-testid="partial-note">Partial copy: only some folders are on this computer. Change this in Settings.</p>
      <span class="grow" />
      <SegmentedControl v-model="changes.mode" label="Folder view" :options="MODES" />
      <IconButton
        v-if="shell.narrow"
        :label="shell.inspectorVisible ? 'Hide details panel' : 'Show details panel'"
        variant="secondary"
        :pressed="shell.inspectorVisible"
        @click="shell.toggleInspector()"
      >
        <PanelRight :size="18" />
      </IconButton>
    </header>

    <div v-if="first" class="pad"><SkeletonRows :count="10" /></div>
    <div v-else-if="files.error && !files.loaded" class="pad">
      <ErrorState :code="files.error.code" :message="files.error.message" :details="files.error.details" @retry="files.refresh()" />
    </div>
    <div v-else-if="none" class="pad">
      <EmptyState title="No files yet" description="Add files to this folder and they show up here.">
        <template #icon><FolderOpen :size="24" /></template>
      </EmptyState>
    </div>
    <div v-else-if="noMatch" class="pad">
      <EmptyState title="No match" :description="`No file or folder path contains “${files.query.trim()}”.`">
        <template #icon><Search :size="24" /></template>
      </EmptyState>
    </div>
    <FileList
      v-else
      class="list"
      :rows="files.rows"
      :selected="files.selected"
      group="files"
      label="All files"
      checks="files"
      explorer
      @select="(p, m) => files.select(p, m)"
      @toggle="(p) => files.toggleFolder(p)"
      @select-all="() => {}"
      @check="check"
    />

    <SaveDock v-if="changes.list.length" />
  </section>
</template>

<style scoped>
.explorer {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
.head {
  display: flex;
  flex: none;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3) var(--space-4);
  padding: var(--space-3) var(--space-6);
}
.find {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex: 1 1 260px;
  max-width: 420px;
  height: var(--control-h);
  padding: 0 var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text-muted);
}
.find:focus-within {
  outline: var(--focus-ring);
  outline-offset: 1px;
}
.q {
  flex: 1;
  min-width: 0;
  border: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  outline: none;
}
.sum {
  margin: 0;
  color: var(--text-muted);
}
.grow {
  flex: 1;
}
.pad {
  flex: 1;
  padding: 0 var(--space-6);
  overflow-y: auto;
}
.list {
  padding: 0 var(--space-4);
}
</style>
