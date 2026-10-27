<script setup lang="ts">
import { computed } from 'vue'
import Button from '@/components/Button.vue'
import { formatEta } from '@/components/format'
import ProgressBar from '@/components/ProgressBar.vue'
import TransferStats from '@/components/TransferStats.vue'
import { CANCELLABLE, etaOf, primaryBar, rateOf, type Op, useOpsStore } from '@/stores/ops'

const props = defineProps<{ op: Op }>()
const ops = useOpsStore()

const bars = computed(() => [...props.op.bars.values()].filter((b) => !b.done))
const eta = computed(() => {
  const b = primaryBar(props.op)
  return b ? etaOf(b) : null
})
const hasBytes = computed(() => bars.value.some((b) => b.unit === 'bytes'))
const took = computed(() =>
  props.op.finishedAt == null ? null : formatEta((props.op.finishedAt - props.op.startedAt) / 1000),
)
const STATE = { queued: 'Waiting', running: 'Running', done: 'Done', failed: 'Failed', cancelled: 'Cancelled' } as const
</script>

<template>
  <li class="op" :class="op.status" :data-status="op.status">
    <div class="head">
      <strong class="label">{{ op.label }}</strong>
      <span v-if="op.status === 'running' && hasBytes" class="meta mono">ETA {{ formatEta(eta) }}</span>
      <span v-else-if="op.status === 'done' && took" class="meta mono">{{ took }}</span>
      <span v-else class="meta">{{ STATE[op.status] }}</span>
    </div>

    <p v-if="op.status === 'queued'" class="sub">Waiting for the previous operation to finish</p>
    <p v-else-if="op.status === 'running'" class="sub">{{ op.phase || 'Starting…' }}</p>
    <p v-else-if="op.status === 'done' && op.summary" class="sub">{{ op.summary }}</p>

    <ul v-if="op.status === 'running' && bars.length" class="bars">
      <li v-for="b in bars" :key="b.id" class="b">
        <span class="bl">{{ b.msg || 'Working' }}</span>
        <ProgressBar size="sm" :label="b.msg || op.label" :value="b.len != null ? b.pos : null" :max="b.len ?? 100" />
        <TransferStats v-if="b.unit === 'bytes'" :done="b.pos" :total="b.len" :rate="rateOf(b)" :eta="etaOf(b)" />
        <span v-else class="mono items">{{ b.pos }}<template v-if="b.len != null"> / {{ b.len }}</template></span>
      </li>
    </ul>

    <div v-if="op.status === 'failed' || op.status === 'cancelled'" class="err">
      <code v-if="op.error" class="code mono">{{ op.error.code }}</code>
      <span class="msg">{{ op.error?.message ?? 'Stopped' }}</span>
      <Button v-if="op.status === 'failed' && op.retry" size="sm" variant="secondary" @click="op.retry()">Retry</Button>
    </div>

    <Button
      v-if="op.status === 'running' && CANCELLABLE.has(op.kind) && op.cancel"
      class="cancel"
      size="sm"
      variant="secondary"
      @click="ops.cancel(op.id)"
    >
      Cancel
    </Button>
  </li>
</template>

<style scoped>
.op {
  display: grid;
  gap: var(--space-2);
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--border);
  list-style: none;
}
.op.failed {
  background: var(--danger-soft);
}
.head {
  display: flex;
  justify-content: space-between;
  gap: var(--space-3);
}
.label {
  overflow-wrap: anywhere;
}
.meta {
  flex: none;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.sub {
  margin: 0;
  color: var(--text-muted);
}
.bars {
  display: grid;
  gap: var(--space-3);
  margin: var(--space-1) 0 0;
  padding: 0;
  list-style: none;
}
.b {
  display: grid;
  gap: var(--space-1);
}
.bl {
  font-size: var(--fs-sm);
}
.items {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.err {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}
.code {
  padding: 0 var(--space-2);
  border: 1px solid var(--danger);
  border-radius: var(--radius-pill);
  color: var(--danger);
  font-size: var(--fs-xs);
}
.msg {
  flex: 1;
  min-width: 0;
  color: var(--text);
}
.cancel {
  justify-self: start;
}
</style>
