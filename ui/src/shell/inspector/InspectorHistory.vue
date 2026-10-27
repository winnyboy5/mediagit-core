<script setup lang="ts">
// The last few versions that touched the selected file; click one to open it in History.
import { computed, watchEffect } from 'vue'
import { useRouter } from 'vue-router'
import type { InspectFile } from '@/lib/inspect'
import { shortOid } from '@/lib/history'
import { timeAgo } from '@/lib/persist'
import { useHistoryStore } from '@/stores/history'
import { useInspectorStore } from '@/stores/inspector'

const props = defineProps<{ file: InspectFile }>()
const inspector = useInspectorStore()
const history = useHistoryStore()
const router = useRouter()

watchEffect(() => inspector.ensureHistory(props.file))
const entry = computed(() => inspector.histories.get(props.file.path))

async function open(oid: string) {
  await router.push('/history')
  void history.select(oid)
}
</script>

<template>
  <section aria-labelledby="insp-hist" data-testid="file-history">
    <h3 id="insp-hist">History of this file</h3>
    <div v-if="!entry || entry.status === 'loading'" class="sk" role="status" aria-label="Loading file history">
      <span v-for="i in 3" :key="i" class="line" />
    </div>
    <p v-else-if="entry.status === 'error'" class="note">History is not available: {{ entry.error?.message }}</p>
    <p v-else-if="!entry.value?.length" class="note">No saved version includes this file yet.</p>
    <ul v-else>
      <li v-for="c in entry.value" :key="c.oid">
        <button type="button" @click="open(c.oid)">
          <span class="sum">{{ c.summary || '(no message)' }}</span>
          <span class="by">{{ c.author.name }} · {{ timeAgo(c.time * 1000) }} · <span class="mono">{{ shortOid(c.oid) }}</span></span>
        </button>
      </li>
    </ul>
  </section>
</template>

<style scoped>
h3 {
  margin: 0 0 var(--space-2);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
ul {
  display: grid;
  gap: var(--space-1);
  margin: 0;
  padding: 0;
  list-style: none;
}
button {
  display: grid;
  gap: 2px;
  width: 100%;
  min-height: var(--control-h);
  padding: var(--space-2) var(--space-3);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
button:hover {
  background: var(--surface-2);
}
button:focus-visible {
  outline: var(--focus-ring);
  outline-offset: var(--focus-offset);
}
.sum {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.by {
  color: var(--text-muted);
  font-size: var(--fs-xs);
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
  height: 32px;
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
