// One place that turns an engine error into words a person can act on.
// The code is for logic only; it is never shown. The raw text goes under "Details".

export interface ErrorView {
  /** Plain sentence(s): what happened and what to do next. */
  text: string
  /** Raw message and engine details, for the collapsed "Details". Empty when there is nothing more. */
  details: string
  /** Whether trying the same thing again can help. */
  retry: boolean
}

interface Entry {
  text: string
  retry: boolean
  /** The engine's own sentence is already plain and carries facts (host, path, reason): keep it. */
  own?: boolean
}

const GENERIC = 'Something went wrong. Try again; if it keeps happening, open Details and send them to support.'

const TABLE: Record<string, Entry> = {
  server_unreachable: { text: 'Cannot reach the server. Check the address and your network, then try again.', retry: true, own: true },
  server_timeout: { text: 'The server did not answer in time. Check your connection and try again.', retry: true, own: true },
  not_signed_in: { text: 'You are not signed in, or your sign-in expired. Sign in and try again.', retry: false, own: true },
  no_access: { text: 'This account cannot do that here. Ask a server admin to give it access.', retry: false, own: true },
  remote_not_found: { text: 'The server does not have that. Check the repository name and the server address.', retry: false, own: true },
  conflict: { text: 'That conflicts with what is already on the server. Refresh and try again.', retry: true, own: true },
  request_rejected: { text: 'The server refused the request. Check what you entered and try again.', retry: false, own: true },
  server_error: { text: 'The server hit a problem. Try again in a moment; if it keeps happening, tell the server admin.', retry: true, own: true },
  request_failed: { text: 'The server could not complete the request. Try again in a moment.', retry: true, own: true },
  permission_denied: { text: 'Access to a file or folder was denied. Check its permissions, then try again.', retry: true, own: true },
  file_in_use: { text: 'A file is in use by another program. Close that program, then try again.', retry: true, own: true },
  already_exists: { text: 'A file or folder with that name already exists. Choose another name or remove it first.', retry: false, own: true },
  disk_full: { text: 'The disk is full. Free some space, then try again.', retry: true, own: true },
  would_overwrite: { text: 'Some of your files would be lost. Save, undo or move them, then try again. Open Details to see which files.', retry: false },
  staged_changes: { text: 'Some changes are set aside for the next version but not saved yet. Save them as a version, or unstage them, then try again. Open Details to see which files.', retry: false },
  needs_input: { text: 'This repository is locked. Unlock it with its passphrase to continue.', retry: false },
  input_required: { text: 'This step needs an answer the app cannot ask for. Run it from the command line instead.', retry: false },
  wrong_passphrase: { text: 'That passphrase did not unlock the repository. Check it and try again.', retry: false, own: true },
  empty_commit: { text: 'Nothing would change: those changes are already here. Pick a different version.', retry: false },
  invalid_args: { text: 'Those options do not fit together. Change them and try again.', retry: false, own: true },
  invalid_argument: { text: 'The app refused something you entered. Check it and try again.', retry: false, own: true },
  keychain_unavailable: { text: 'This computer’s password store is not available. Unlock it, then try again.', retry: true },
  json_unsupported: { text: 'The app cannot run that command. Update MediaGit so the app and its core match.', retry: false },
  internal: { text: 'MediaGit hit an unexpected problem. Try again; if it keeps happening, send the details to support.', retry: true },
  cli_not_found: { text: 'MediaGit could not find its core files next to the app. Reinstall MediaGit to restore it.', retry: true },
  contract_mismatch: { text: 'This app and the MediaGit core were built for different versions. Reinstall MediaGit so they match.', retry: true },
  spawn_failed: { text: 'MediaGit could not start its core. Restart the app; if it keeps happening, reinstall MediaGit.', retry: true },
  protocol: { text: 'The app and the MediaGit core did not understand each other. Restart the app and try again.', retry: true },
  setup_failed: { text: 'Setup did not finish. Check what you entered and try again.', retry: true, own: true },
  cancelled: { text: 'Stopped.', retry: false },
  error: { text: GENERIC, retry: true, own: true },
}

/** Text that is a person-sized sentence, not a status line, an OS error or an error chain. */
function looksPlain(m: string): boolean {
  if (!m || m.length > 220 || /[\r\n]/.test(m)) return false
  if (/\b(status|HTTP\/?\d?)\b.{0,12}\b\d{3}\b|failed with status|^\s*\d{3}\b|os error|caused by|panicked|\.rs:\d+|\w::\w/i.test(m)) return false
  return true
}

// The CLI adds "(mediagit auth login <server>)" style hints; the app has its own buttons.
function tidy(m: string, period = false): string {
  let t = m.replace(/^invalid argument: /i, '').replace(/\s*\(mediagit [^)]*\)/g, '').trim()
  if (period && /[A-Za-z0-9)]$/.test(t)) t += '.'
  return t
}
// Server reasons arrive without a final full stop.
const REASONS = new Set(['conflict', 'request_rejected', 'not_signed_in'])

export function plainError(code: string, message: string, details?: string | null): ErrorView {
  const raw = (message ?? '').trim()
  const known = TABLE[code]
  let text: string
  let retry: boolean
  if (known) {
    retry = known.retry
    text = known.own && looksPlain(raw) ? tidy(raw, REASONS.has(code)) : known.text
    if (code === 'not_signed_in' && !/sign in/i.test(text)) text += ' Sign in and try again.'
  } else {
    // An unknown code: a short plain message is shown as it is, never the code.
    retry = true
    text = looksPlain(raw) ? tidy(raw) : GENERIC
  }
  const parts = [details?.trim(), raw].filter((p): p is string => !!p && p !== text)
  return { text, details: [...new Set(parts)].join('\n'), retry }
}

// Checks the form can make before asking the server, mirroring the server's own rules.

const RESERVED = /^(con|prn|aux|nul|(com|lpt)[1-9])$/i

/** The server's rule for a new repository name (repos_admin.rs and security.rs validate_repo_name). */
export function repoNameError(name: string): string | undefined {
  if (!name) return undefined
  if (name.includes('/')) return 'Names cannot contain a slash.'
  if (name.startsWith('.')) return 'Names cannot start with a dot.'
  if (/[. ]$/.test(name)) return 'Names cannot end in a dot or a space.'
  if (RESERVED.test(name.split('.')[0]!.trimEnd())) return 'That name is reserved by Windows. Choose another.'
  if (name.includes('..')) return 'Names cannot contain two dots in a row.'
  if (!/^[\p{Alphabetic}\p{N}._-]+$/u.test(name)) return 'Use letters, numbers, dots, dashes and underscores.'
  return undefined
}

// The server counts bytes, not characters.
const byteLength = (s: string) => new TextEncoder().encode(s).length

/** Username and password rules of the server (auth/handlers.rs). */
export function usernameError(name: string): string | undefined {
  return name && byteLength(name.trim()) < 3 ? 'Usernames need at least 3 characters.' : undefined
}
export function passwordError(pw: string): string | undefined {
  return pw && byteLength(pw) < 8 ? 'Passwords need at least 8 characters.' : undefined
}

/** What a storage form still lacks before a test or setup can work, in plain words. */
export function storageMissing(s: { kind: string; bucket?: string; container?: string }): string | undefined {
  if ((s.kind === 's3' || s.kind === 'gcs') && !s.bucket?.trim()) return 'Enter a bucket name.'
  if (s.kind === 'azure' && !s.container?.trim()) return 'Enter a container name.'
  return undefined
}
