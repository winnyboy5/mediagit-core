<script setup lang="ts">
// "Undo…" menu for one version: move the branch back, revert, or (after a consequence dialog) discard everything back to it.
import Button from '@/components/Button.vue'
import DropdownMenu, { type MenuItem } from '@/components/DropdownMenu.vue'
import { useUndoStore } from '@/stores/undo'

const props = defineProps<{ rev: string; summary: string; disabled?: boolean }>()
const undo = useUndoStore()

const ITEMS: MenuItem[] = [
  { id: 'revert', label: 'Undo this version (adds a new one)' },
  { id: 'soft', label: 'Move branch back here, keep changes ready to save' },
  { id: 'mixed', label: 'Move branch back here, keep changes in my files' },
  { id: 'hard', label: 'Discard everything back to here…', danger: true, separated: true },
]

function pick(id: string) {
  if (id === 'revert') undo.ask({ kind: 'revert', rev: props.rev, summary: props.summary })
  else if (id === 'soft' || id === 'mixed' || id === 'hard') undo.ask({ kind: 'reset', mode: id, rev: props.rev })
}
</script>

<template>
  <DropdownMenu :items="ITEMS" @select="pick">
    <Button size="sm" :disabled="disabled || undo.busy">Undo…</Button>
  </DropdownMenu>
</template>
