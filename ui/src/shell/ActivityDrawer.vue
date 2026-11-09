<script setup lang="ts">
import { DialogContent, DialogDescription, DialogPortal, DialogRoot, DialogTitle } from 'reka-ui'
import { useOpsStore } from '@/stores/ops'
import { useShellStore } from '@/stores/shell'
import Button from '@/components/Button.vue'
import OpRow from './OpRow.vue'

const ops = useOpsStore()
const shell = useShellStore()

// The pill toggles the drawer; don't let the outside-press close it first.
function keepOpenForBar(e: Event) {
  const t = (e as CustomEvent<{ originalEvent: Event }>).detail?.originalEvent?.target
  if (t instanceof Element && t.closest('[data-activity-pill]')) e.preventDefault()
}
</script>

<template>
  <DialogRoot :open="shell.activityOpen" :modal="false" @update:open="(o: boolean) => (shell.activityOpen = o)">
    <DialogPortal>
      <DialogContent
        class="drawer"
        data-testid="activity-drawer"
        @interact-outside="keepOpenForBar"
        @focus-outside="keepOpenForBar"
      >
        <header>
          <DialogTitle class="title">Activity</DialogTitle>
          <Button size="sm" variant="secondary" @click="shell.activityOpen = false">Close</Button>
        </header>
        <DialogDescription class="sr-only">Running, waiting, finished and failed operations</DialogDescription>

        <div class="scroll">
          <section v-if="ops.running.length" aria-labelledby="act-running">
            <h3 id="act-running">Running</h3>
            <ul><OpRow v-for="o in ops.running" :key="o.id" :op="o" /></ul>
          </section>
          <section v-if="ops.queued.length" aria-labelledby="act-queued">
            <h3 id="act-queued">Queued</h3>
            <ul><OpRow v-for="o in ops.queued" :key="o.id" :op="o" /></ul>
          </section>
          <section v-if="ops.finished.some((o) => o.status === 'failed')" aria-labelledby="act-failed">
            <h3 id="act-failed">Failed</h3>
            <ul><OpRow v-for="o in ops.finished.filter((x) => x.status === 'failed')" :key="o.id" :op="o" /></ul>
          </section>
          <section v-if="ops.finished.some((o) => o.status !== 'failed')" aria-labelledby="act-done">
            <h3 id="act-done">Finished</h3>
            <ul><OpRow v-for="o in ops.finished.filter((x) => x.status !== 'failed')" :key="o.id" :op="o" /></ul>
          </section>
          <p v-if="!ops.running.length && !ops.queued.length && !ops.finished.length" class="none">
            Nothing has run yet. Saving versions, staging files and syncing show up here.
          </p>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
.drawer {
  position: fixed;
  z-index: 40;
  top: 0;
  right: 0;
  bottom: 57px;
  display: flex;
  flex-direction: column;
  width: min(440px, 100vw);
  border-left: 1px solid var(--border);
  background: var(--surface);
  box-shadow: var(--shadow-3);
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--border);
}
.title {
  margin: 0;
  font-size: var(--fs-xl);
  font-weight: 600;
}
.scroll {
  flex: 1;
  overflow-y: auto;
}
h3 {
  margin: 0;
  padding: var(--space-3) var(--space-5);
  background: var(--surface-2);
  color: var(--text-muted);
  font-size: var(--fs-md);
  font-weight: 600;
}
ul {
  margin: 0;
  padding: 0;
}
.none {
  margin: 0;
  padding: var(--space-8) var(--space-5);
  color: var(--text-muted);
}
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
}
</style>
