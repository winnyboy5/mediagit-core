<script setup lang="ts">
// A section list on the left, its page on the right. For pages with 4-6 parts
// that are too heavy for tabs (Server, Advanced tools); never nested in tabs.
const model = defineModel<string>({ required: true })
defineProps<{ items: { value: string; label: string }[]; label: string }>()
</script>

<template>
  <div class="sidelist">
    <nav class="nav" :aria-label="label">
      <button
        v-for="t in items"
        :key="t.value"
        type="button"
        class="it"
        :aria-current="model === t.value ? 'page' : undefined"
        @click="model = t.value"
      >
        {{ t.label }}
      </button>
    </nav>
    <div class="page"><slot :value="model" /></div>
  </div>
</template>

<style scoped>
.sidelist {
  display: flex;
  gap: var(--space-6);
  align-items: flex-start;
  min-width: 0;
}
.nav {
  display: grid;
  flex: none;
  gap: 2px;
  width: 200px;
}
.it {
  min-height: var(--control-h);
  padding: var(--space-1) var(--space-3);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text);
  font: 400 var(--fs-md) / 1.25 var(--font-sans);
  text-align: left;
  cursor: pointer;
}
.it:hover {
  background: var(--surface-2);
}
.it[aria-current='page'] {
  background: var(--accent-soft);
  font-weight: 600;
}
.page {
  flex: 1;
  min-width: 0;
}
@media (max-width: 900px) {
  .sidelist {
    flex-direction: column;
    gap: var(--space-3);
  }
  .nav {
    display: flex;
    flex-wrap: wrap;
    width: auto;
  }
}
</style>
