import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { routerKey } from 'vue-router'
import ErrorState from '@/components/ErrorState.vue'
import { passwordError, plainError, repoNameError, storageMissing, usernameError } from '@/lib/errorText'
import { toApiError } from '@/api'

// Every code the CLI or the desktop engine can send (task-6B1-codes.md) plus the app's own.
const CODES = [
  'server_unreachable', 'server_timeout', 'not_signed_in', 'no_access', 'remote_not_found', 'conflict',
  'request_rejected', 'server_error', 'request_failed', 'permission_denied', 'file_in_use', 'already_exists',
  'disk_full', 'needs_input', 'invalid_args', 'keychain_unavailable', 'empty_commit', 'json_unsupported',
  'internal', 'error', 'cli_not_found', 'spawn_failed', 'protocol', 'contract_mismatch', 'invalid_argument',
  'cancelled', 'input_required', 'setup_failed', 'wrong_passphrase', 'unknown', 'E_SOMETHING_NEW',
]
const RAW = 'POST /admin/repos failed with status: 400 Bad Request'
const FORBIDDEN = [/\bempty_commit\b/, /\bE_[A-Z_]+\b/, /failed with status/, /\b[1-5]\d\d (Bad|Not|Internal|Conflict|Forbidden|Unauthorized)/, /^error$/i]

describe('plainError', () => {
  it.each(CODES)('%s: plain words, no code, no status line', (code) => {
    for (const message of ['', RAW, 'Cannot reach the server at host:9000. Check the address, then try again.']) {
      const v = plainError(code, message)
      expect(v.text.length, code).toBeGreaterThan(5)
      if (code.includes('_')) expect(v.text.toLowerCase()).not.toContain(code.toLowerCase())
      for (const bad of FORBIDDEN) expect(v.text, `${code} / ${message}`).not.toMatch(bad)
      expect(v.text).toMatch(/[.!?]$/)
    }
  })

  it('shows a short plain message of an unmapped code, never the code', () => {
    const v = plainError('E_LOCKED', 'Another save is running.')
    expect(v.text).toBe('Another save is running.')
    expect(v.details).toBe('')
  })

  it('an unmapped code with a raw status line gets a generic sentence and keeps the raw text under details', () => {
    const v = plainError('error', RAW)
    expect(v.text).not.toContain('status')
    expect(v.details).toBe(RAW)
  })

  it('a 401 with a server reason shows the reason and the sign-in action, not the CLI hint', () => {
    const v = plainError('not_signed_in', 'Invalid credentials. Sign in and try again (mediagit auth login <server>).')
    expect(v.text).toBe('Invalid credentials. Sign in and try again.')
    expect(plainError('not_signed_in', 'Invalid credentials').text).toBe('Invalid credentials. Sign in and try again.')
    expect(plainError('not_signed_in', 'You are not signed in, or your sign-in expired. Sign in and try again (mediagit auth login <server>).').text).toBe(
      'You are not signed in, or your sign-in expired. Sign in and try again.',
    )
  })

  it('keeps the engine details under details', () => {
    const v = plainError('server_unreachable', 'Cannot reach the server at h:1. Check the address, then try again.', 'connect refused')
    expect(v.details).toBe('connect refused')
    expect(v.retry).toBe(true)
    expect(plainError('no_access', 'x').retry).toBe(false)
  })

  it('is idempotent through toApiError', () => {
    const once = toApiError({ code: 'error', message: RAW, details: 'chain' })
    expect(toApiError(once)).toEqual(once)
    expect(once.details).toContain(RAW)
    expect(once.details).toContain('chain')
  })
})

describe('form checks', () => {
  it('repository names follow the server rule', () => {
    for (const bad of ['CON', 'con.txt', 'film.', 'film ', 'a/b', '.hidden', 'COM3', 'lpt9.x', 'a..b', 'a b', 'a:b', 'a\b']) expect(repoNameError(bad), bad).toBeTruthy()
    // security.rs validate_repo_name: any Unicode letter or digit, and a leading dash or underscore, are fine.
    for (const ok of ['hero-film', 'a.b', 'Project_2', 'COM10', 'console', '-x', '_x', 'Filmé', 'フィルム', 'a-.b']) expect(repoNameError(ok), ok).toBeUndefined()
  })
  it('usernames count bytes like the server', () => {
    expect(usernameError('éa')).toBeUndefined() // 3 bytes
    expect(usernameError('ab ')).toBeTruthy() // the server trims first
    expect(passwordError('éééé')).toBeUndefined() // 8 bytes
    expect(passwordError('abcdefg')).toBeTruthy()
  })
  it('usernames and passwords', () => {
    expect(usernameError('ab')).toBeTruthy()
    expect(usernameError('abc')).toBeUndefined()
    expect(passwordError('abc')).toBeTruthy()
    expect(passwordError('abcdefgh')).toBeUndefined()
  })
  it('storage needs its required names', () => {
    expect(storageMissing({ kind: 's3' })).toBeTruthy()
    expect(storageMissing({ kind: 's3', bucket: ' ' })).toBeTruthy()
    expect(storageMissing({ kind: 's3', bucket: 'b' })).toBeUndefined()
    expect(storageMissing({ kind: 'filesystem' })).toBeUndefined()
    expect(storageMissing({ kind: 'azure' })).toBeTruthy()
  })
})

describe('ErrorState actions follow the code and the raw text', () => {
  const show = (code: string, message: string, details?: string) =>
    mount(ErrorState, { props: { code, message, details }, global: { provide: { [routerKey as symbol]: { push: () => {} } } } })
  const GENERIC = 'Something went wrong. Try again; if it keeps happening, open Details and send them to support.'
  it('a missing remote offers Add remote, not Retry', () => {
    const w = show('error', GENERIC, "No such remote: 'origin'")
    expect(w.text()).toContain('Add remote')
    expect(w.text()).not.toContain('Retry')
  })
  it('a folder that is not a repository offers no Retry', () => {
    const w = show('error', GENERIC, 'Not a mediagit repository (or any parent up to mount point)')
    expect(w.text()).not.toContain('Retry')
  })
  it('permission_denied keeps Retry, as the table says', () => {
    expect(show('permission_denied', 'Access to a file or folder was denied. Check its permissions, then try again.').text()).toContain('Retry')
  })
  it('would_overwrite explains the loss and offers no Retry', () => {
    const raw = 'Your local changes to the following files would be overwritten by merge: new.psd. Please commit your changes or stash them before you merge.'.replace(': ', ':\n')
    const v = plainError('would_overwrite', raw)
    expect(v.text).toMatch(/would be lost/)
    expect(v.retry).toBe(false)
    expect(v.details).toContain('new.psd')
    expect(show('would_overwrite', raw).text).not.toContain('Retry')
    expect(v.text).not.toMatch(/stash/i)
  })
  it('staged_changes asks to save or unstage, with no Retry', () => {
    const raw = 'These files have staged changes that are not committed:\n  wip.psd\nPlease commit them.'
    const v = plainError('staged_changes', raw)
    expect(v.text).toMatch(/unstage/)
    expect(v.retry).toBe(false)
    expect(v.details).toContain('wip.psd')
  })
})
