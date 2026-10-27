<script setup lang="ts">
import { ToastClose, ToastDescription, ToastProvider, ToastRoot, ToastTitle, ToastViewport } from 'reka-ui'
import { CircleAlert, CircleCheck, Info, X } from 'lucide-vue-next'
import { useToast } from './useToast'

const { items, dismiss } = useToast()
const ICON = { success: CircleCheck, error: CircleAlert, info: Info }
</script>

<template>
  <ToastProvider :duration="6000" label="Notifications">
    <ToastRoot
      v-for="t in items"
      :key="t.id"
      class="mg-toast"
      :class="`mg-${t.kind}`"
      :type="t.kind === 'error' ? 'foreground' : 'background'"
      @update:open="(o: boolean) => !o && dismiss(t.id)"
    >
      <component :is="ICON[t.kind]" class="mg-ico" :size="20" aria-hidden="true" />
      <div class="mg-txt">
        <ToastTitle class="mg-t">{{ t.title }}</ToastTitle>
        <ToastDescription v-if="t.description" class="mg-d">{{ t.description }}</ToastDescription>
      </div>
      <ToastClose class="mg-x" aria-label="Dismiss"><X :size="16" /></ToastClose>
    </ToastRoot>
    <ToastViewport class="mg-viewport" />
  </ToastProvider>
</template>

<style>
/* Unscoped: reka renders the viewport list and toast roots outside this component's scope id. */
.mg-viewport {
  position: fixed;
  right: var(--space-6);
  bottom: calc(var(--space-6) + 56px); /* clear the activity bar */
  z-index: 80;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  width: 380px;
  max-width: calc(100vw - var(--space-8));
  margin: 0;
  padding: 0;
  list-style: none;
}
.mg-toast {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow-2);
}
.mg-ico {
  flex: none;
  margin-top: 1px;
}
.mg-success .mg-ico {
  color: var(--success);
}
.mg-error .mg-ico {
  color: var(--danger);
}
.mg-info .mg-ico {
  color: var(--accent);
}
.mg-txt {
  flex: 1;
  min-width: 0;
}
.mg-t {
  font-weight: 600;
}
.mg-d {
  margin-top: var(--space-1);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.mg-x {
  display: grid;
  place-items: center;
  width: var(--control-h-sm);
  height: var(--control-h-sm);
  margin: calc(var(--space-1) * -1);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}
.mg-x:hover {
  background: var(--surface-2);
  color: var(--text);
}
@media (prefers-reduced-motion: no-preference) {
  .mg-toast[data-state='open'] {
    animation: slide var(--dur) var(--ease);
  }
}
@keyframes slide {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
}
</style>
