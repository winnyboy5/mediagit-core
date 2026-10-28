<script setup lang="ts">
// Asks for the repository's encryption passphrase when a command needs it. Mounted once in the shell.
import { computed, ref, watch } from 'vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import TextField from '@/components/TextField.vue'
import { useUnlockStore } from '@/stores/unlock'

const unlock = useUnlockStore()
const pass = ref('')

const open = computed({
  get: () => unlock.open,
  set: (o: boolean) => {
    if (!o && unlock.open) {
      pass.value = ''
      unlock.answer(null)
    }
  },
})
// Never keep the text around once the dialog is gone.
watch(
  () => unlock.open,
  (o) => !o && (pass.value = ''),
)

function submit() {
  if (!pass.value) return
  const p = pass.value
  pass.value = ''
  unlock.answer(p)
}
</script>

<template>
  <Dialog v-model:open="open" title="Unlock this repository" description="Its files are encrypted. Enter the passphrase to continue. MediaGit keeps it in memory, and only in memory, until you lock the repository or close the app.">
    <form id="unlock-form" class="form" autocomplete="off" @submit.prevent="submit">
      <TextField v-model="pass" label="Passphrase" type="password" :error="unlock.retry ? 'That passphrase did not unlock the repository.' : undefined" />
    </form>
    <template #footer>
      <Button @click="open = false">Cancel</Button>
      <Button type="submit" form="unlock-form" variant="primary" :disabled="!pass">Unlock</Button>
    </template>
  </Dialog>
</template>

<style scoped>
.form {
  display: grid;
  gap: var(--space-3);
}
</style>
