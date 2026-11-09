<script setup lang="ts">
// Where a ref has pointed, newest first. "Restore to here" is a soft reset behind the usual consequence dialog.
import { computed, onMounted, ref, watch } from 'vue'
import { History } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import TextField from '@/components/TextField.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { formatWhen, shortOid } from '@/lib/history'
import { ALL_REFS, useReflogStore } from '@/stores/advanced'
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

const refs = computed(() => [{ value: 'HEAD', label: 'HEAD (where you are)' }, { value: ALL_REFS, label: 'Every branch and tag (newest only)' }, ...branches.list.map((b) => ({ value: b.name, label: b.name }))])
const first = computed(() => reflog.entries[0]?.index)
const all = computed(() => reflog.reference === ALL_REFS)

// Cleaning up cannot be undone: both ask first.
const keep = ref('30')
const keepNum = computed(() => (/^\d{1,6}$/.test(String(keep.value).trim()) ? Number(keep.value) : null))
const scopeAll = ref(false)
const clearOpen = ref(false)
const trimOpen = ref(false)
const trimTarget = computed(() => (scopeAll.value === true || all.value ? null : reflog.reference))
</script>

<template>
  <div class="panel" data-panel="reflog" :aria-busy="reflog.loading || undefined">
    <p class="muted">Every place this branch or your current position has been. If a version seems lost, it is probably listed here.</p>
    <div class="tools"><Select v-model="reflog.reference" label="Look at" :options="refs" /></div>

    <UndoDialogs />

    <div v-if="!reflog.loaded && !reflog.error"><SkeletonRows :count="6" /></div>
    <ErrorState v-else-if="reflog.error && !reflog.loaded" :code="reflog.error.code" :message="reflog.error.message" @retry="reflog.refresh()" />
    <div v-else-if="all" class="groups" data-testid="reflog-all">
      <section v-for="g in reflog.groups" :key="g.reference" class="group">
        <h3 class="mono">{{ g.reference }}</h3>
        <ol class="rows" :aria-label="`Reflog of ${g.reference}, newest first`">
          <li v-for="e in g.entries" :key="e.index" class="row">
            <span class="idx mono">@{{ e.index }}</span>
            <div class="msg">
              <strong>{{ e.message || '(no message)' }}</strong>
              <span class="muted mono">{{ shortOid(e.old_oid) }} → {{ shortOid(e.new_oid) }}</span>
            </div>
            <time class="muted" :datetime="new Date(e.time * 1000).toISOString()">{{ formatWhen(e.time) }}</time>
          </li>
        </ol>
      </section>
      <p v-if="!reflog.groups.length" class="muted">Nothing has moved any branch yet.</p>
    </div>
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

    <MoreOptions label="Clean up this log">
      <p class="muted">These remove records of where branches have been, so “Restore to here” can no longer reach them. They cannot be undone.</p>
      <div v-if="reflog.actionError" class="banner" role="alert">{{ reflog.actionError.message }}</div>
      <TextField v-model="keep" label="Entries to keep" hint="The newest ones stay; older ones are forgotten." flag="--keep" :error="keepNum === null ? 'Whole number from 0 to 999999' : undefined" />
      <OptionCheck v-model="scopeAll" label="Trim every branch and tag, not just this one" hint="Leave off to trim only what is shown above" :disabled="all" />
      <div class="cleanup">
        <Button variant="danger" :disabled="keepNum === null || reflog.busy" @click="trimOpen = true">Remove older entries…</Button>
        <Button variant="danger" :disabled="all || reflog.busy" @click="clearOpen = true">Forget all of {{ reflog.reference }}…</Button>
      </div>
    </MoreOptions>

    <AlertDialog
      v-model:open="trimOpen"
      title="Remove older undo entries?"
      :description="`Everything but the newest ${keepNum} ${keepNum === 1 ? 'entry' : 'entries'} of ${trimTarget ?? 'every branch and tag'} is forgotten. You will not be able to restore to those positions.`"
      confirm-label="Remove entries"
      destructive
      @confirm="keepNum !== null && reflog.trim(trimTarget, keepNum)"
    />
    <AlertDialog
      v-model:open="clearOpen"
      :title="`Forget everything about ${reflog.reference}?`"
      description="Its whole undo log is deleted. You will not be able to restore to any earlier position of it."
      confirm-label="Forget it all"
      destructive
      @confirm="reflog.forget(reflog.reference)"
    />
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
.groups {
  display: grid;
  gap: var(--space-4);
}
.group h3 {
  margin: 0 0 var(--space-2);
  font-size: var(--fs-md);
}
.cleanup {
  display: flex;
  gap: var(--space-2);
}
.banner {
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
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
