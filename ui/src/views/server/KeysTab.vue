<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Copy, KeyRound } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TextField from '@/components/TextField.vue'
import { useServerStore } from '@/stores/server'
import NeedAccess from './NeedAccess.vue'

defineEmits<{ overview: [] }>()
const server = useServerStore()
const k = computed(() => server.keys)

watch(() => [server.url, server.access], () => (server.access === 'admin' || server.access === 'member') && void server.loadKeys(), { immediate: true })

const date = (s: number) => new Date(s * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })

// New key ------------------------------------------------------------------------
const makeOpen = ref(false)
const name = ref('')
// Limit what the key may do (`auth key create --permissions`); none ticked = all you can do.
const PERMS = [
  { value: 'repo:read', label: 'Look at repositories (repo:read)' },
  { value: 'repo:write', label: 'Change files (repo:write)' },
  { value: 'repo:admin', label: 'Manage repositories (repo:admin)' },
]
const perms = ref<Record<string, boolean | 'indeterminate'>>({})
async function make() {
  if (!name.value || server.busy) return
  const chosen = PERMS.filter((p) => perms.value[p.value] === true).map((p) => p.value)
  if (await server.createKey(name.value, chosen)) {
    makeOpen.value = false
    name.value = ''
    perms.value = {}
  }
}

// The key is shown once ----------------------------------------------------------------
const saved = ref<boolean | 'indeterminate'>(false)
const copied = ref(false)
const showKey = computed({
  get: () => server.newKey !== null,
  // Only "I saved it" closes it: no Escape, no click outside.
  set: () => {},
})
watch(showKey, (o) => o && ((saved.value = false), (copied.value = false)))
async function copy() {
  try {
    await navigator.clipboard.writeText(server.newKey?.key ?? '')
    copied.value = true
  } catch {
    copied.value = false
  }
}

// Revoke -----------------------------------------------------------------------------
// `revoking` outlives the dialog: closing it runs before the confirm handler reads it.
const revoking = ref<{ id: string; name: string } | null>(null)
const revokeOpen = ref(false)
function askRevoke(key: { id: string; name: string }) {
  revoking.value = key
  revokeOpen.value = true
}
</script>

<template>
  <NeedAccess what="API keys" @overview="$emit('overview')">
    <div class="pane">
      <div class="bar">
        <p>Keys let scripts and other tools sign in as you, without your password.</p>
        <Button variant="primary" @click="makeOpen = true"><KeyRound :size="16" aria-hidden="true" /> New key…</Button>
      </div>
      <ErrorState v-if="k.error" :code="k.error.code" :message="k.error.message" @retry="server.loadKeys()" />
      <SkeletonRows v-else-if="k.loading && !k.data.length" :count="3" />
      <EmptyState v-else-if="!k.data.length" title="You have no API keys" description="Make one when a tool or a build machine needs to reach this server." />
      <table v-else class="table" data-testid="keys">
        <caption class="sr-only">Your API keys</caption>
        <thead>
          <tr><th scope="col">Name</th><th scope="col">Made on</th><th scope="col"><span class="sr-only">Actions</span></th></tr>
        </thead>
        <tbody>
          <tr v-for="key in k.data" :key="key.id">
            <th scope="row">{{ key.name }}</th>
            <td>{{ date(key.created_at) }}</td>
            <td class="act"><Button size="sm" variant="ghost" :aria-label="`Revoke key ${key.name}`" @click="askRevoke(key)">Revoke…</Button></td>
          </tr>
        </tbody>
      </table>
    </div>
  </NeedAccess>

  <Dialog v-model:open="makeOpen" title="New API key" description="Name it after what will use it, like “build machine”. It can do everything you can.">
    <form id="new-key" class="form" autocomplete="off" @submit.prevent="make">
      <TextField v-model="name" label="Name" placeholder="build machine" />
      <MoreOptions label="Limit what it can do">
        <p class="muted">Tick what the key may do. With nothing ticked it can do everything you can.</p>
        <Checkbox v-for="p in PERMS" :key="p.value" v-model="perms[p.value]" :label="p.label" />
      </MoreOptions>
      <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
    </form>
    <template #footer>
      <Button @click="makeOpen = false">Cancel</Button>
      <Button type="submit" form="new-key" variant="primary" :loading="server.busy" :disabled="!name">Make key</Button>
    </template>
  </Dialog>

  <Dialog v-model:open="showKey" :dismissable="false" title="Copy your new key" description="This is the only time it is shown. MediaGit does not keep it. Store it somewhere safe, like a password manager.">
    <div class="secret" data-testid="new-key-value"><code class="mono">{{ server.newKey?.key }}</code></div>
    <div class="row"><Button @click="copy"><Copy :size="16" aria-hidden="true" /> {{ copied ? 'Copied' : 'Copy key' }}</Button></div>
    <Checkbox v-model="saved" label="I saved it" />
    <template #footer>
      <Button variant="primary" :disabled="saved !== true" @click="server.dismissKey()">Done</Button>
    </template>
  </Dialog>

  <AlertDialog
    v-model:open="revokeOpen"
    :title="`Revoke key ${revoking?.name}?`"
    description="Anything using this key stops working at once. This cannot be undone, but you can make a new key."
    confirm-label="Revoke key"
    destructive
    @confirm="revoking && server.revokeKey(revoking.id, revoking.name)"
  />
</template>

<style scoped src="./panel.css"></style>
<style scoped>
.row {
  display: flex;
  margin: var(--space-3) 0;
}
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
