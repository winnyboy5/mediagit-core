<script setup lang="ts">
// A folder tree (or flat list) of files in one version; picking a file shows it in the inspector.
import { computed, onBeforeUnmount, ref } from 'vue'
import type { FileStatus } from '@/components/StatusBadge.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import { buildTree, flatRows, flattenTree, type FileEntry } from '@/lib/fileTree'
import { load, save } from '@/lib/persist'
import FileList from '../changes/FileList.vue'

const props = defineProps<{
  entries: FileEntry[]
  group: string
  label: string
  /** Tree entries carry no change status, so no badges. */
  plain?: boolean
}>()
const emit = defineEmits<{ pick: [file: { path: string; size: number | null; status: FileStatus }] }>()

const MODES = [
  { value: 'tree', label: 'Tree' },
  { value: 'flat', label: 'Flat' },
]
const mode = ref<string>(load('mg.history.mode', 'tree'))
const collapsed = ref<ReadonlySet<string>>(new Set())
const selected = ref<ReadonlySet<string>>(new Set())

const tree = computed(() => buildTree(props.entries))
const rows = computed(() => (mode.value === 'tree' ? flattenTree(tree.value, collapsed.value) : flatRows(props.entries)))

function setMode(m: string) {
  mode.value = m
  save('mg.history.mode', m)
}

function toggle(path: string) {
  const next = new Set(collapsed.value)
  if (!next.delete(path)) next.add(path)
  collapsed.value = next
}

// Arrowing through a long list must not start an extraction per row.
let timer: ReturnType<typeof setTimeout> | undefined
function select(path: string) {
  selected.value = new Set([`${props.group}:${path}`])
  const node = rows.value.find((r) => r.node.path === path)?.node
  if (!node || node.isDir) return
  clearTimeout(timer)
  timer = setTimeout(() => emit('pick', { path, size: node.size, status: node.status ?? 'modified' }), 200)
}
onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
  <div class="vf">
    <div class="bar">
      <span class="count">{{ entries.length }} {{ entries.length === 1 ? 'file' : 'files' }}</span>
      <SegmentedControl :model-value="mode" label="Folder view" :options="MODES" @update:model-value="setMode" />
    </div>
    <FileList
      :rows="rows"
      :selected="selected"
      :group="group"
      :label="label"
      :plain="plain"
      @select="(p) => select(p)"
      @toggle="toggle"
      @select-all="() => {}"
    />
  </div>
</template>

<style scoped>
.vf {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
.bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-2) var(--space-4);
}
.count {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
