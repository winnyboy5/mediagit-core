<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { UserPlus } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TextField from '@/components/TextField.vue'
import { useServerStore } from '@/stores/server'
import NeedAccess from './NeedAccess.vue'
import { LEVELS, levelLabel } from './roles'

defineEmits<{ overview: [] }>()
const server = useServerStore()
const u = computed(() => server.users)

watch(() => [server.url, server.access], () => server.access === 'admin' && void server.loadUsers(), { immediate: true })

// Add -------------------------------------------------------------------------
const addOpen = ref(false)
const name = ref('')
const email = ref('')
const role = ref('write')
const pass = ref('')
const canAdd = computed(() => !!name.value && !!email.value && !!pass.value && !server.busy)
async function add() {
  if (!canAdd.value) return
  const pw = pass.value
  pass.value = ''
  if (await server.createUser(name.value, email.value, role.value, pw)) {
    addOpen.value = false
    name.value = email.value = ''
  }
}
watch(addOpen, (o) => !o && (pass.value = ''))

// Change role --------------------------------------------------------------------
const roleFor = ref<string | null>(null)
const newRole = ref('write')
const roleOpen = computed({ get: () => roleFor.value !== null, set: (v) => !v && (roleFor.value = null) })
function openRole(user: string, current: string) {
  roleFor.value = user
  newRole.value = current
}
async function saveRole() {
  if (roleFor.value && (await server.setRole(roleFor.value, newRole.value))) roleFor.value = null
}

// Disable / enable -------------------------------------------------------------------
// `disabling` outlives the dialog: closing it runs before the confirm handler reads it.
const disabling = ref<string | null>(null)
const disableOpen = ref(false)
function askDisable(user: string) {
  disabling.value = user
  disableOpen.value = true
}

// Reset password ---------------------------------------------------------------------
const resetFor = ref<string | null>(null)
const resetPass = ref('')
const resetOpen = computed({ get: () => resetFor.value !== null, set: (v) => !v && (resetFor.value = null) })
async function saveReset() {
  if (!resetFor.value || !resetPass.value) return
  const pw = resetPass.value
  resetPass.value = ''
  if (await server.resetPassword(resetFor.value, pw)) resetFor.value = null
}
watch(resetOpen, (o) => !o && (resetPass.value = ''))
</script>

<template>
  <NeedAccess what="people" admin @overview="$emit('overview')">
    <div class="pane">
      <div class="bar">
        <p>Everyone with an account on this server.</p>
        <Button variant="primary" @click="addOpen = true"><UserPlus :size="16" aria-hidden="true" /> Add person…</Button>
      </div>
      <ErrorState v-if="u.error" :code="u.error.code" :message="u.error.message" @retry="server.loadUsers()" />
      <SkeletonRows v-else-if="u.loading && !u.data.length" :count="4" />
      <EmptyState v-else-if="!u.data.length" title="No one has an account yet" description="Add the first person to give them access." />
      <table v-else class="table" data-testid="people">
        <caption class="sr-only">People</caption>
        <thead>
          <tr><th scope="col">Person</th><th scope="col">What they can do</th><th scope="col"><span class="sr-only">Actions</span></th></tr>
        </thead>
        <tbody>
          <tr v-for="p in u.data" :key="p.id">
            <th scope="row">{{ p.username }}</th>
            <td>
              <span class="chip">{{ levelLabel(p.role) }}</span>
              <span v-if="p.disabled" class="chip off">Cannot sign in</span>
            </td>
            <td class="act">
              <Button size="sm" variant="ghost" :aria-label="`Change what ${p.username} can do`" @click="openRole(p.username, p.role)">Change role…</Button>
              <Button v-if="p.disabled" size="sm" variant="ghost" :aria-label="`Let ${p.username} sign in again`" @click="server.enableUser(p.username)">Enable</Button>
              <Button v-else size="sm" variant="ghost" :aria-label="`Stop ${p.username} signing in`" @click="askDisable(p.username)">Disable…</Button>
              <Button size="sm" variant="ghost" :aria-label="`Reset password for ${p.username}`" @click="resetFor = p.username">Reset password…</Button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </NeedAccess>

  <AlertDialog
    v-model:open="disableOpen"
    :title="`Stop ${disabling} signing in?`"
    description="They are signed out and cannot sign in until you enable them again. Their account and access settings are kept."
    confirm-label="Disable"
    destructive
    @confirm="disabling && server.disableUser(disabling)"
  />

  <Dialog v-model:open="addOpen" title="Add person" description="They sign in with this username and password. Ask them to change the password afterwards.">
    <form id="add-person" class="form" autocomplete="off" @submit.prevent="add">
      <TextField v-model="name" label="Username" />
      <TextField v-model="email" label="Email" type="email" />
      <Select v-model="role" label="What they can do" :options="LEVELS" />
      <TextField v-model="pass" label="Starting password" type="password" />
      <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
    </form>
    <template #footer>
      <Button @click="addOpen = false">Cancel</Button>
      <Button type="submit" form="add-person" variant="primary" :loading="server.busy" :disabled="!canAdd">Add person</Button>
    </template>
  </Dialog>

  <Dialog v-model:open="roleOpen" :title="`Change what ${roleFor} can do`" description="This applies on the whole server. Access to a single repository is set on the Access tab.">
    <form id="set-role" class="form" @submit.prevent="saveRole">
      <Select v-model="newRole" label="What they can do" :options="LEVELS" />
      <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
    </form>
    <template #footer>
      <Button @click="roleOpen = false">Cancel</Button>
      <Button type="submit" form="set-role" variant="primary" :loading="server.busy">Save</Button>
    </template>
  </Dialog>

  <Dialog v-model:open="resetOpen" :title="`Reset password for ${resetFor}`" description="Their old password stops working at once. Tell them the new one in person or through a secure channel.">
    <form id="reset-pw" class="form" autocomplete="off" @submit.prevent="saveReset">
      <TextField v-model="resetPass" label="New password" type="password" />
      <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
    </form>
    <template #footer>
      <Button @click="resetOpen = false">Cancel</Button>
      <Button type="submit" form="reset-pw" variant="primary" :loading="server.busy" :disabled="!resetPass">Reset password</Button>
    </template>
  </Dialog>
</template>

<style scoped src="./panel.css"></style>
<style scoped>
.chip.off {
  margin-left: var(--space-2);
  background: var(--danger-soft);
  color: var(--danger);
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
