<script setup lang="ts">
// The files a version changed as a grid of thumbnails with a status chip each.
// Images are extracted from the version; every other kind shows its type icon.
import { computed, ref, watchEffect } from 'vue'
import { api } from '@/api'
import FileIcon from '@/components/FileIcon.vue'
import StatusBadge, { type FileStatus } from '@/components/StatusBadge.vue'
import Button from '@/components/Button.vue'
import type { FileEntry } from '@/lib/fileTree'
import { previewKind } from '@/lib/inspect'
import { fileKey, useInspectorStore } from '@/stores/inspector'

const SHOWN = 12
const props = defineProps<{ entries: FileEntry[]; rev: string; label: string }>()
const emit = defineEmits<{ pick: [file: { path: string; size: number | null; status: FileStatus }] }>()

const inspector = useInspectorStore()
const all = ref(false)
const shown = computed(() => (all.value ? props.entries : props.entries.slice(0, SHOWN)))
const selected = ref<string | null>(null)

const file = (e: FileEntry) => ({ path: e.path, size: e.size ?? null, rev: props.rev, deleted: e.status === 'deleted' })
const wantsImage = (e: FileEntry) => previewKind(e.path) === 'image' && e.status !== 'deleted'

watchEffect(() => {
  for (const e of shown.value) if (wantsImage(e)) inspector.ensureExtraction(file(e))
})

function src(e: FileEntry) {
  if (!wantsImage(e)) return undefined
  const x = inspector.extractions.get(fileKey(file(e)))
  return x?.status === 'ready' && x.path ? api.fileSrc(x.path) : undefined
}
const name = (p: string) => p.split('/').pop() ?? p

function pick(e: FileEntry) {
  selected.value = e.path
  emit('pick', { path: e.path, size: e.size ?? null, status: (e.status ?? 'modified') as FileStatus })
}
</script>

<template>
  <div class="tiles">
    <ul :aria-label="label">
      <li v-for="e in shown" :key="e.path">
        <button type="button" class="tile" :aria-pressed="selected === e.path" :title="e.path" @click="pick(e)">
          <span class="thumb">
            <img v-if="src(e)" :src="src(e)" alt="" loading="lazy" />
            <FileIcon v-else :path="e.path" />
            <StatusBadge class="chip" :status="(e.status ?? 'modified') as FileStatus" />
          </span>
          <span class="nm">{{ name(e.path) }}</span>
        </button>
      </li>
    </ul>
    <Button v-if="!all && entries.length > SHOWN" size="sm" variant="ghost" @click="all = true">Show all {{ entries.length }} files</Button>
  </div>
</template>

<style scoped>
.tiles {
  display: grid;
  gap: var(--space-3);
}
ul {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(112px, 1fr));
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
}
.tile {
  display: grid;
  gap: var(--space-1);
  justify-items: start;
  width: 100%;
  padding: var(--space-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.tile:hover {
  background: var(--surface-2);
}
.tile[aria-pressed='true'] {
  border-color: var(--border-strong);
  background: var(--accent-soft);
}
.tile:focus-visible {
  outline: var(--focus-ring);
  outline-offset: var(--focus-offset);
}
.thumb {
  position: relative;
  display: grid;
  place-items: center;
  width: 100%;
  aspect-ratio: 4 / 3;
  overflow: hidden;
  border-radius: var(--radius-control);
  background: var(--surface-2);
}
/* The status chip sits on the thumbnail's top-left corner; an opaque ground keeps it legible over images. */
.chip {
  position: absolute;
  top: var(--space-1);
  left: var(--space-1);
  box-shadow: 0 0 0 2px var(--surface);
  background-clip: padding-box;
}
.thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.nm {
  max-width: 100%;
  overflow: hidden;
  font-size: var(--fs-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
