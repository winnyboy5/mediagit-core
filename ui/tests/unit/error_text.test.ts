import { describe, expect, it } from 'vitest'
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
    for (const bad of ['CON', 'con.txt', 'film.', 'film ', 'a/b', '.hidden', 'COM3', 'lpt9.x']) expect(repoNameError(bad), bad).toBeTruthy()
    for (const ok of ['hero-film', 'a.b', 'Project_2', 'COM10', 'console']) expect(repoNameError(ok), ok).toBeUndefined()
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
