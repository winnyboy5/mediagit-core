<script setup lang="ts">
import { computed } from 'vue'
import { mediaKind } from './media'

const props = defineProps<{ path: string }>()
const info = computed(() => mediaKind(props.path))
</script>

<template>
  <span
    class="ficon"
    :data-kind="info.kind"
    :style="{ color: `var(${info.hueVar})`, background: `var(${info.softVar})` }"
    aria-hidden="true"
  >
    <component :is="info.icon" :size="18" :stroke-width="1.75" />
  </span>
</template>

<style scoped>
.ficon {
  display: inline-grid;
  flex: none;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-control);
}
</style>
