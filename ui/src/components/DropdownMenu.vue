<script setup lang="ts">
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from 'reka-ui'

export interface MenuItem {
  id: string
  label: string
  danger?: boolean
  disabled?: boolean
  /** Draw a separator above this item. */
  separated?: boolean
}

defineProps<{ items: MenuItem[] }>()
const emit = defineEmits<{ select: [id: string] }>()
</script>

<template>
  <DropdownMenuRoot>
    <DropdownMenuTrigger as-child><slot /></DropdownMenuTrigger>
    <DropdownMenuPortal>
      <DropdownMenuContent class="menu" :side-offset="6" align="end">
        <template v-for="it in items" :key="it.id">
          <DropdownMenuSeparator v-if="it.separated" class="sep" />
          <DropdownMenuItem
            class="item"
            :class="{ danger: it.danger }"
            :disabled="it.disabled"
            @select="emit('select', it.id)"
          >
            {{ it.label }}
          </DropdownMenuItem>
        </template>
      </DropdownMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>

<style scoped>
.menu {
  z-index: 60;
  min-width: 200px;
  padding: var(--space-1);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow-2);
}
.item {
  display: flex;
  align-items: center;
  height: var(--control-h);
  padding: 0 var(--space-3);
  border-radius: var(--radius-control);
  cursor: pointer;
  outline: none;
}
.item[data-highlighted] {
  background: var(--accent-soft);
}
.item[data-disabled] {
  opacity: 0.5;
  cursor: not-allowed;
}
.item.danger {
  color: var(--danger);
}
.sep {
  height: 1px;
  margin: var(--space-1) 0;
  background: var(--border);
}
</style>
