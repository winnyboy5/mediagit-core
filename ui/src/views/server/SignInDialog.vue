<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import TextField from '@/components/TextField.vue'
import { useServerStore } from '@/stores/server'

const open = defineModel<boolean>('open', { default: false })
const server = useServerStore()
const username = ref('')
const password = ref('')
const ok = computed(() => !!username.value && !!password.value && !server.busy)

async function submit() {
  if (!ok.value) return
  // The password is handed over and the field emptied before anything else happens.
  const pw = password.value
  password.value = ''
  if (await server.login(username.value, pw)) open.value = false
}
watch(open, (o) => !o && (password.value = ''))
</script>

<template>
  <Dialog v-model:open="open" title="Sign in" :description="`Sign in to ${server.target.label}.`">
    <form id="server-login" class="form" autocomplete="off" @submit.prevent="submit">
      <TextField v-model="username" label="Username or email" />
      <TextField v-model="password" label="Password" type="password" />
      <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
    </form>
    <template #footer>
      <Button @click="open = false">Cancel</Button>
      <Button type="submit" form="server-login" variant="primary" :disabled="!ok">Sign in</Button>
    </template>
  </Dialog>
</template>

<style scoped src="./panel.css"></style>
<style scoped>
.err {
  margin: 0;
  color: var(--danger);
}
</style>
