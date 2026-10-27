<script setup lang="ts">
import { TooltipContent, TooltipPortal, TooltipProvider, TooltipRoot, TooltipTrigger } from 'reka-ui'

defineProps<{ text: string }>()
</script>

<template>
  <TooltipProvider :delay-duration="400">
    <TooltipRoot>
      <TooltipTrigger as-child><slot /></TooltipTrigger>
      <TooltipPortal>
        <TooltipContent class="tip" :side-offset="6">{{ text }}</TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </TooltipProvider>
</template>

<style scoped>
.tip {
  z-index: 70;
  max-width: 260px;
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-control);
  background: var(--text);
  color: var(--bg);
  font-size: var(--fs-sm);
  box-shadow: var(--shadow-2);
}
</style>
