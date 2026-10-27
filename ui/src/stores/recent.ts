import { defineStore } from 'pinia'
import { ref } from 'vue'
import { baseName, load, save } from '@/lib/persist'

export interface RecentRepo {
  path: string
  name: string
  openedAt: number
  branch?: string | null
}

const KEY = 'mg.recent'
const MAX = 8

/** Recently opened repositories, newest first, kept in localStorage. */
export const useRecentStore = defineStore('recent', () => {
  const list = ref<RecentRepo[]>(load<RecentRepo[]>(KEY, []).filter((r) => r && typeof r.path === 'string'))

  function add(path: string, branch?: string | null) {
    const next = [{ path, name: baseName(path), openedAt: Date.now(), branch }, ...list.value.filter((r) => r.path !== path)]
    list.value = next.slice(0, MAX)
    save(KEY, list.value)
  }

  function remove(path: string) {
    list.value = list.value.filter((r) => r.path !== path)
    save(KEY, list.value)
  }

  return { list, add, remove }
})
