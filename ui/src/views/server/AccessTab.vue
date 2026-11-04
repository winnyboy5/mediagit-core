<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import EmptyState from '@/components/EmptyState.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { useServerStore } from '@/stores/server'
import NeedAccess from './NeedAccess.vue'
import { LEVELS, levelLabel } from './roles'

defineEmits<{ overview: [] }>()
const server = useServerStore()

watch(
  () => [server.url, server.access],
  () => {
    if (server.access !== 'admin') return
    void server.loadUsers()
    void server.loadRepos()
  },
  { immediate: true },
)

const user = ref<string>()
const repoName = ref<string>()
const level = ref('write')
const people = computed(() => server.users.data.map((p) => ({ value: p.username, label: `${p.username}: ${levelLabel(p.role)}` })))
const repoOptions = computed(() => server.repos.data.map((r) => ({ value: r.name, label: r.name })))
const ready = computed(() => !!user.value && !!repoName.value && !server.busy)
const loading = computed(() => (server.users.loading || server.repos.loading) && !people.value.length)
const mine = computed(() => server.me?.identity?.grants ?? [])

watch(user, (u) => u && void server.loadUserGrants(u))
watch(repoName, (r) => r && void server.loadRepoAccess(r))
const userRows = computed(() => server.userGrants)
const repoRows = computed(() => server.repoAccess)

// What the person has on this repository right now, so "Remove access" can be checked first.
const current = computed(() => {
  const g = server.userGrants.data.find((x) => x.repo === repoName.value)
  return g ? `${user.value} now: ${levelLabel(g.level)} on ${repoName.value}.` : `${user.value} has no access of their own to ${repoName.value} yet.`
})

const removeOpen = ref(false)
async function give() {
  if (ready.value) await server.grant(user.value!, repoName.value!, level.value)
}
async function remove() {
  if (user.value && repoName.value) await server.revokeGrant(user.value, repoName.value)
}
</script>

<template>
  <NeedAccess what="access" admin @overview="$emit('overview')">
    <div class="pane">
      <p class="muted">Decide who can use each repository. A person's role on the People tab applies everywhere; this narrows or widens it for one repository.</p>
      <SkeletonRows v-if="loading" :count="3" />
      <EmptyState v-else-if="!repoOptions.length" title="No repositories yet" description="Create a repository on the Repositories tab, then give people access to it here." />
      <form v-else class="card form" aria-label="Repository access" data-testid="grant-form" @submit.prevent="give">
        <div class="selects">
          <Select v-model="user" label="Person" :options="people" placeholder="Choose a person" />
          <Select v-model="repoName" label="Repository" :options="repoOptions" placeholder="Choose a repository" />
          <Select v-model="level" label="What they can do here" :options="LEVELS" />
        </div>
        <p v-if="user && repoName" class="muted" data-testid="current-level">{{ current }}</p>
        <div class="btns end">
          <Button type="submit" variant="primary" :loading="server.busy" :disabled="!ready">Give access</Button>
          <Button variant="danger" :disabled="!ready" @click="removeOpen = true">Remove access…</Button>
        </div>
        <p v-if="server.actionError" class="err" role="alert">{{ server.actionError.message }}</p>
      </form>

      <section v-if="user" class="card" aria-labelledby="ug-h" data-testid="user-grants">
        <h2 id="ug-h">What {{ user }} can reach</h2>
        <p v-if="userRows.error" class="err" role="alert">{{ userRows.error.message }}</p>
        <p v-else-if="!userRows.data.length" class="muted">Nothing specific. Their role on the People tab applies.</p>
        <ul v-else class="list" aria-label="Access by repository">
          <li v-for="g in userRows.data" :key="g.repo"><span class="mono">{{ g.repo }}</span> <span class="chip">{{ levelLabel(g.level) }}</span></li>
        </ul>
      </section>

      <section v-if="repoName" class="card" aria-labelledby="ra-h" data-testid="repo-access">
        <h2 id="ra-h">Who can reach {{ repoName }}</h2>
        <p v-if="repoRows.error" class="err" role="alert">{{ repoRows.error.message }}</p>
        <p v-else-if="!repoRows.data.length" class="muted">No one has been given access to this repository specifically.</p>
        <ul v-else class="list" aria-label="Access by person">
          <li v-for="a in repoRows.data" :key="a.user_id"><span>{{ a.username }}</span> <span class="chip">{{ levelLabel(a.level) }}</span></li>
        </ul>
      </section>

      <section class="card" aria-labelledby="mine-h">
        <h2 id="mine-h">Your own access</h2>
        <p v-if="!mine.length" class="muted">Your role covers every repository on this server.</p>
        <ul v-else class="list" aria-label="Your access">
          <li v-for="g in mine" :key="g.repo"><span class="mono">{{ g.repo }}</span> <span class="chip">{{ levelLabel(g.level) }}</span></li>
        </ul>
      </section>
    </div>
  </NeedAccess>

  <AlertDialog
    v-model:open="removeOpen"
    :title="`Remove ${user}'s access to ${repoName}?`"
    description="They can no longer open this repository. Their copies on their own computers stay, and you can give access back any time."
    confirm-label="Remove access"
    destructive
    @confirm="remove"
  />
</template>

<style scoped src="./panel.css"></style>
<style scoped>
.selects {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--space-3);
  width: 100%;
}
.card.form {
  justify-items: stretch;
}
.btns.end {
  justify-content: flex-end;
}
.list {
  display: grid;
  gap: var(--space-1);
  margin: 0;
  padding: 0;
  list-style: none;
}
.small {
  font-size: var(--fs-sm);
}
.err {
  margin: 0;
  color: var(--danger);
}
</style>
