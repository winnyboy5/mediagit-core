import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { api } from '@/api'
import { useRepoStore } from './repo'

/** Who "me" is: the signed-in account and the commit author, either of which may own a lock. */
export const useIdentityStore = defineStore('identity', () => {
  const repo = useRepoStore()
  const username = ref<string | null>(null)
  const authorName = ref<string | null>(null)
  const authorEmail = ref<string | null>(null)
  const loaded = ref(false)

  const names = computed(() => [username.value, authorName.value].filter((n): n is string => !!n))
  let token = 0

  /** Best effort: no server, no sign-in or no author set simply leaves the field empty. */
  async function refresh() {
    const root = repo.path
    if (!root) return
    const mine = ++token
    const [auth, cfg] = await Promise.allSettled([api.authRun(root, 'whoami', null), api.configList(root)])
    if (mine !== token) return
    username.value = auth.status === 'fulfilled' ? (auth.value?.identity?.username ?? null) : null
    const get = (k: string) => (cfg.status === 'fulfilled' ? (cfg.value?.entries?.find((e) => e.key === k)?.value ?? null) : null)
    authorName.value = get('author.name')
    authorEmail.value = get('author.email')
    loaded.value = true
  }

  watch(
    () => repo.path,
    () => {
      token++
      username.value = authorName.value = authorEmail.value = null
      loaded.value = false
    },
  )

  return { username, authorName, authorEmail, names, loaded, refresh }
})
