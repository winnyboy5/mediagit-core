<script setup lang="ts">
// Where a ref has pointed, newest first. "Restore to here" is a soft reset behind the usual consequence dialog.
import { computed, onMounted, watch } from 'vue'
import { History } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { formatWhen, shortOid } from '@/lib/history'
import { useReflogStore } from '@/stores/advanced'
import { useBranchesStore } from '@/stores/branches'
import { useUndoStore } from '@/stores/undo'
import UndoDialogs from '../history/UndoDialogs.vue'

const reflog = useReflogStore()
const branches = useBranchesStore()
const undo = useUndoStore()
onMounted(() => void reflog.refresh())
watch(() => reflog.reference, () => void reflog.refresh())
// A restore moves the ref: read the list again once it is done.
watch(
  () => undo.busy,
  (b, was) => was && !b && void reflog.refresh(),
)

const refs = computed(() => [{ value: 'HEAD', label: 'HEAD (where you are)' }, ...branches.list.map((b) => ({ value: b.name, label: b.name }))])
const first = computed(() => reflog.entries[0]?.index)
</script>

<template>
  <div class="panel" data-panel="reflog" :aria-busy="reflog.loading || undefined">
    <p class="muted">Every place this branch or your current position has been. If a version seems lost, it is probably listed here.</p>
    <div class="tools"><Select v-model="reflog.reference" label="Look at" :options="refs" /></div>

    <UndoDialogs />

    <div v-if="!reflog.loaded && !reflog.error"><SkeletonRows :count="6" /></div>
    <ErrorState v-else-if="reflog.error && !reflog.loaded" :code="reflog.error.code" :message="reflog.error.message" @retry="reflog.refresh()" />
    <EmptyState v-else-if="!reflog.entries.length" title="No history for this ref" description="Nothing has moved it yet.">
      <template #icon><History :size="24" /></template>
    </EmptyState>
    <ol v-else class="rows" aria-label="Reflog, newest first">
      <li v-for="e in reflog.entries" :key="e.index" class="row" :data-index="e.index">
        <span class="idx mono">@{{ e.index }}</span>
        <div class="msg">
          <strong>{{ e.message || '(no message)' }}</strong>
          <span class="muted mono">{{ shortOid(e.old_oid) }} → {{ shortOid(e.new_oid) }}</span>
        </div>
        <time class="muted" :datetime="new Date(e.time * 1000).toISOString()">{{ formatWhen(e.time) }}</time>
        <Button
          size="sm"
          :disabled="undo.busy || e.index === first"
          :aria-label="`Restore to @${e.index}`"
          @click="undo.ask({ kind: 'reset', mode: 'soft', rev: e.new_oid })"
        >
          Restore to here…
        </Button>
      </li>
    </ol>
  </div>
</template>

<style scoped>
.panel {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-4) 0;
}
.muted {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.tools :deep(.field) {
  max-width: 320px;
}
.rows {
  margin: 0;
  padding: 0;
  list-style: none;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.row {
  display: grid;
  grid-template-columns: 56px minmax(0, 1fr) 160px auto;
  align-items: center;
  gap: var(--space-4);
  min-height: calc(var(--row-h) + var(--space-4));
  padding: var(--space-2) var(--space-5);
  border-bottom: 1px solid var(--border);
}
.row:last-child {
  border-bottom: 0;
}
.idx {
  color: var(--text-muted);
}
.msg {
  display: grid;
  min-width: 0;
  overflow-wrap: anywhere;
}
</style>
