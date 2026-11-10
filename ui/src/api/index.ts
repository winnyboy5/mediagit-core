// The only module that talks to the Tauri backend. Views and stores call
// these wrappers, so a later HTTP transport (server web UI) swaps in here.
import { Channel, convertFileSrc, invoke as rawInvoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  ApiError,
  AuthReport,
  BisectReport,
  CherryPickReport,
  ConfigReport,
  ConfigSchemaReport,
  FsckCliReport,
  GcReport,
  KeyReport,
  RebaseReport,
  ReflogAllReport,
  ReflogMaintainReport,
  BranchProtectReport,
  BranchEntryReport,
  ReflogReport,
  SparseReport,
  StatsReport,
  BranchActionReport,
  BranchListReport,
  DiffReport,
  DownloadReport,
  EngineInfo,
  Extracted,
  FetchReport,
  LocksReport,
  LsTreeReport,
  LockReport,
  LogReport,
  MergeReport,
  MediaInfo,
  Progress,
  PullReport,
  PushReport,
  RemotesReport,
  ResetReport,
  RevertReport,
  ShowCommitReport,
  StashReport,
  TagActionReport,
  StatusReport,
  TagListReport,
  TagShowReport,
  TagVerifyReport,
  VerifyRangeReport,
  AuthOpReport,
  HostedRepoActionReport,
  HostedReposReport,
  LocalServerStatus,
  StorageOpts,
  StorageTestResult,
} from './types'

import type { AddOpts, CommitOpts, FetchOpts, PullOpts, PushOpts } from './options'
import type { AddReport } from './generated/AddReport'
import type { LsFilesReport } from './generated/LsFilesReport'
import type { CommitDryRunReport } from './generated/CommitDryRunReport'
import type { RemoteShowReport } from './generated/RemoteShowReport'

export type * from './types'
export type * from './options'
export type { AddReport, CommitDryRunReport, LsFilesReport, RemoteShowReport }
export type { LsFilesEntry } from './generated/LsFilesEntry'

/** Contract version this UI was written against (engine.rs CONTRACT_VERSION). */
export const CONTRACT_VERSION = 1

export function isApiError(e: unknown): e is ApiError {
  return typeof e === 'object' && e !== null && 'code' in e && 'message' in e
}

export function toApiError(e: unknown): ApiError {
  return isApiError(e) ? e : { code: 'unknown', message: e instanceof Error ? e.message : String(e) }
}

function channel(onProgress?: (p: Progress) => void): Channel<Progress> {
  const c = new Channel<Progress>()
  c.onmessage = onProgress ?? (() => {})
  return c
}

type Report = (p: Progress) => void

/** Asks the person to unlock `repo`; true once it is unlocked, false if they cancelled. */
type UnlockHandler = (repo: string) => Promise<boolean>
let unlockHandler: UnlockHandler | null = null

/** Told when a command still needs the passphrase right after the person unlocked the repository. */
let stillLockedHandler: ((repo: string) => void) | null = null
export function setStillLockedHandler(h: ((repo: string) => void) | null) {
  stillLockedHandler = h
}

/** Registered once at startup (main.ts), so this module needs no store. */
export function setUnlockHandler(h: UnlockHandler | null) {
  unlockHandler = h
}

/**
 * Every command goes through here. An encrypted repository that is not unlocked fails
 * `needs_input`; the person is asked for the passphrase once (the backend keeps it in memory
 * for that repository) and the command is run again, once. The passphrase never passes
 * through this function: it only goes to `unlock_repo`.
 */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await rawInvoke<T>(cmd, args)
  } catch (e) {
    const repo = args?.repo
    if (toApiError(e).code !== 'needs_input' || typeof repo !== 'string' || !unlockHandler) throw e
    if (!(await unlockHandler(repo))) throw { code: 'cancelled', message: 'Cancelled' } satisfies ApiError
    try {
      return await rawInvoke<T>(cmd, args)
    } catch (e2) {
      if (toApiError(e2).code === 'needs_input') stillLockedHandler?.(repo)
      throw e2
    }
  }
}

/** Extra options of `tag create`; `force` is only sent with `confirmed`. */
export interface TagOptions {
  commit?: string | null
  message?: string | null
  tagger?: string | null
  email?: string | null
  force?: boolean
  confirmed?: boolean
}

/** Options of `gc` beyond preview-or-run. */
export interface GcOptions {
  noPrune?: boolean
  auto?: boolean
  repack?: boolean
}

/** Options of `fsck`. A real `repair` needs `confirmed`; `dryRun` needs `repair`. */
export interface FsckOptions {
  deep?: boolean
  all?: boolean
  lostFound?: boolean
  noDangling?: boolean
  repair?: boolean
  dryRun?: boolean
  confirmed?: boolean
}

/** Options of `verify`: the whole repository, one version (`commit`) or a range (`start`/`end`). */
export interface VerifyOptions {
  quick?: boolean
  commit?: string | null
  start?: string | null
  end?: string | null
  checksums?: boolean
  detailed?: boolean
}

/** Options of `stash apply|pop`: `index` also restores what was staged; `force` overwrites changed files (confirmed first). */
export interface StashRestore {
  index?: boolean
  force?: boolean
}
const stashRestoreArgs = (o: StashRestore) => ({ restoreIndex: !!o.index, force: !!o.force, confirmed: !!o.force })

export interface LogFilter {
  revision?: string
  author?: string
  grep?: string
  since?: string
  until?: string
  paths?: string[]
}
export interface BranchCreateOpts {
  startPoint?: string
  setUpstream?: string
  tracking?: 'track' | 'none'
}
export interface BranchSwitchOpts {
  create?: boolean
  track?: boolean
  force?: boolean
  confirmed?: boolean
}
export interface BranchDeleteOpts {
  force?: boolean
  mergedOnly?: boolean
  remote?: boolean
  confirmed?: boolean
}
export interface MergeOpts {
  noFf?: boolean
  ffOnly?: boolean
  squash?: boolean
  noCommit?: boolean
  message?: string
}

/** `recursive` asks per file; `ours`/`theirs` settle every conflict the same way. */
export type MergeStrategy = 'recursive' | 'ours' | 'theirs'

export const api = {
  engineInfo: () => invoke<EngineInfo>('engine_info'),
  repoStatus: (repo: string) => invoke<StatusReport>('repo_status', { repo }),
  /** Streams two bars: bytes and items (files done / total). */
  add: (repo: string, paths: string[], opts?: AddOpts, onProgress?: Report) =>
    invoke<AddReport>('repo_add', { repo, paths, opts: opts ?? null, onProgress: channel(onProgress) }),
  /** Every file of the working folder with its status; `ignored` adds the files the ignore list skips. */
  lsFiles: (repo: string, ignored = false, paths?: string[]) =>
    invoke<LsFilesReport>('repo_ls_files', { repo, paths: paths ?? null, ignored }),
  /** Take paths out of the staging area (`reset`); the files are not touched. */
  unstage: (repo: string, paths: string[]) => invoke<unknown>('repo_unstage', { repo, paths }),
  commit: (repo: string, message: string, opts?: CommitOpts) =>
    invoke<{ oid?: string } & Partial<CommitDryRunReport>>('repo_commit', { repo, message, opts: opts ?? null }),
  init: (path: string, initialBranch?: string) => invoke<unknown>('repo_init', { path, initialBranch: initialBranch ?? null }),
  clone: (url: string, dest: string, opId: string, onProgress: Report, branch?: string) =>
    invoke<unknown>('repo_clone', { url, dest, branch: branch ?? null, opId, onProgress: channel(onProgress) }),
  /** Stop a running push/pull/fetch/clone. */
  cancelOp: (opId: string) => invoke<boolean>('cancel_op', { opId }),
  /** Subscribe to working-copy changes; returns the unsubscribe function. */
  onRepoChanged: (cb: () => void) => listen('repo-changed', cb),

  /** Let the webview load files under this repository (previews). */
  allowRepo: (repo: string) => invoke<void>('repo_allow', { repo }),
  /** A URL the webview can load for a local file. */
  fileSrc: (path: string) => convertFileSrc(path),

  log: (repo: string, skip: number, max: number, all: boolean, path?: string, filter?: LogFilter) =>
    invoke<LogReport>('repo_log', { repo, skip, max, all, path: path ?? null, ...(filter ? { filter } : {}) }),
  show: (repo: string, rev: string, tree = false) => invoke<ShowCommitReport>('repo_show', { repo, rev, tree }),
  /** Write `<rev>:<path>` to the preview folder; `expected` is the size the tree reported. */
  extract: (repo: string, rev: string, path: string, expected: number | null, onProgress: Report) =>
    invoke<Extracted>('repo_extract', { repo, rev, path, expected, onProgress: channel(onProgress) }),
  mediaInfo: (repo: string, path: string) => invoke<MediaInfo>('media_info', { repo, path }),

  /** `remote` lists the remote-tracking branches (`origin/feature`) instead of the local ones. */
  branchList: (repo: string, remote = false) => invoke<BranchListReport>('branch_list', { repo, ...(remote ? { remote } : {}) }),
  branchCreate: (repo: string, name: string, opts?: BranchCreateOpts) =>
    invoke<BranchActionReport>('branch_create', { repo, name, ...(opts ? { opts } : {}) }),
  branchSwitch: (repo: string, name: string, onProgress: Report, opts?: BranchSwitchOpts) =>
    invoke<BranchActionReport>('branch_switch', { repo, name, ...(opts ? { opts } : {}), onProgress: channel(onProgress) }),
  branchDelete: (repo: string, name: string, opts?: BranchDeleteOpts) =>
    invoke<unknown>('branch_delete', { repo, name, ...(opts ? { opts } : {}) }),
  branchRename: (repo: string, from: string, to: string, overwrite = false) =>
    invoke<BranchActionReport>('branch_rename', { repo, from, to, ...(overwrite ? { force: true, confirmed: true } : {}) }),
  branchShow: (repo: string, name: string | null) => invoke<BranchEntryReport>('branch_show', { repo, name }),
  branchProtect: (repo: string, name: string, requireReviews: boolean, unprotect: boolean) =>
    invoke<BranchProtectReport>('branch_protect', { repo, name, requireReviews, unprotect, confirmed: unprotect }),
  /** `pattern`: a glob that narrows the list (`v1.*`). */
  tagList: (repo: string, pattern: string | null = null) => invoke<TagListReport>('tag_list', { repo, pattern }),
  tagShow: (repo: string, name: string) => invoke<TagShowReport>('tag_show', { repo, name }),
  tagVerify: (repo: string, name: string) => invoke<TagVerifyReport>('tag_verify', { repo, name }),
  /** Name a version. A message makes the tag annotated; `tagger`/`email` need one. `force` replaces an existing tag and needs `confirmed`. */
  tagCreate: (repo: string, name: string, options: TagOptions = {}) => invoke<TagActionReport>('tag_create', { repo, name, options }),
  tagDelete: (repo: string, name: string) => invoke<TagActionReport>('tag_delete', { repo, name }),

  lockList: (repo: string, remote?: string) => invoke<LocksReport>('lock_list', { repo, remote: remote ?? null }),
  lockCreate: (repo: string, path: string, owner?: string, remote?: string) =>
    invoke<LockReport>('lock_create', { repo, path, owner: owner ?? null, remote: remote ?? null }),
  lockUnlock: (repo: string, lockId: string, force = false, remote?: string) =>
    invoke<unknown>('lock_unlock', { repo, lockId, force, remote: remote ?? null }),

  remoteList: (repo: string) => invoke<RemotesReport>('remote_list', { repo }),
  /** `token` (optional) goes to the CLI on stdin only. */
  remoteAdd: (repo: string, name: string, url: string, opts: { fetch?: boolean; token?: string } = {}) =>
    invoke<RemotesReport>('remote_add', { repo, name, url, fetch: opts.fetch ?? false, token: opts.token || null }),
  remoteRename: (repo: string, old: string, to: string) => invoke<unknown>('remote_rename', { repo, old, new: to }),
  remoteSetUrl: (repo: string, name: string, url: string, push = false) => invoke<unknown>('remote_set_url', { repo, name, url, push }),
  remoteShow: (repo: string, name: string) => invoke<RemoteShowReport>('remote_show', { repo, name }),
  remoteRemove: (repo: string, name: string) => invoke<RemotesReport>('remote_remove', { repo, name }),
  fetch: (repo: string, remote: string | null, opId: string, onProgress: Report, opts?: FetchOpts) =>
    invoke<FetchReport>('repo_fetch', { repo, remote, opts: opts ?? null, opId, onProgress: channel(onProgress) }),
  pull: (repo: string, remote: string | null, opId: string, onProgress: Report, opts?: PullOpts) =>
    invoke<PullReport>('repo_pull', { repo, remote, opts: opts ?? null, opId, onProgress: channel(onProgress) }),
  push: (repo: string, remote: string | null, opId: string, onProgress: Report, opts?: PushOpts) =>
    invoke<PushReport>('repo_push', { repo, remote, opts: opts ?? null, opId, onProgress: channel(onProgress) }),

  merge: (repo: string, branch: string, strategy: MergeStrategy, opts: MergeOpts, onProgress: Report) =>
    invoke<MergeReport>('repo_merge', { repo, branch, strategy, opts, onProgress: channel(onProgress) }),
  mergeControl: (repo: string, action: 'continue' | 'abort', onProgress: Report) =>
    invoke<MergeReport>('repo_merge_control', { repo, action, onProgress: channel(onProgress) }),
  /** Take our version (`ours`) or `theirs` (a commit id) of one conflicted file, and stage it. */
  mergeResolve: (repo: string, path: string, side: 'ours' | 'theirs', theirs: string) =>
    invoke<unknown>('merge_resolve', { repo, path, side, theirs }),

  /** Two versions, or (both null) the working folder against the latest version. */
  /** `cached` (both null): the changes set aside for the next version, against the latest one. */
  diff: (repo: string, from: string | null, to: string | null, cached = false) =>
    invoke<DiffReport>('repo_diff', { repo, from, to, ...(cached ? { cached } : {}) }),

  stashList: (repo: string) => invoke<StashReport>('stash_list', { repo }),
  stashShow: (repo: string, index: number) => invoke<StashReport>('stash_show', { repo, index }),
  /** `paths` empty = everything; otherwise only those files are set aside (`stash push -- paths`). */
  stashSave: (repo: string, message: string | null, includeUntracked: boolean, paths: string[], onProgress: Report) =>
    invoke<StashReport>('stash_save', { repo, message, includeUntracked, paths, onProgress: channel(onProgress) }),
  stashApply: (repo: string, index: number, opts: StashRestore, onProgress: Report) =>
    invoke<StashReport>('stash_apply', { repo, index, ...stashRestoreArgs(opts), onProgress: channel(onProgress) }),
  stashPop: (repo: string, index: number, opts: StashRestore, onProgress: Report) =>
    invoke<StashReport>('stash_pop', { repo, index, ...stashRestoreArgs(opts), onProgress: channel(onProgress) }),
  stashDrop: (repo: string, index: number) => invoke<StashReport>('stash_drop', { repo, index }),
  stashClear: (repo: string) => invoke<StashReport>('stash_clear', { repo }),

  /** Move the current branch to `rev`, keeping every file as it is. Never touches files. */
  reset: (repo: string, rev: string, mode: 'soft' | 'mixed') => invoke<ResetReport>('repo_reset', { repo, rev, mode }),
  /** Move the branch to `rev` and overwrite the working folder. Unsaved work is lost. */
  resetHard: (repo: string, rev: string, confirmed: boolean, onProgress: Report) =>
    invoke<ResetReport>('repo_reset_hard', { repo, rev, confirmed, onProgress: channel(onProgress) }),
  revert: (repo: string, revs: string[], opts?: { noCommit?: boolean; message?: string }) =>
    invoke<RevertReport>('repo_revert', { repo, revs, ...(opts ? { opts } : {}) }),
  revertControl: (repo: string, action: 'continue' | 'abort' | 'skip') =>
    invoke<RevertReport>('repo_revert_control', { repo, action }),
  /** Overwrite one working file with its content at `rev`. */
  restoreFile: (repo: string, rev: string, path: string) => invoke<unknown>('repo_restore_file', { repo, rev, path }),
  /** One folder level of a remote repository; null ref = the server's default branch. */
  remoteLsTree: (repo: string, remote: string, dir: string | null, gitRef: string | null) =>
    invoke<LsTreeReport>('remote_ls_tree', { repo, remote, dir, gitRef }),
  /** Download a remote file into the preview folder so the webview can show it. */
  downloadPreview: (repo: string, remotePath: string, gitRef: string | null, opId: string, onProgress: Report) =>
    invoke<Extracted>('repo_download_preview', { repo, remotePath, gitRef, opId, onProgress: channel(onProgress) }),
  download: (repo: string, remotePath: string, gitRef: string | null, output: string, opId: string, onProgress: Report) =>
    invoke<DownloadReport>('repo_download', { repo, remotePath, gitRef, output, opId, onProgress: channel(onProgress) }),

  /** Check a passphrase and keep it for this repository for the session; rejects `wrong_passphrase`. */
  unlockRepo: (repo: string, passphrase: string) => rawInvoke<void>('unlock_repo', { repo, passphrase }),
  /** Forget the session passphrase. */
  lockRepo: (repo: string) => rawInvoke<void>('lock_repo', { repo }),

  // Secrets (`passphrase`, `password`, `code`) go to the child's stdin only; `passphrase` answers a
  // `needs_input` prompt on retry (see stores/unlock.ts).
  stats: (repo: string, passphrase?: string) => invoke<StatsReport>('repo_stats', { repo, passphrase: passphrase ?? null }),
  gc: (repo: string, dryRun: boolean, confirmed: boolean, options: GcOptions, passphrase: string | undefined, onProgress: Report) =>
    invoke<GcReport>('repo_gc', { repo, dryRun, confirmed, options, passphrase: passphrase ?? null, onProgress: channel(onProgress) }),
  fsck: (repo: string, options: FsckOptions, passphrase: string | undefined, onProgress: Report) =>
    invoke<FsckCliReport>('repo_fsck', { repo, options, passphrase: passphrase ?? null, onProgress: channel(onProgress) }),
  /** The whole repository gives an `FsckCliReport`; one version or a range gives a `VerifyRangeReport`. */
  verify: (repo: string, options: VerifyOptions, passphrase: string | undefined, onProgress: Report) =>
    invoke<FsckCliReport | VerifyRangeReport>('repo_verify', { repo, options, passphrase: passphrase ?? null, onProgress: channel(onProgress) }),

  authLogin: (repo: string | null, server: string | null, username: string | null, password: string) =>
    rawInvoke<AuthReport>('auth_login', { repo, server, username, password }),
  authRun: (repo: string | null, action: 'status' | 'whoami' | 'logout', server: string | null, all = false) =>
    invoke<AuthReport>('auth_run', { repo, action, server, all }),
  // Server panel. `server` is an origin (http(s)://host[:port]); passwords are one call's argument.
  serverLocalStatus: () => rawInvoke<LocalServerStatus>('server_local_status'),
  serverLocalSetup: (username: string, email: string, password: string, storage?: StorageOpts) =>
    rawInvoke<LocalServerStatus>('server_local_setup', { username, email, password, storage }),
  /** No path = the default folder. */
  serverLocalSetFolder: (path?: string) => rawInvoke<LocalServerStatus>('server_local_set_folder', { path }),
  serverStorageTest: (storage: StorageOpts) => rawInvoke<StorageTestResult>('server_storage_test', { storage }),
  serverLocalStart: () => rawInvoke<LocalServerStatus>('server_local_start'),
  serverLocalStop: () => rawInvoke<LocalServerStatus>('server_local_stop'),
  serverLocalLog: (lines: number) => rawInvoke<string>('server_local_log', { lines }),
  serverLocalSetNetwork: (enabled: boolean) => rawInvoke<LocalServerStatus>('server_local_set_network', { enabled }),
  serverWhoami: (server: string) => rawInvoke<AuthReport>('auth_run', { repo: null, action: 'status', server, all: false }),
  serverLogin: (server: string, username: string, password: string) =>
    rawInvoke<AuthReport>('auth_login', { repo: null, server, username, password }),
  serverUsers: (server: string) => rawInvoke<AuthOpReport>('server_users_list', { server }),
  serverUserCreate: (server: string, user: string, email: string, role: string, password: string) =>
    rawInvoke<AuthOpReport>('server_user_create', { server, user, email, role, password }),
  serverUserSetRole: (server: string, user: string, role: string) => rawInvoke<AuthOpReport>('server_user_set_role', { server, user, role }),
  serverUserResetPassword: (server: string, user: string, password: string) =>
    rawInvoke<AuthOpReport>('server_user_reset_password', { server, user, password }),
  serverUserDisable: (server: string, user: string) => rawInvoke<AuthOpReport>('server_user_disable', { server, user, confirm: true }),
  serverUserEnable: (server: string, user: string) => rawInvoke<AuthOpReport>('server_user_enable', { server, user }),
  serverUserGrants: (server: string, user: string) => rawInvoke<AuthOpReport>('server_user_grants', { server, user }),
  serverRepoAccess: (server: string, repo: string) => rawInvoke<AuthOpReport>('server_repo_access', { server, repo }),
  serverGrantSet: (server: string, user: string, repo: string, level: string) =>
    rawInvoke<AuthOpReport>('server_grant_set', { server, user, repo, level }),
  serverGrantRemove: (server: string, user: string, repo: string) =>
    rawInvoke<AuthOpReport>('server_grant_remove', { server, user, repo, confirm: true }),
  serverKeys: (server: string) => rawInvoke<AuthOpReport>('server_keys_list', { server }),
  serverKeyCreate: (server: string, name: string, permissions: string[]) =>
    rawInvoke<AuthOpReport>('server_key_create', { server, name, permissions }),
  serverKeyRevoke: (server: string, id: string) => rawInvoke<AuthOpReport>('server_key_revoke', { server, id, confirm: true }),
  serverRepos: (server: string) => rawInvoke<HostedReposReport>('server_repos_list', { server }),
  serverRepoCreate: (server: string, name: string) => rawInvoke<HostedRepoActionReport>('server_repo_create', { server, name }),
  serverRepoDelete: (server: string, name: string, typedName: string, force: boolean) =>
    rawInvoke<HostedRepoActionReport>('server_repo_delete', { server, name, typedName, force, confirm: true }),

  authRegister: (server: string | null, username: string, email: string, password: string) =>
    rawInvoke<AuthOpReport>('auth_register', { server, username, email, password }),
  authPasswd: (server: string | null, current: string, newPassword: string) =>
    rawInvoke<AuthOpReport>('auth_passwd', { server, current, new: newPassword }),
  /** Re-lock the key under a new master: a key file path, or a new passphrase (`current` only if the key is passphrase-protected now). */
  keyRotateMaster: (repo: string, newKeyfile: string | null, current: string | null, newPassphrase: string | null) =>
    rawInvoke<KeyReport>('key_rotate_master', { repo, newKeyfile, current, newPassphrase }),
  keyStatus: (repo: string, passphrase?: string) => invoke<KeyReport>('key_status', { repo, passphrase: passphrase ?? null }),
  keyInit: (repo: string, passphrase: string | null) => rawInvoke<KeyReport>('key_init', { repo, passphrase }),
  keyRecover: (repo: string, code: string, passphrase: string | null) => rawInvoke<KeyReport>('key_recover', { repo, code, passphrase }),

  configList: (repo: string) => invoke<ConfigReport>('config_list', { repo }),
  /** `secret`: the value goes to the CLI privately (stdin) and is never shown again. */
  configSet: (repo: string, key: string, value: string, secret = false) => invoke<ConfigReport>('config_set', { repo, key, value, secret }),
  /** Every setting the CLI knows (type, default, documentation, secret or not). Needs no repository. */
  configSchema: () => rawInvoke<ConfigSchemaReport>('config_schema'),
  configUnset: (repo: string, key: string) => invoke<ConfigReport>('config_unset', { repo, key }),
  sparseList: (repo: string) => invoke<SparseReport>('sparse_list', { repo }),
  sparseSet: (repo: string, patterns: string[], patternMode: boolean, onProgress: Report) =>
    invoke<SparseReport>('sparse_set', { repo, patterns, patternMode, onProgress: channel(onProgress) }),
  sparseDisable: (repo: string, onProgress: Report) => invoke<SparseReport>('sparse_disable', { repo, onProgress: channel(onProgress) }),

  rebaseStart: (repo: string, upstream: string, onProgress: Report, extra?: { branch?: string; keepEmpty?: boolean }) =>
    invoke<RebaseReport>('rebase_start', { repo, upstream, ...extra, onProgress: channel(onProgress) }),
  sequencerControl: <T = RebaseReport | CherryPickReport>(repo: string, command: 'rebase' | 'cherry-pick', action: 'continue' | 'abort' | 'skip', onProgress: Report) =>
    invoke<T>('sequencer_control', { repo, command, action, onProgress: channel(onProgress) }),
  pickStart: (repo: string, commits: string[], onProgress: Report, opts?: { noCommit?: boolean; appendMessage?: boolean }) =>
    invoke<CherryPickReport>('pick_start', { repo, commits, ...(opts ? { opts } : {}), onProgress: channel(onProgress) }),
  /** `restart` throws away a bisect already running; the person has agreed to that. */
  bisectStart: (repo: string, bad: string, good: string | null, onProgress: Report, restart = false) =>
    invoke<BisectReport>('bisect_start', { repo, bad, good, ...(restart ? { restart, confirmed: true } : {}), onProgress: channel(onProgress) }),
  bisectMark: (repo: string, verdict: 'good' | 'bad' | 'skip', commit: string | null, onProgress: Report) =>
    invoke<BisectReport>('bisect_mark', { repo, verdict, commit, onProgress: channel(onProgress) }),
  /** `commit` moves the branch there instead of back to where the bisect began; the person has agreed. */
  bisectReset: (repo: string, onProgress: Report, commit?: string) =>
    invoke<BisectReport>('bisect_reset', { repo, ...(commit ? { commit, confirmed: true } : {}), onProgress: channel(onProgress) }),
  bisectReplay: (repo: string, logfile: string, onProgress: Report) =>
    invoke<BisectReport>('bisect_replay', { repo, logfile, onProgress: channel(onProgress) }),
  bisectLog: (repo: string) => invoke<BisectReport>('bisect_log', { repo }),
  reflog: (repo: string, reference: string, count: number) => invoke<ReflogReport>('reflog_list', { repo, reference, count }),
  reflogAll: (repo: string, count: number) => invoke<ReflogAllReport>('reflog_all', { repo, count }),
  /** Forget where a ref has been. Cannot be undone; the person has agreed. */
  reflogDelete: (repo: string, reference: string) => invoke<ReflogMaintainReport>('reflog_delete', { repo, reference, confirmed: true }),
  /** Keep only the newest `keep` entries (of `reference`, or of every ref when null). Cannot be undone. */
  reflogExpire: (repo: string, reference: string | null, keep: number) =>
    invoke<ReflogMaintainReport>('reflog_expire', { repo, reference, keep, confirmed: true }),
}
