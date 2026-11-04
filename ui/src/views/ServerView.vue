<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Plus } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import Select from '@/components/Select.vue'
import Tabs from '@/components/Tabs.vue'
import TextField from '@/components/TextField.vue'
import { useServerStore } from '@/stores/server'
import AccessTab from './server/AccessTab.vue'
import KeysTab from './server/KeysTab.vue'
import OverviewTab from './server/OverviewTab.vue'
import PeopleTab from './server/PeopleTab.vue'
import ReposTab from './server/ReposTab.vue'

const server = useServerStore()
onMounted(() => void server.init())

const TABS = [
  { value: 'overview', label: 'Overview' },
  { value: 'people', label: 'People' },
  { value: 'access', label: 'Access' },
  { value: 'keys', label: 'API keys' },
  { value: 'repos', label: 'Repositories' },
]
const tab = ref('overview')

const options = computed(() => server.targets.map((t) => ({ value: t.id, label: t.label })))
const selected = computed({
  get: () => server.selected,
  set: (v) => void server.select(v),
})

// Add a server ---------------------------------------------------------------------
const addOpen = ref(false)
const address = ref('')
const addError = ref<string>()
function add() {
  if (server.addServer(address.value)) {
    addOpen.value = false
    address.value = ''
    addError.value = undefined
  } else addError.value = 'Use an http(s) address like https://mediagit.studio.example'
}
</script>

<template>
  <section class="srv" :aria-busy="server.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Server</h1>
        <p>people, access and repositories</p>
      </div>
      <div class="pick">
        <Select v-model="selected" label="Managing" :options="options" />
        <Button size="sm" @click="addOpen = true"><Plus :size="14" aria-hidden="true" /> Add server…</Button>
      </div>
    </header>

    <div v-if="server.actionError" class="banner" role="alert">
      <code class="mono">{{ server.actionError.code }}</code>
      <span>{{ server.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="server.actionError = null">Dismiss</Button>
    </div>

    <Tabs v-model="tab" label="Server sections" :items="TABS">
      <template #default="{ value }">
        <OverviewTab v-if="value === 'overview'" />
        <PeopleTab v-else-if="value === 'people'" @overview="tab = 'overview'" />
        <AccessTab v-else-if="value === 'access'" @overview="tab = 'overview'" />
        <KeysTab v-else-if="value === 'keys'" @overview="tab = 'overview'" />
        <ReposTab v-else @overview="tab = 'overview'" />
      </template>
    </Tabs>

    <Dialog v-model:open="addOpen" title="Add a server" description="Another MediaGit server you can manage, like your studio's.">
      <form id="add-server" class="form" autocomplete="off" @submit.prevent="add">
        <TextField v-model="address" label="Server address" placeholder="https://mediagit.studio.example" :error="addError" />
      </form>
      <template #footer>
        <Button @click="addOpen = false">Cancel</Button>
        <Button type="submit" form="add-server" variant="primary" :disabled="!address">Add server</Button>
      </template>
    </Dialog>
  </section>
</template>

<style scoped>
.srv {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
  overflow-y: auto;
}
.head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-4);
}
.ttl {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
}
h1 {
  margin: 0;
  font-size: var(--fs-2xl);
}
.ttl p {
  margin: 0;
  color: var(--text-muted);
}
.pick {
  display: flex;
  align-items: flex-end;
  gap: var(--space-2);
}
.banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
}
.banner span {
  flex: 1;
}
.form {
  display: grid;
  gap: var(--space-3);
}
</style>
