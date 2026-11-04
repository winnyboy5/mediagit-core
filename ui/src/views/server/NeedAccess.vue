<script setup lang="ts">
import { computed, ref } from 'vue'
import { LogIn, ShieldAlert } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { useServerStore } from '@/stores/server'
import { levelLabel } from './roles'
import SignInDialog from './SignInDialog.vue'

/**
 * Shows `what` only when the person may see it, and otherwise says why not and what to do:
 * server not running, not signed in, or not an admin. `accounts: false` is for things that
 * also work on a server without sign-in.
 */
const props = withDefaults(defineProps<{ what: string; admin?: boolean; accounts?: boolean }>(), { admin: false, accounts: true })
const emit = defineEmits<{ overview: [] }>()
const server = useServerStore()
const signIn = ref(false)

const state = computed(() => {
  if (!server.url) return 'down'
  if (server.meError) return 'error'
  const a = server.access
  if (a === 'unknown') return 'loading'
  if (a === 'no_accounts') return props.accounts ? 'open' : 'ok'
  if (a === 'signed_out') return 'signed_out'
  if (props.admin && a !== 'admin') return 'member'
  return 'ok'
})
</script>

<template>
  <div v-if="state === 'ok'"><slot /></div>
  <div v-else class="pane" :data-testid="`gate-${state}`">
    <SkeletonRows v-if="state === 'loading'" :count="4" />
    <ErrorState v-else-if="state === 'error'" :code="server.meError!.code" :message="server.meError!.message" @retry="server.refreshMe()" />
    <EmptyState v-else-if="state === 'down'" title="The local server is not running" :description="`Start it on the Overview tab to manage ${what}.`">
      <Button variant="primary" @click="emit('overview')">Go to Overview</Button>
    </EmptyState>
    <EmptyState v-else-if="state === 'open'" title="This server does not use accounts" :description="`Anyone who can reach it can use it, so there are no ${what} to manage.`" />
    <EmptyState v-else-if="state === 'signed_out'" :title="`Sign in to manage ${what}`" :description="`You are not signed in to ${server.target.label}.`">
      <Button variant="primary" @click="signIn = true"><LogIn :size="16" aria-hidden="true" /> Sign in</Button>
    </EmptyState>
    <EmptyState
      v-else
      title="You need admin rights"
      :description="`You are signed in as ${server.me?.identity?.username}. Your role: ${levelLabel(server.me?.identity?.role ?? '')}. Ask an admin of this server to manage ${what}.`"
    >
      <template #icon><ShieldAlert :size="28" aria-hidden="true" /></template>
    </EmptyState>
    <SignInDialog v-model:open="signIn" />
  </div>
</template>

<style scoped src="./panel.css"></style>
