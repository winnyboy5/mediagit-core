import { readonly, shallowRef } from 'vue'

export type ToastKind = 'success' | 'error' | 'info'
export interface ToastItem {
  id: number
  kind: ToastKind
  title: string
  description?: string
}

const items = shallowRef<ToastItem[]>([])
let next = 1

export function useToast() {
  function push(t: Omit<ToastItem, 'id' | 'kind'> & { kind?: ToastKind }): number {
    const id = next++
    items.value = [...items.value, { kind: 'info', ...t, id }]
    return id
  }
  function dismiss(id: number) {
    items.value = items.value.filter((t) => t.id !== id)
  }
  return { items: readonly(items), push, dismiss }
}
