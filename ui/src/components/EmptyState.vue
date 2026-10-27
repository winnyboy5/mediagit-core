<script setup lang="ts">
defineProps<{ title: string; description?: string }>()
</script>

<template>
  <div class="empty">
    <span v-if="$slots.icon" class="ico" aria-hidden="true"><slot name="icon" /></span>
    <h3>{{ title }}</h3>
    <p v-if="description">{{ description }}</p>
    <div v-if="$slots.default" class="act"><slot /></div>
  </div>
</template>

<style scoped>
.empty {
  display: grid;
  justify-items: center;
  gap: var(--space-2);
  padding: var(--space-12) var(--space-6);
  text-align: center;
}
.ico {
  display: grid;
  place-items: center;
  width: 56px;
  height: 56px;
  margin-bottom: var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--surface-2);
  color: var(--text-muted);
}
h3 {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
}
p {
  max-width: 44ch;
  margin: 0;
  color: var(--text-muted);
  text-wrap: balance;
}
.act {
  margin-top: var(--space-4);
}
</style>
