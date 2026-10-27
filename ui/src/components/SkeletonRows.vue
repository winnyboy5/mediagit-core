<script setup lang="ts">
withDefaults(defineProps<{ count?: number }>(), { count: 5 })
</script>

<template>
  <div class="sk" role="status" aria-label="Loading">
    <div v-for="i in count" :key="i" class="row" aria-hidden="true">
      <span class="ico" />
      <span class="line" :style="{ width: 38 + ((i * 17) % 40) + '%' }" />
      <span class="size" />
    </div>
  </div>
</template>

<style scoped>
.row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  height: var(--row-h);
  padding: 0 var(--space-4);
}
.ico,
.line,
.size {
  background: var(--surface-3);
  border-radius: var(--radius-control);
}
.ico {
  width: 28px;
  height: 28px;
}
.line {
  height: 12px;
}
.size {
  width: 56px;
  height: 12px;
  margin-left: auto;
}
@media (prefers-reduced-motion: no-preference) {
  .ico,
  .line,
  .size {
    animation: pulse 1.4s ease-in-out infinite;
  }
}
@keyframes pulse {
  50% {
    opacity: 0.5;
  }
}
</style>
