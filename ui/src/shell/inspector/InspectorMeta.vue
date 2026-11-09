<script setup lang="ts">
// Media details (dimensions, duration, codec...) from `media info`; the shape
// differs per file type, so rows are generated from whatever the parser reports.
import { computed, watchEffect } from 'vue'
import { flattenInfo, hasMediaInfo, type InspectFile } from '@/lib/inspect'
import { fileKey, useInspectorStore } from '@/stores/inspector'

const props = defineProps<{ file: InspectFile }>()
const inspector = useInspectorStore()

const supported = computed(() => hasMediaInfo(props.file.path) && !props.file.deleted && props.file.status !== 'deleted')
watchEffect(() => {
  if (supported.value) inspector.ensureInfo(props.file)
})
const entry = computed(() => inspector.infos.get(fileKey(props.file)))
const rows = computed(() => (entry.value?.value ? flattenInfo(entry.value.value) : []))
</script>

<template>
  <section v-if="supported" aria-labelledby="insp-media" data-testid="media-details">
    <h3 id="insp-media">Media details</h3>
    <div v-if="!entry || entry.status === 'loading'" class="sk" role="status" aria-label="Loading media details">
      <span v-for="i in 3" :key="i" class="line" :style="{ width: 40 + i * 15 + '%' }" />
    </div>
    <p v-else-if="entry.status === 'error'" class="note">Details are not available for this file: {{ entry.error?.message }}</p>
    <p v-else-if="!rows.length" class="note">The file has no readable media details.</p>
    <dl v-else>
      <template v-for="r in rows" :key="r.label">
        <dt>{{ r.label }}</dt>
        <dd class="mono">{{ r.value }}</dd>
      </template>
    </dl>
  </section>
</template>

<style scoped>
h3 {
  margin: 0 0 var(--space-2);
  color: var(--text-muted);
  font-size: var(--fs-md);
  font-weight: 600;
}
dl {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--space-2) var(--space-4);
  margin: 0;
}
dt {
  color: var(--text-muted);
}
dd {
  margin: 0;
  overflow-wrap: anywhere;
  text-align: right;
  font-size: var(--fs-sm);
}
.note {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.sk {
  display: grid;
  gap: var(--space-2);
}
.line {
  height: 12px;
  border-radius: var(--radius-control);
  background: var(--surface-3);
}
@media (prefers-reduced-motion: no-preference) {
  .line {
    animation: pulse 1.4s ease-in-out infinite;
  }
}
@keyframes pulse {
  50% {
    opacity: 0.5;
  }
}
</style>
