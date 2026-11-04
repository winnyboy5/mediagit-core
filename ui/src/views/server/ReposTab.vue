<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { FolderPlus, TriangleAlert } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TextField from '@/components/TextField.vue'
import { formatBytes } from '@/components/format'
import { timeAgo } from '@/lib/persist'
import { useServerStore } from '@/stores/server'
import NeedAccess from './NeedAccess.vue'

defineEmits<{ overview: [] }>()
const server = useServerStore()
const r = computed(() => server.repos)

watch(() => [server.url, server.access], () => (server.access === 'admin' || server.access === 'no_accounts') && void server.loadRepos(), { immediate: true })

// Create --------------------------------------------------------------------------
const makeOpen = ref(false)
const name = ref('')
const nameError = computed(() => (name.value && !/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(name.value) ? 'Use letters, numbers, dots, dashes and underscores' : undefined))
async function make() {
  if (!name.value || nameError.value || server.busy) return
  if (await server.createRepo(name.value)) {
    makeOpen.value = false
    name.value = ''
  }
}

// Delete: the name must be typed back ----------------------------------------------------
const doomed = ref<string | null>(null)
const typed = ref('')
const force = ref<boolean | 'indeterminate'>(false)
const doomedSize = computed(() => r.value.data.find((x) => x.name === doomed.value)?.size_bytes ?? 0)
const delOpen = computed({ get: () => doomed.value !== null, set: (v) => !v && (doomed.value = null) })
watch(doomed, () => ((typed.value = ''), (force.value = false)))
async function remove() {
  if (doomed.value && typed.value === doomed.value && (await server.deleteRepo(doomed.value, typed.value, force.value === true))) doomed.value = null
}
</script>

<template>
  <NeedAccess what="repositories" admin :accounts="false" @overview="$emit('overview')">
    <div class="pane">
      <div class="bar">
        <p>The projects stored on this server.</p>
        <Button variant="primary" @click="makeOpen = true"><FolderPlus :size="16" aria-hidden="true" /> New repository…</Button>
      </div>
      <div v-if="server.retainedNote" class="banner note" role="status" data-testid="retained">
        <TriangleAlert :size="18" aria-hidden="true" />
        <span>
          “{{ server.retainedNote.name }}” is removed from the server, but the data in the storage bucket{{ server.retainedNote.backend ? ` (${server.retainedNote.backend})` : '' }} is kept.
          Remove it there to free the space.
        </span>
        <Button size="sm" variant="ghost" @click="server.retainedNote = null">Dismiss</Button>
      </div>
      <ErrorState v-if="r.error" :code="r.error.code" :message="r.error.message" @retry="server.loadRepos()" />
      <SkeletonRows v-else-if="r.loading && !r.data.length" :count="4" />
      <EmptyState v-else-if="!r.data.length" title="No repositories yet" description="Create one, then push a project to it from any computer that can reach this server." />
      <table v-else class="table" data-testid="repos">
        <caption class="sr-only">Repositories</caption>
        <thead>
          <tr><th scope="col">Name</th><th scope="col">Size</th><th scope="col">Last change</th><th scope="col"><span class="sr-only">Actions</span></th></tr>
        </thead>
        <tbody>
          <tr v-for="repo in r.data" :key="repo.name">
            <th scope="row">{{ repo.name }}</th>
            <td>{{ formatBytes(repo.size_bytes) }}</td>
            <td>{{ repo.last_modified ? timeAgo(repo.last_modified * 1000) : 'Nothing pushed yet' }}</td>
            <td class="act"><Button size="sm" variant="ghost" :aria-label="`Delete ${repo.name}`" @click="doomed = repo.name">Delete…</Button></td>
          </tr>
        </tbody>
      </table>
    </div>
  </NeedAccess>

  <Dialog v-model:open="makeOpen" title="New repository" description="An empty project on this server. Push to it from a repository on your computer.">
    <form id="new-repo" class="form" autocomplete="off" @submit.prevent="make">
      <TextField v-model="name" label="Name" placeholder="hero-film" :error="nameError" />
      <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
    </form>
    <template #footer>
      <Button @click="makeOpen = false">Cancel</Button>
      <Button type="submit" form="new-repo" variant="primary" :loading="server.busy" :disabled="!name || !!nameError">Create repository</Button>
    </template>
  </Dialog>

  <Dialog v-model:open="delOpen" :title="`Delete ${doomed}?`" description="The server stops hosting it and people can no longer fetch or push to it. This cannot be undone. Files already in the storage bucket stay there until you remove them.">
    <form id="del-repo" class="form" autocomplete="off" @submit.prevent="remove">
      <TextField v-model="typed" :label="`Type ${doomed} to confirm`" />
      <Checkbox v-if="doomedSize > 0" v-model="force" :label="`Delete even though it holds ${formatBytes(doomedSize)} of versions`" />
      <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
    </form>
    <template #footer>
      <Button @click="delOpen = false">Cancel</Button>
      <Button type="submit" form="del-repo" variant="danger" :loading="server.busy" :disabled="typed !== doomed">Delete repository</Button>
    </template>
  </Dialog>
</template>

<style scoped src="./panel.css"></style>
<style scoped>
.err {
  margin: 0;
  color: var(--danger);
}
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
}
</style>
