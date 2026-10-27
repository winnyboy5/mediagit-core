<script setup lang="ts">
import { computed } from 'vue'
import FileIcon from './FileIcon.vue'
import ProgressBar from './ProgressBar.vue'
import StatusBadge, { type FileStatus } from './StatusBadge.vue'
import { formatBytes } from './format'

const props = defineProps<{
  path: string
  size?: number
  status?: FileStatus
  selected?: boolean
  /** Present while the file is being processed: number = determinate, null = indeterminate. */
  progress?: number | null
}>()

const slash = computed(() => Math.max(props.path.lastIndexOf('/'), props.path.lastIndexOf('\\')))
const dir = computed(() => props.path.slice(0, slash.value + 1))
const name = computed(() => props.path.slice(slash.value + 1))
</script>

<template>
  <div class="row" :class="{ selected }">
    <FileIcon :path="path" />
    <span class="path" :title="path"><span class="dir">{{ dir }}</span><span class="name">{{ name }}</span></span>
    <ProgressBar v-if="progress !== undefined" class="mini" size="sm" :label="`Processing ${name}`" :value="progress" :max="100" />
    <span v-if="size != null" class="size mono">{{ formatBytes(size) }}</span>
    <StatusBadge v-if="status" :status="status" />
  </div>
</template>

<style scoped>
.row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  height: var(--row-h);
  padding: 0 var(--space-4);
  border-radius: var(--radius-control);
}
.row:hover {
  background: var(--surface-2);
}
.row.selected {
  background: var(--accent-soft);
}
.path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dir {
  color: var(--text-muted);
}
.name {
  font-weight: 500;
}
.mini {
  flex: none;
  width: 88px;
}
.size {
  flex: none;
  min-width: 72px;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  text-align: right;
}
</style>
