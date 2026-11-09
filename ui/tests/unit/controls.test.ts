import { afterEach, describe, expect, it } from 'vitest'
import { mount, enableAutoUnmount } from '@vue/test-utils'
import { nextTick } from 'vue'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Dialog from '@/components/Dialog.vue'
import DropdownMenu from '@/components/DropdownMenu.vue'
import IconButton from '@/components/IconButton.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import Select from '@/components/Select.vue'
import Tabs from '@/components/Tabs.vue'
import TextField from '@/components/TextField.vue'
import Textarea from '@/components/Textarea.vue'
import Toast from '@/components/Toast.vue'
import { useToast } from '@/components/useToast'

enableAutoUnmount(afterEach)

describe('Button', () => {
  it('renders variants and size classes', () => {
    const w = mount(Button, { props: { variant: 'primary', size: 'sm' }, slots: { default: 'Save' } })
    expect(w.classes()).toEqual(expect.arrayContaining(['primary', 'sm']))
    expect(w.attributes('type')).toBe('button')
  })
  it('loading disables, sets aria-busy and shows spinner', async () => {
    const w = mount(Button, { props: { loading: true } })
    expect(w.attributes('disabled')).toBeDefined()
    expect(w.attributes('aria-busy')).toBe('true')
    expect(w.find('svg').exists()).toBe(true)
    await w.trigger('click')
    expect(w.emitted('click')).toBeUndefined()
  })
  it('click fires when enabled', async () => {
    const w = mount(Button)
    await w.trigger('click')
    expect(w.emitted('click')).toHaveLength(1)
  })
})

describe('IconButton', () => {
  it('uses label as accessible name and reflects pressed', () => {
    const w = mount(IconButton, { props: { label: 'Add file', pressed: true } })
    expect(w.attributes('aria-label')).toBe('Add file')
    expect(w.attributes('aria-pressed')).toBe('true')
  })
})

describe('Checkbox', () => {
  it('toggles on click and exposes checkbox role', async () => {
    const w = mount(Checkbox, { props: { label: 'All', modelValue: false, 'onUpdate:modelValue': (v: unknown) => w.setProps({ modelValue: v as boolean }) } })
    const box = w.get('[role=checkbox]')
    expect(box.attributes('aria-checked')).toBe('false')
    await box.trigger('click')
    expect(box.attributes('aria-checked')).toBe('true')
  })
  it('indeterminate reports mixed', () => {
    const w = mount(Checkbox, { props: { modelValue: 'indeterminate', ariaLabel: 'x' } })
    expect(w.get('[role=checkbox]').attributes('aria-checked')).toBe('mixed')
  })
})

describe('TextField / Textarea', () => {
  it('label is associated and v-model works', async () => {
    const w = mount(TextField, { props: { label: 'Name', modelValue: '' } })
    const input = w.get('input')
    expect(w.get('label').attributes('for')).toBe(input.attributes('id'))
    await input.setValue('abc')
    expect(w.emitted('update:modelValue')?.[0]).toEqual(['abc'])
  })
  it('error sets aria-invalid and describes the input', () => {
    const w = mount(TextField, { props: { label: 'URL', error: 'Bad URL', hint: 'hint' } })
    const input = w.get('input')
    expect(input.attributes('aria-invalid')).toBe('true')
    const note = w.get('.note')
    expect(note.text()).toBe('Bad URL')
    expect(input.attributes('aria-describedby')).toBe(note.attributes('id'))
  })
  it('flag renders as secondary code text and is described; an error replaces it', async () => {
    const w = mount(TextField, { props: { label: 'Saved by', hint: 'Only versions by this person.', flag: '--author' } })
    const note = w.get('.note')
    expect(note.text()).toContain('Only versions by this person.')
    expect(note.get('code.flag').text()).toBe('--author')
    expect(w.get('input').attributes('aria-describedby')).toBe(note.attributes('id'))
    await w.setProps({ error: 'Bad' })
    expect(w.find('code.flag').exists()).toBe(false)
  })
  it('a disabled field carries the disabled attribute', () => {
    const w = mount(TextField, { props: { label: 'Tagger', disabled: true, hint: 'Add a note first to set these.' } })
    expect(w.get('input').attributes('disabled')).toBeDefined()
  })
  it('Textarea wires label and error', () => {
    const w = mount(Textarea, { props: { label: 'Message', error: 'Required', rows: 2 } })
    const ta = w.get('textarea')
    expect(w.get('label').attributes('for')).toBe(ta.attributes('id'))
    expect(ta.attributes('aria-invalid')).toBe('true')
    expect(ta.attributes('rows')).toBe('2')
  })
})

describe('Select', () => {
  it('renders a labelled combobox trigger', () => {
    const w = mount(Select, {
      props: { label: 'Branch', modelValue: 'main', options: [{ value: 'main', label: 'main' }, { value: 'dev', label: 'dev' }] },
      attachTo: document.body,
    })
    const trigger = w.get('[role=combobox]')
    expect(trigger.attributes('id')).toBe(w.get('label').attributes('for'))
    expect(trigger.attributes('aria-expanded')).toBe('false')
  })
})

describe('SegmentedControl', () => {
  const options = [{ value: 'tree', label: 'Tree' }, { value: 'flat', label: 'Flat' }]
  it('selects an option and never ends up empty', async () => {
    const w = mount(SegmentedControl, {
      props: { options, label: 'Layout', modelValue: 'tree', 'onUpdate:modelValue': (v: unknown) => w.setProps({ modelValue: v as string }) },
    })
    const [tree, flat] = w.findAll('button')
    expect(tree!.attributes('data-state')).toBe('on')
    await flat!.trigger('click')
    expect(flat!.attributes('data-state')).toBe('on')
    await flat!.trigger('click')
    expect(flat!.attributes('data-state')).toBe('on')
    expect(w.get('[role=group]').attributes('aria-label')).toBe('Layout')
  })
})

describe('Tabs', () => {
  it('has tablist, selected tab and selection', async () => {
    const w = mount(Tabs, {
      props: { label: 'Sections', modelValue: 'a', items: [{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }], 'onUpdate:modelValue': (v: unknown) => w.setProps({ modelValue: v as string }) },
      slots: { default: '<template #default="{ value }">panel {{ value }}</template>' },
      attachTo: document.body,
    })
    expect(w.get('[role=tablist]').attributes('aria-label')).toBe('Sections')
    const tabs = w.findAll('[role=tab]')
    expect(tabs[0]!.attributes('aria-selected')).toBe('true')
    await tabs[1]!.trigger('mousedown', { button: 0 })
    await nextTick()
    expect(w.findAll('[role=tab]')[1]!.attributes('aria-selected')).toBe('true')
  })
})

describe('Dialog', () => {
  it('renders title, description, close button; Escape closes', async () => {
    const w = mount(Dialog, {
      props: { open: true, title: 'Clone', description: 'Copy a repo', 'onUpdate:open': (v: unknown) => w.setProps({ open: v as boolean }) },
      slots: { default: '<p>body</p>', footer: '<button>OK</button>' },
      attachTo: document.body,
    })
    await nextTick()
    const dlg = document.body.querySelector('[role=dialog]')!
    expect(dlg).toBeTruthy()
    expect(dlg.textContent).toContain('Clone')
    expect(dlg.querySelector('[aria-label=Close]')).toBeTruthy()
    expect(dlg.getAttribute('aria-labelledby')).toBeTruthy()
    dlg.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()
    expect(w.emitted('update:open')?.at(-1)).toEqual([false])
  })
})

describe('AlertDialog', () => {
  it('has alertdialog role and emits confirm', async () => {
    const w = mount(AlertDialog, {
      props: { open: true, title: 'Force unlock?', description: 'Consequences', confirmLabel: 'Force unlock', destructive: true },
      attachTo: document.body,
    })
    await nextTick()
    const dlg = document.body.querySelector('[role=alertdialog]')!
    expect(dlg).toBeTruthy()
    expect(dlg.textContent).toContain('Consequences')
    const buttons = [...dlg.querySelectorAll('button')]
    expect(buttons.map((b) => b.textContent?.trim())).toEqual(['Cancel', 'Force unlock'])
    buttons[1]!.click()
    expect(w.emitted('confirm')).toHaveLength(1)
  })
})

describe('DropdownMenu', () => {
  it('opens via keyboard and emits the chosen id', async () => {
    const w = mount(DropdownMenu, {
      props: { items: [{ id: 'stage', label: 'Stage' }, { id: 'rm', label: 'Discard', danger: true, separated: true }] },
      slots: { default: '<button>menu</button>' },
      attachTo: document.body,
    })
    const trigger = w.get('button')
    expect(trigger.attributes('aria-haspopup')).toBe('menu')
    await trigger.trigger('keydown', { key: 'ArrowDown' })
    await nextTick()
    const items = document.body.querySelectorAll('[role=menuitem]')
    expect(items).toHaveLength(2)
    ;(items[0] as HTMLElement).click()
    await nextTick()
    expect(w.emitted('select')?.[0]).toEqual(['stage'])
  })
})

describe('Toast', () => {
  it('renders pushed toasts in a labelled region and dismisses', async () => {
    const t = useToast()
    mount(Toast, { attachTo: document.body })
    const id = t.push({ kind: 'error', title: 'Push failed', description: 'Try again' })
    await nextTick()
    await nextTick()
    expect(document.body.textContent).toContain('Push failed')
    expect(document.body.querySelector('[aria-label^=Notifications]')).toBeTruthy()
    t.dismiss(id)
    await nextTick()
    expect(t.items.value.find((x) => x.id === id)).toBeUndefined()
  })
})
