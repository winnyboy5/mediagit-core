<script setup lang="ts">
import { computed } from 'vue'

// value == null => indeterminate (CLI reported no length yet): never fake a number.
const props = withDefaults(
  defineProps<{ label: string; value?: number | null; max?: number; size?: 'md' | 'sm' }>(),
  { max: 100, size: 'md' },
)
const pct = computed(() =>
  props.value == null || props.max <= 0 ? null : Math.min(100, Math.max(0, (props.value / props.max) * 100)),
)
</script>

<template>
  <div
    class="bar"
    :class="[size, { indeterminate: pct === null }]"
    role="progressbar"
    :aria-label="label"
    aria-valuemin="0"
    :aria-valuemax="max"
    :aria-valuenow="pct === null ? undefined : value!"
  >
    <div class="fill" :style="pct === null ? undefined : { width: pct + '%' }" />
  </div>
</template>

<style scoped>
.bar {
  position: relative;
  width: 100%;
  height: 8px;
  overflow: hidden;
  border: 1px solid var(--text);
  border-radius: var(--radius-pill);
  background: var(--surface);
}
.bar.sm {
  height: 4px;
}
.fill {
  height: 100%;
  border-radius: inherit;
  background: var(--brand);
  transition: width var(--dur) var(--ease);
}
.indeterminate .fill {
  width: 100%;
  background: repeating-linear-gradient(
    -45deg,
    var(--brand) 0 10px,
    color-mix(in srgb, var(--brand) 55%, var(--surface)) 10px 20px
  );
  background-size: 28px 28px;
  animation: stripe 900ms linear infinite;
}
@keyframes stripe {
  to {
    background-position: 28px 0;
  }
}
@media (prefers-reduced-motion: reduce) {
  .indeterminate .fill {
    animation: none;
  }
}
</style>
