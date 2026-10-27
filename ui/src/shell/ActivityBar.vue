<script setup lang="ts">
import { computed } from 'vue'
import Button from '@/components/Button.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import TransferStats from '@/components/TransferStats.vue'
import { CANCELLABLE, etaOf, primaryBar, rateOf, useOpsStore } from '@/stores/ops'
import { useShellStore } from '@/stores/shell'

const ops = useOpsStore()
const shell = useShellStore()

// The bar shows only after the anti-flicker delay (ops.current).
const op = computed(() => ops.current)
const bar = computed(() => (op.value ? primaryBar(op.value) : undefined))
const others = computed(() => ops.running.length - (op.value ? 1 : 0) + ops.queued.length)
const canCancel = computed(() => !!op.value && CANCELLABLE.has(op.value.kind) && !!op.value.cancel)
const isBytes = computed(() => bar.value?.unit === 'bytes')
const value = computed(() => (bar.value?.len != null ? bar.value.pos : null))
const failed = computed(() => ops.finished.filter((o) => o.status === 'failed').length)
</script>

<template>
  <footer class="abar" data-activity-bar aria-label="Activity">
    <template v-if="op">
      <strong class="label">{{ op.label }}</strong>
      <span class="phase" aria-live="polite">{{ op.phase }}</span>
      <div class="bar">
        <ProgressBar :label="op.phase || op.label" :value="value" :max="bar?.len ?? 100" />
      </div>
      <TransferStats v-if="bar && isBytes" :done="bar.pos" :total="bar.len" :rate="rateOf(bar)" :eta="etaOf(bar)" />
      <p v-else-if="bar" class="items mono">{{ bar.pos }}<template v-if="bar.len != null"> / {{ bar.len }}</template> items</p>
      <span v-if="others" class="more mono">+{{ others }} more</span>
    </template>
    <span v-else class="idle">
      No activity<template v-if="failed"> · <span class="bad">{{ failed }} failed</span></template>
    </span>
    <span class="spacer" />
    <Button v-if="canCancel" size="sm" variant="secondary" @click="ops.cancel(op!.id)">Cancel</Button>
    <Button size="sm" variant="secondary" :aria-expanded="shell.activityOpen" aria-haspopup="dialog" @click="shell.activityOpen = !shell.activityOpen">
      Activity
    </Button>
  </footer>
</template>

<style scoped>
.abar {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  min-height: 56px;
  padding: var(--space-2) var(--space-4);
  border-top: 1px solid var(--border);
  background: var(--surface);
}
.label {
  font-weight: 600;
  white-space: nowrap;
}
.phase {
  color: var(--text-muted);
  white-space: nowrap;
}
.bar {
  flex: 1 1 160px;
  min-width: 120px;
  max-width: 420px;
}
.items {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.more {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.idle {
  color: var(--text-muted);
}
.bad {
  color: var(--danger);
}
.spacer {
  flex: 1;
}
</style>
