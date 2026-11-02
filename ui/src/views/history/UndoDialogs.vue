<script setup lang="ts">
// Confirm dialogs for every undo action, plus the banner for a revert that stopped on clashes.
// Mounted once per view that offers undo; all state lives in the undo store.
import { computed, ref, watch } from 'vue'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Dialog from '@/components/Dialog.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import { shortOid } from '@/lib/history'
import { primaryBar } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useUndoStore } from '@/stores/undo'

const undo = useUndoStore()
const repo = useRepoStore()

const p = computed(() => undo.pending)
const kind = computed(() => (p.value?.kind === 'reset' ? (p.value.mode === 'hard' ? 'hard' : 'reset') : p.value?.kind))
const rev = computed(() => (p.value ? (p.value.rev === 'HEAD' ? 'the latest version' : shortOid(p.value.rev)) : ''))
const isOpen = (k: string) => computed({ get: () => undo.open && kind.value === k, set: (o: boolean) => !o && (undo.open = false) })
const resetOpen = isOpen('reset')
const hardOpen = isOpen('hard')
const revertOpen = isOpen('revert')
const restoreOpen = isOpen('restore')

const s = computed(() => repo.status?.summary)
const unsaved = computed(() => (s.value ? s.value.staged + s.value.modified + s.value.deleted : 0))
const understood = ref<boolean | 'indeterminate'>(false)
watch(hardOpen, (o) => o && (understood.value = false))

const resetText = computed(() =>
  p.value?.kind === 'reset' && p.value.mode === 'soft'
    ? 'The branch moves back and later versions leave it. Their changes stay ready to save, and your files are not touched.'
    : 'The branch moves back and later versions leave it. Their changes stay in your files, not yet staged. Nothing is deleted.',
)
const bar = computed(() => (undo.running ? primaryBar(undo.running) : undefined))
</script>

<template>
  <div class="undo">
    <div v-if="undo.error" class="banner" role="alert">
      <code class="mono">{{ undo.error.code }}</code>
      <span>{{ undo.error.message }}</span>
      <Button size="sm" variant="ghost" @click="undo.error = null">Dismiss</Button>
    </div>

    <div v-if="undo.revertConflicts.length" class="banner warn" role="alert" data-testid="revert-stopped">
      <span>The undo stopped: {{ undo.revertConflicts.length }} {{ undo.revertConflicts.length === 1 ? 'file clashes' : 'files clash' }} ({{ undo.revertConflicts.join(', ') }}).</span>
      <Button size="sm" variant="danger" :disabled="undo.busy" @click="undo.revertControl('abort')">Abandon undo</Button>
    </div>

    <div v-if="undo.running" class="busy" role="status" data-testid="undo-progress">
      <span>{{ undo.running.label }}<template v-if="bar && bar.len != null"> · {{ bar.pos }} / {{ bar.len }} files</template></span>
      <ProgressBar :label="undo.running.label" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
    </div>

    <AlertDialog
      v-model:open="resetOpen"
      :title="`Move this branch back to ${rev}?`"
      :description="resetText"
      confirm-label="Move branch back"
      destructive
      @confirm="undo.confirm()"
    />

    <AlertDialog
      v-model:open="revertOpen"
      :title="`Undo ${p?.kind === 'revert' ? p.summary || rev : ''}?`"
      description="This adds a new version that reverses it. Nothing is deleted, and the original stays in History."
      confirm-label="Undo this version"
      @confirm="undo.confirm()"
    />

    <AlertDialog
      v-model:open="restoreOpen"
      :title="`Restore ${p?.kind === 'restore' ? p.path : ''}?`"
      :description="`Your copy of this file is replaced with the one from ${rev}. Unsaved changes to this file are lost.`"
      confirm-label="Restore file"
      destructive
      @confirm="undo.confirm()"
    />

    <Dialog v-model:open="hardOpen" :title="`Discard everything back to ${rev}?`" description="This cannot be undone.">
      <ul class="cons">
        <li>Your files are replaced with how they were at {{ rev }}.</li>
        <li v-if="unsaved"><strong>{{ unsaved }} {{ unsaved === 1 ? 'file with unsaved changes' : 'files with unsaved changes' }} will be lost.</strong></li>
        <li v-else>You have no unsaved changes to lose.</li>
        <li v-if="p?.rev !== 'HEAD'">Versions after it leave this branch. They stay in the project until it is cleaned up.</li>
      </ul>
      <Checkbox v-model="understood" label="I understand my unsaved work will be lost" />
      <template #footer>
        <Button @click="undo.open = false">Keep my files</Button>
        <Button variant="danger" :disabled="understood !== true || undo.busy" @click="undo.confirm()">Discard and go back</Button>
      </template>
    </Dialog>
  </div>
</template>

<style scoped>
.undo {
  display: contents;
}
.banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin: var(--space-3) var(--space-4) 0;
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
}
.banner.warn {
  border-color: var(--conflict);
  background: var(--conflict-soft);
}
.banner span {
  flex: 1;
}
.busy {
  display: grid;
  gap: var(--space-2);
  margin: var(--space-3) var(--space-4) 0;
  font-size: var(--fs-sm);
}
.cons {
  display: grid;
  gap: var(--space-2);
  margin: 0 0 var(--space-4);
  padding-left: var(--space-5);
}
</style>
