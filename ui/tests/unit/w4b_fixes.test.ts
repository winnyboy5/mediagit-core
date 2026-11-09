import { afterEach, describe, expect, it } from 'vitest'
import { enableAutoUnmount, mount } from '@vue/test-utils'
import SettingRow from '@/views/settings/SettingRow.vue'
import { SETTING_COPY } from '@/lib/settingCopy'

enableAutoUnmount(afterEach)

const spec = { key: 'performance.upload_concurrency', type: 'int', default: '32', doc: 'Unset: MEDIAGIT_UPLOAD_CONCURRENCY', enum_values: [], secret: false, min: null, max: null }

describe('setting row', () => {
  const props = { spec, fullKey: spec.key, label: 'Files sent at once', value: null, busy: false, description: SETTING_COPY[spec.key]!.description, env: SETTING_COPY[spec.key]!.env }

  it('shows the plain description and keeps the env var on the muted key line', () => {
    const w = mount(SettingRow, { props })
    expect(w.text()).toContain('How many pieces are sent in parallel')
    expect(w.get('.note, .field .note').text()).not.toContain('MEDIAGIT_')
    expect(w.get('.key').text()).toContain('MEDIAGIT_UPLOAD_CONCURRENCY')
  })

  it('a changed row says Not saved and reports it dirty; saved again it clears', async () => {
    const w = mount(SettingRow, { props })
    expect(w.text()).not.toContain('Not saved')
    await w.get('input').setValue('8')
    expect(w.text()).toContain('Not saved')
    expect(w.emitted('dirty')!.at(-1)).toEqual([spec.key, true])
    await w.setProps({ value: '8' })
    expect(w.text()).not.toContain('Not saved')
    expect(w.emitted('dirty')!.at(-1)).toEqual([spec.key, false])
  })
})
