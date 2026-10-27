<script setup lang="ts">
import { formatBytes, formatEta, formatRate } from './format'

// rate / eta stay `--` until the engine has measured them.
defineProps<{ done?: number | null; total?: number | null; rate?: number | null; eta?: number | null }>()
</script>

<template>
  <p class="stats mono">
    <span>{{ done == null ? '--' : formatBytes(done) }}<template v-if="total != null"> / {{ formatBytes(total) }}</template></span>
    <span class="rate">{{ formatRate(rate) }}</span>
    <span class="eta">ETA {{ formatEta(eta) }}</span>
  </p>
</template>

<style scoped>
.stats {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1) var(--space-4);
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
