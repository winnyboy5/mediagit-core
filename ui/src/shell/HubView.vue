<script setup lang="ts">
// One page per hub: the hub name, one section control, hub-wide actions, then the section.
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import SegmentedControl from '@/components/SegmentedControl.vue'
import ActivityPill from './ActivityPill.vue'
import HubActions from './HubActions.vue'
import { hubById, sectionOf } from './hubs'

const route = useRoute()
const router = useRouter()

const hub = computed(() => hubById(route.meta.hub)!)
const section = computed(() => sectionOf(hub.value, route.params.section as string | undefined))
const options = computed(() => hub.value.sections.map((s) => ({ value: s.id, label: s.label, count: s.count?.() })))
const current = computed({
  get: () => section.value.id,
  set: (id: string) => void router.push(`/${hub.value.id}/${id}`),
})
const listed = computed(() => hub.value.layout === 'list')
</script>

<template>
  <div class="hub">
    <header class="hubhead">
      <h1>{{ hub.label }}</h1>
      <SegmentedControl v-if="!listed && options.length > 1" v-model="current" label="Section" :options="options" />
      <span class="grow" />
      <ActivityPill />
      <HubActions v-if="hub.sync" />
    </header>
    <div class="body" :class="{ listed }">
      <nav v-if="listed" class="list" aria-label="Section">
        <button
          v-for="o in options"
          :key="o.value"
          type="button"
          class="it"
          :aria-current="current === o.value ? 'page' : undefined"
          @click="current = o.value"
        >
          {{ o.label }}
        </button>
      </nav>
      <component :is="section.component" v-bind="section.props" :key="hub.id + section.id" />
    </div>
  </div>
</template>

<style scoped>
.hub {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
.hubhead {
  display: flex;
  flex: none;
  align-items: center;
  gap: var(--space-4);
  /* one height for every hub, with or without Get latest / Send */
  height: 64px;
  padding: 0 var(--space-6);
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}
h1 {
  margin: 0;
  font-size: var(--fs-xl);
  font-weight: 600;
}
.grow {
  flex: 1;
}
.body {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
.body.listed {
  flex-direction: row;
}
.list {
  display: grid;
  flex: none;
  align-content: start;
  gap: 2px;
  width: 200px;
  padding: var(--space-4) var(--space-3);
  border-right: 1px solid var(--border);
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
.listed > :not(.list) {
  flex: 1;
  min-width: 0;
}
@media (max-width: 900px) {
  .body.listed {
    flex-direction: column;
  }
  .list {
    display: flex;
    flex-wrap: wrap;
    width: auto;
    border-right: 0;
    border-bottom: 1px solid var(--border);
  }
}
</style>
