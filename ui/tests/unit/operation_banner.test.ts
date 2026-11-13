import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { stoppedOperation, useChangesStore } from '@/stores/changes'
import { useRepoStore } from '@/stores/repo'
import type { StatusReport } from '@/api'
import conflict from '../fixtures/status_conflict.json'
import clean from '../fixtures/status_empty.json'

beforeEach(() => setActivePinia(createPinia()))

describe('stoppedOperation (BUG-021: the app now sees a stopped operation)', () => {
  it('is null when nothing is stopped, or status is missing or from an older program', () => {
    expect(stoppedOperation(null)).toBeNull()
    expect(stoppedOperation(clean as StatusReport)).toBeNull()
    expect(stoppedOperation({ ...(clean as StatusReport), operation: undefined } as unknown as StatusReport)).toBeNull()
  })

  it('names the operation and counts the files that need a decision', () => {
    expect(stoppedOperation(conflict as StatusReport)).toEqual({ kind: 'merge', title: 'A merge is in progress', detail: '2 files need a decision.' })
  })

  it('uses plain words per kind, singular for one file, and says when all are decided', () => {
    const base = conflict as StatusReport
    const of = (kind: 'rebase' | 'cherry_pick' | 'revert', unresolved: string[]) => stoppedOperation({ ...base, operation: { kind, ours: null, theirs: null }, unresolved })!
    expect(of('revert', ['a']).title).toBe('An undo is in progress')
    expect(of('revert', ['a']).detail).toBe('1 file needs a decision.')
    expect(of('cherry_pick', []).title).toBe('A cherry-pick is in progress')
    expect(of('rebase', []).detail).toBe('Every file is decided.')
  })

  it('reaches the Changes store from the repo status', () => {
    const repo = useRepoStore()
    const changes = useChangesStore()
    expect(changes.stopped).toBeNull()
    repo.status = conflict as StatusReport
    expect(changes.stopped?.kind).toBe('merge')
    repo.status = clean as StatusReport
    expect(changes.stopped).toBeNull()
  })
})
