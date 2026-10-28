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
  <!-- Non-modal: a modal menu aria-hides the whole app while focusable controls remain in it (axe aria-hidden-focus). -->
  <DropdownMenuRoot :modal="false">
    <DropdownMenuTrigger as-child><slot /></DropdownMenuTrigger>
    <DropdownMenuPortal>
      <DropdownMenuContent class="mg-menu" :side-offset="6" align="end">
        <template v-for="it in items" :key="it.id">
          <DropdownMenuSeparator v-if="it.separated" class="mg-menu-sep" />
          <DropdownMenuItem
            class="mg-menu-item"
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

<style>
/* Unscoped: the menu renders in a portal, outside this component's scope id. */
.mg-menu {
  z-index: 60;
  min-width: 200px;
  padding: var(--space-1);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow-2);
}
.mg-menu-item {
  display: flex;
  align-items: center;
  height: var(--control-h);
  padding: 0 var(--space-3);
  border-radius: var(--radius-control);
  cursor: pointer;
  outline: none;
}
.mg-menu-item[data-highlighted] {
  background: var(--accent-soft);
}
.mg-menu-item[data-disabled] {
  opacity: 0.5;
  cursor: not-allowed;
}
.mg-menu-item.danger {
  color: var(--danger);
}
.mg-menu-sep {
  height: 1px;
  margin: var(--space-1) 0;
  background: var(--border);
}
</style>
