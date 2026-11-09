<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Select from '@/components/Select.vue'
import TextField from '@/components/TextField.vue'
import type { ConfigKeySchema } from '@/api'

/**
 * One setting from the CLI's schema, edited by its type. `fullKey` is the concrete key
 * (`remotes.origin.url`). A secret is never shown: it reads "set" or "not set", and a new
 * value is typed into a password field and sent privately.
 */
const props = defineProps<{ spec: ConfigKeySchema; fullKey: string; label: string; value: string | null; busy: boolean; description?: string; env?: string }>()
const emit = defineEmits<{ save: [key: string, value: string, secret: boolean]; reset: [key: string]; dirty: [key: string, dirty: boolean] }>()

const draft = ref('')
const flag = ref<boolean | 'indeterminate'>(false)
const isSet = computed(() => props.value !== null)
watch(
  () => [props.value, props.spec.secret] as const,
  ([v, secret]) => {
    draft.value = secret ? '' : (v ?? '')
    flag.value = (v ?? props.spec.default) === 'true'
  },
  { immediate: true },
)

const current = computed(() => (props.spec.type === 'bool' ? String(flag.value === true) : draft.value.trim()))
const error = computed(() => {
  const s = props.spec
  const v = draft.value.trim()
  if (!v || s.type !== 'int') return undefined
  if (!/^\d+$/.test(v)) return 'Use a whole number'
  const n = Number(v)
  if (s.min !== null && n < s.min) return `At least ${s.min}`
  if (s.max !== null && n > s.max) return `At most ${s.max}`
  return undefined
})
const dirty = computed(() => {
  if (props.spec.secret) return !!draft.value
  if (props.spec.type === 'bool') return flag.value !== ((props.value ?? props.spec.default) === 'true')
  return current.value !== (props.value ?? '')
})
const canSave = computed(() => dirty.value && !error.value && !props.busy && (props.spec.type === 'bool' || !!current.value))
watch(dirty, (d) => emit('dirty', props.fullKey, d), { immediate: true })
onBeforeUnmount(() => emit('dirty', props.fullKey, false))
const hint = computed(() => {
  const s = props.spec
  const d = s.default ? ` Default: ${s.default}.` : ''
  return `${props.description ?? s.doc}${d}`
})
const options = computed(() => props.spec.enum_values.map((v) => ({ value: v, label: v })))

function save() {
  if (!canSave.value) return
  const value = props.spec.secret ? draft.value : current.value
  // A secret is handed over and the field emptied at once.
  if (props.spec.secret) draft.value = ''
  emit('save', props.fullKey, value, props.spec.secret)
}
</script>

<template>
  <form class="setting" :data-key="fullKey" @submit.prevent="save">
    <div class="ctl">
      <Checkbox v-if="spec.type === 'bool'" v-model="flag" :label="label" :disabled="busy" />
      <Select v-else-if="spec.type === 'enum'" v-model="draft" :label="label" :options="options" :disabled="busy" />
      <TextField
        v-else
        v-model="draft"
        :label="label"
        :type="spec.secret ? 'password' : 'text'"
        :placeholder="spec.secret ? (isSet ? 'A value is set. Type to replace it' : 'Not set') : (spec.default ?? 'Not set')"
        :mono="spec.type === 'path'"
        :error="error"
        :hint="hint"
        :disabled="busy"
      />
      <p v-if="spec.type === 'bool' || spec.type === 'enum'" class="note">{{ hint }}</p>
      <p class="key mono">{{ fullKey }}<template v-if="env"> · env {{ env }}</template><template v-if="spec.secret"> · {{ isSet ? 'set' : 'not set' }}</template></p>
    </div>
    <div class="act">
      <span v-if="dirty" class="unsaved" role="status"><span class="dot" aria-hidden="true"></span>Not saved</span>
      <Button type="submit" size="sm" variant="primary" :disabled="!canSave">Save</Button>
      <Button size="sm" variant="ghost" :disabled="busy || !isSet" :aria-label="`Reset ${fullKey}`" @click="emit('reset', fullKey)">Reset</Button>
    </div>
  </form>
</template>

<style scoped>
.setting {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: var(--space-3);
  padding: var(--space-3) 0;
  border-bottom: 1px solid var(--border);
}
.setting:last-child {
  border-bottom: 0;
}
.ctl {
  display: grid;
  flex: 1 1 280px;
  gap: var(--space-1);
  min-width: 0;
  max-width: 560px;
}
.act {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}
.note,
.key {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
/* Status beside Save: icon + words, so it never relies on colour alone. */
.unsaved {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  margin-inline-end: var(--space-2);
  color: var(--modified);
  font-size: var(--fs-sm);
  font-weight: 500;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: currentColor;
}
.key {
  font-size: var(--fs-xs);
  overflow-wrap: anywhere;
}
</style>
