<script setup lang="ts">
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
  DialogTrigger,
} from 'reka-ui'
import { X } from 'lucide-vue-next'

const open = defineModel<boolean>('open', { default: false })
withDefaults(defineProps<{ title: string; description?: string; dismissable?: boolean }>(), { dismissable: true })
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogTrigger v-if="$slots.trigger" as-child><slot name="trigger" /></DialogTrigger>
    <DialogPortal>
      <DialogOverlay class="scrim" />
      <DialogContent
        class="panel"
        @escape-key-down="(e: Event) => !dismissable && e.preventDefault()"
        @pointer-down-outside="(e: Event) => !dismissable && e.preventDefault()"
        @interact-outside="(e: Event) => !dismissable && e.preventDefault()"
      >
        <header>
          <DialogTitle class="title">{{ title }}</DialogTitle>
          <DialogClose v-if="dismissable" class="x" aria-label="Close"><X :size="18" /></DialogClose>
        </header>
        <DialogDescription v-if="description" class="desc">{{ description }}</DialogDescription>
        <div class="body"><slot /></div>
        <footer v-if="$slots.footer"><slot name="footer" /></footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
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
  width: min(520px, calc(100vw - var(--space-8)));
  max-height: calc(100vh - var(--space-12));
  overflow: auto;
  transform: translate(-50%, -50%);
  padding: var(--space-6);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow-3);
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}
.title {
  margin: 0;
  font-size: var(--fs-xl);
  font-weight: 600;
  line-height: 1.25;
}
.x {
  display: grid;
  place-items: center;
  width: var(--control-h-sm);
  height: var(--control-h-sm);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}
.x:hover {
  background: var(--surface-2);
  color: var(--text);
}
.desc {
  margin: var(--space-2) 0 0;
  color: var(--text-muted);
}
.body {
  margin-top: var(--space-5);
}
footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-3);
  margin-top: var(--space-6);
}
@media (prefers-reduced-motion: no-preference) {
  .panel[data-state='open'] {
    animation: pop var(--dur) var(--ease);
  }
}
@keyframes pop {
  from {
    opacity: 0;
    transform: translate(-50%, calc(-50% + 6px));
  }
}
</style>
