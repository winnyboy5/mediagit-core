<script setup lang="ts">
import {
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogOverlay,
  AlertDialogPortal,
  AlertDialogRoot,
  AlertDialogTitle,
  AlertDialogTrigger,
} from 'reka-ui'
import { TriangleAlert } from 'lucide-vue-next'

const open = defineModel<boolean>('open', { default: false })
withDefaults(
  defineProps<{
    title: string
    description: string
    confirmLabel: string
    cancelLabel?: string
    destructive?: boolean
  }>(),
  { cancelLabel: 'Cancel' },
)
const emit = defineEmits<{ confirm: [] }>()
</script>

<template>
  <AlertDialogRoot v-model:open="open">
    <AlertDialogTrigger v-if="$slots.trigger" as-child><slot name="trigger" /></AlertDialogTrigger>
    <AlertDialogPortal>
      <AlertDialogOverlay class="scrim" />
      <AlertDialogContent class="panel">
        <div class="head">
          <span v-if="destructive" class="warn"><TriangleAlert :size="20" aria-hidden="true" /></span>
          <div>
            <AlertDialogTitle class="title">{{ title }}</AlertDialogTitle>
            <AlertDialogDescription class="desc">{{ description }}</AlertDialogDescription>
          </div>
        </div>
        <footer>
          <AlertDialogCancel class="b secondary">{{ cancelLabel }}</AlertDialogCancel>
          <AlertDialogAction class="b" :class="destructive ? 'danger' : 'primary'" @click="emit('confirm')">
            {{ confirmLabel }}
          </AlertDialogAction>
        </footer>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 50;
  background: var(--scrim);
}
.panel {
  position: fixed;
  top: 50%;
  left: 50%;
  z-index: 51;
  width: min(460px, calc(100vw - var(--space-8)));
  transform: translate(-50%, -50%);
  padding: var(--space-6);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow-3);
}
.head {
  display: flex;
  gap: var(--space-4);
}
.warn {
  display: grid;
  flex: none;
  place-items: center;
  width: 40px;
  height: 40px;
  border-radius: var(--radius-pill);
  background: var(--danger-soft);
  color: var(--danger);
}
.title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
}
.desc {
  margin: var(--space-2) 0 0;
  color: var(--text-muted);
}
footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-3);
  margin-top: var(--space-6);
}
.b {
  height: var(--control-h);
  padding: 0 var(--space-4);
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  font: 500 var(--fs-md) / 1 var(--font-sans);
  cursor: pointer;
}
.secondary {
  background: var(--surface);
  color: var(--text);
  border-color: var(--border-strong);
}
.primary {
  background: var(--accent);
  color: var(--on-accent);
}
.danger {
  background: var(--danger-soft);
  color: var(--danger);
  border-color: var(--danger);
}
.danger:hover {
  background: var(--danger);
  color: var(--surface);
}
</style>
