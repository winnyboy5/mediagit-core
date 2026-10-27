<script setup lang="ts">
import { TabsContent, TabsList, TabsRoot, TabsTrigger } from 'reka-ui'

const model = defineModel<string>({ required: true })
defineProps<{ items: { value: string; label: string }[]; label: string }>()
</script>

<template>
  <TabsRoot v-model="model">
    <TabsList class="list" :aria-label="label">
      <TabsTrigger v-for="t in items" :key="t.value" :value="t.value" class="tab">{{ t.label }}</TabsTrigger>
    </TabsList>
    <TabsContent v-for="t in items" :key="t.value" :value="t.value" class="panel">
      <slot :value="t.value" />
    </TabsContent>
  </TabsRoot>
</template>

<style scoped>
.list {
  display: flex;
  gap: var(--space-1);
  border-bottom: 1px solid var(--border);
}
.tab {
  height: var(--control-h);
  padding: 0 var(--space-4);
  margin-bottom: -1px;
  border: 0;
  border-bottom: 2px solid transparent;
  background: transparent;
  color: var(--text-muted);
  font: 500 var(--fs-md) / 1 var(--font-sans);
  cursor: pointer;
}
.tab:hover {
  color: var(--text);
}
.tab[data-state='active'] {
  color: var(--text);
  border-bottom-color: var(--accent);
}
.panel {
  padding-top: var(--space-4);
}
</style>
