import { readonly, shallowRef } from 'vue'

export type ToastKind = 'success' | 'warning' | 'error' | 'info'
export interface ToastItem {
  id: number
  kind: ToastKind
  title: string
  description?: string
}

const MAX_VISIBLE = 3
const items =shallowRef<ToastItem[]>([])
let next = 1

export function useToast() {
  function push(t: Omit<ToastItem, 'id' | 'kind'> & { kind?: ToastKind }): number {
    const id = next++
    const next3 = [...items.value, { kind: 'info' as ToastKind, ...t, id }]
    // Cap the visible stack at 3: drop the oldest that is not an error first.
    while (next3.length > MAX_VISIBLE) next3.splice(Math.max(0, next3.findIndex((x) => x.kind !== 'error')), 1)
    items.value = next3
    return id
  }
  function dismiss(id: number) {
    items.value = items.value.filter((t) => t.id !== id)
  }
  return { items: readonly(items), push, dismiss }
}
