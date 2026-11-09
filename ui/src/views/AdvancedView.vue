<script setup lang="ts">
import { onMounted, ref } from 'vue'
import Tabs from '@/components/Tabs.vue'
import { useVersionsStore } from '@/stores/advanced'
import { useBranchesStore } from '@/stores/branches'
import BisectPanel from './advanced/BisectPanel.vue'
import ReflogPanel from './advanced/ReflogPanel.vue'
import SequencerPanel from './advanced/SequencerPanel.vue'

const branches = useBranchesStore()
const versions = useVersionsStore()
onMounted(() => {
  void branches.refresh()
  void versions.load()
})

const TABS = [
  { value: 'rebase', label: 'Replay onto a branch (rebase)' },
  { value: 'pick', label: 'Copy a version (cherry-pick)' },
  { value: 'bisect', label: 'Find the break (bisect)' },
  { value: 'reflog', label: 'Undo log (reflog)' },
]
const tab = ref('rebase')
</script>

<template>
  <section class="adv">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">Advanced</h2>
      </div>
    </header>
    <Tabs v-model="tab" label="Advanced tools" :items="TABS">
      <template #default="{ value }">
        <SequencerPanel v-if="value === 'rebase'" kind="rebase" />
        <SequencerPanel v-else-if="value === 'pick'" kind="pick" />
        <BisectPanel v-else-if="value === 'bisect'" />
        <ReflogPanel v-else />
      </template>
    </Tabs>
  </section>
</template>

<style scoped>
.adv {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
  overflow-y: auto;
}
.ttl {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
}
.pt {
  margin: 0;
  font-size: var(--fs-xl);
}
.ttl p {
  margin: 0;
  color: var(--text-muted);
}
</style>
