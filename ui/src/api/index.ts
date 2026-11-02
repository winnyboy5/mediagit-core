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
  FsckCliReport,
  GcReport,
  KeyReport,
  RebaseReport,
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
} from './types'

export type * from './types'

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

/** `recursive` asks per file; `ours`/`theirs` settle every conflict the same way. */
export type MergeStrategy = 'recursive' | 'ours' | 'theirs'

export const api = {
  engineInfo: () => invoke<EngineInfo>('engine_info'),
  repoStatus: (repo: string) => invoke<StatusReport>('repo_status', { repo }),
  add: (repo: string, paths: string[]) => invoke<unknown>('repo_add', { repo, paths }),
  commit: (repo: string, message: string) => invoke<unknown>('repo_commit', { repo, message }),
  init: (path: string) => invoke<unknown>('repo_init', { path }),
  clone: (url: string, dest: string, opId: string, onProgress: Report) =>
    invoke<unknown>('repo_clone', { url, dest, opId, onProgress: channel(onProgress) }),
  /** Stop a running push/pull/fetch/clone. */
  cancelOp: (opId: string) => invoke<boolean>('cancel_op', { opId }),
  /** Subscribe to working-copy changes; returns the unsubscribe function. */
  onRepoChanged: (cb: () => void) => listen('repo-changed', cb),

  /** Let the webview load files under this repository (previews). */
  allowRepo: (repo: string) => invoke<void>('repo_allow', { repo }),
  /** A URL the webview can load for a local file. */
  fileSrc: (path: string) => convertFileSrc(path),

  log: (repo: string, skip: number, max: number, all: boolean, path?: string) =>
    invoke<LogReport>('repo_log', { repo, skip, max, all, path: path ?? null }),
  show: (repo: string, rev: string, tree = false) => invoke<ShowCommitReport>('repo_show', { repo, rev, tree }),
  /** Write `<rev>:<path>` to the preview folder; `expected` is the size the tree reported. */
  extract: (repo: string, rev: string, path: string, expected: number | null, onProgress: Report) =>
    invoke<Extracted>('repo_extract', { repo, rev, path, expected, onProgress: channel(onProgress) }),
  mediaInfo: (repo: string, path: string) => invoke<MediaInfo>('media_info', { repo, path }),

  branchList: (repo: string) => invoke<BranchListReport>('branch_list', { repo }),
  branchCreate: (repo: string, name: string) => invoke<BranchActionReport>('branch_create', { repo, name }),
  branchSwitch: (repo: string, name: string, onProgress: Report) =>
    invoke<BranchActionReport>('branch_switch', { repo, name, onProgress: channel(onProgress) }),
  branchDelete: (repo: string, name: string, force = false) => invoke<unknown>('branch_delete', { repo, name, force }),
  branchRename: (repo: string, from: string, to: string) => invoke<BranchActionReport>('branch_rename', { repo, from, to }),
  tagList: (repo: string) => invoke<TagListReport>('tag_list', { repo }),
  /** Name a version (`commit` null = the latest); a message makes the tag annotated. */
  tagCreate: (repo: string, name: string, commit: string | null, message: string | null) =>
    invoke<TagActionReport>('tag_create', { repo, name, commit, message }),
  tagDelete: (repo: string, name: string) => invoke<TagActionReport>('tag_delete', { repo, name }),

  lockList: (repo: string) => invoke<LocksReport>('lock_list', { repo }),
  lockCreate: (repo: string, path: string) => invoke<LockReport>('lock_create', { repo, path }),
  lockUnlock: (repo: string, lockId: string, force = false) => invoke<unknown>('lock_unlock', { repo, lockId, force }),

  remoteList: (repo: string) => invoke<RemotesReport>('remote_list', { repo }),
  remoteAdd: (repo: string, name: string, url: string) => invoke<RemotesReport>('remote_add', { repo, name, url }),
  remoteRemove: (repo: string, name: string) => invoke<RemotesReport>('remote_remove', { repo, name }),
  fetch: (repo: string, remote: string | null, opId: string, onProgress: Report) =>
    invoke<FetchReport>('repo_fetch', { repo, remote, opId, onProgress: channel(onProgress) }),
  pull: (repo: string, remote: string | null, opId: string, onProgress: Report) =>
    invoke<PullReport>('repo_pull', { repo, remote, opId, onProgress: channel(onProgress) }),
  push: (repo: string, remote: string | null, opId: string, onProgress: Report) =>
    invoke<PushReport>('repo_push', { repo, remote, opId, onProgress: channel(onProgress) }),

  merge: (repo: string, branch: string, strategy: MergeStrategy, noFf: boolean, onProgress: Report) =>
    invoke<MergeReport>('repo_merge', { repo, branch, strategy, noFf, onProgress: channel(onProgress) }),
  mergeControl: (repo: string, action: 'continue' | 'abort', onProgress: Report) =>
    invoke<MergeReport>('repo_merge_control', { repo, action, onProgress: channel(onProgress) }),
  /** Take our version (`ours`) or `theirs` (a commit id) of one conflicted file, and stage it. */
  mergeResolve: (repo: string, path: string, side: 'ours' | 'theirs', theirs: string) =>
    invoke<unknown>('merge_resolve', { repo, path, side, theirs }),

  /** Two versions, or (both null) the working folder against the latest version. */
  diff: (repo: string, from: string | null, to: string | null) => invoke<DiffReport>('repo_diff', { repo, from, to }),

  stashList: (repo: string) => invoke<StashReport>('stash_list', { repo }),
  stashShow: (repo: string, index: number) => invoke<StashReport>('stash_show', { repo, index }),
  stashSave: (repo: string, message: string | null, includeUntracked: boolean, onProgress: Report) =>
    invoke<StashReport>('stash_save', { repo, message, includeUntracked, onProgress: channel(onProgress) }),
  stashApply: (repo: string, index: number, onProgress: Report) =>
    invoke<StashReport>('stash_apply', { repo, index, onProgress: channel(onProgress) }),
  stashPop: (repo: string, index: number, onProgress: Report) =>
    invoke<StashReport>('stash_pop', { repo, index, onProgress: channel(onProgress) }),
  stashDrop: (repo: string, index: number) => invoke<StashReport>('stash_drop', { repo, index }),
  stashClear: (repo: string) => invoke<StashReport>('stash_clear', { repo }),

  /** Move the current branch to `rev`, keeping every file as it is. Never touches files. */
  reset: (repo: string, rev: string, mode: 'soft' | 'mixed') => invoke<ResetReport>('repo_reset', { repo, rev, mode }),
  /** Move the branch to `rev` and overwrite the working folder. Unsaved work is lost. */
  resetHard: (repo: string, rev: string, onProgress: Report) =>
    invoke<ResetReport>('repo_reset_hard', { repo, rev, onProgress: channel(onProgress) }),
  revert: (repo: string, rev: string) => invoke<RevertReport>('repo_revert', { repo, rev }),
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
  gc: (repo: string, dryRun: boolean, confirmed: boolean, passphrase: string | undefined, onProgress: Report) =>
    invoke<GcReport>('repo_gc', { repo, dryRun, confirmed, passphrase: passphrase ?? null, onProgress: channel(onProgress) }),
  fsck: (repo: string, deep: boolean, passphrase: string | undefined, onProgress: Report) =>
    invoke<FsckCliReport>('repo_fsck', { repo, deep, passphrase: passphrase ?? null, onProgress: channel(onProgress) }),
  verify: (repo: string, quick: boolean, passphrase: string | undefined, onProgress: Report) =>
    invoke<FsckCliReport>('repo_verify', { repo, quick, passphrase: passphrase ?? null, onProgress: channel(onProgress) }),

  authLogin: (repo: string | null, server: string | null, username: string | null, password: string) =>
    rawInvoke<AuthReport>('auth_login', { repo, server, username, password }),
  authRun: (repo: string | null, action: 'status' | 'whoami' | 'logout', server: string | null, all = false) =>
    invoke<AuthReport>('auth_run', { repo, action, server, all }),
  keyStatus: (repo: string, passphrase?: string) => invoke<KeyReport>('key_status', { repo, passphrase: passphrase ?? null }),
  keyInit: (repo: string, passphrase: string | null) => rawInvoke<KeyReport>('key_init', { repo, passphrase }),
  keyRecover: (repo: string, code: string, passphrase: string | null) => rawInvoke<KeyReport>('key_recover', { repo, code, passphrase }),

  configList: (repo: string) => invoke<ConfigReport>('config_list', { repo }),
  configSet: (repo: string, key: string, value: string) => invoke<ConfigReport>('config_set', { repo, key, value }),
  configUnset: (repo: string, key: string) => invoke<ConfigReport>('config_unset', { repo, key }),
  sparseList: (repo: string) => invoke<SparseReport>('sparse_list', { repo }),
  sparseSet: (repo: string, patterns: string[], patternMode: boolean, onProgress: Report) =>
    invoke<SparseReport>('sparse_set', { repo, patterns, patternMode, onProgress: channel(onProgress) }),
  sparseDisable: (repo: string, onProgress: Report) => invoke<SparseReport>('sparse_disable', { repo, onProgress: channel(onProgress) }),

  rebaseStart: (repo: string, upstream: string, onProgress: Report) =>
    invoke<RebaseReport>('rebase_start', { repo, upstream, onProgress: channel(onProgress) }),
  sequencerControl: <T = RebaseReport | CherryPickReport>(repo: string, command: 'rebase' | 'cherry-pick', action: 'continue' | 'abort' | 'skip', onProgress: Report) =>
    invoke<T>('sequencer_control', { repo, command, action, onProgress: channel(onProgress) }),
  pickStart: (repo: string, commits: string[], onProgress: Report) =>
    invoke<CherryPickReport>('pick_start', { repo, commits, onProgress: channel(onProgress) }),
  bisectStart: (repo: string, bad: string, good: string | null, onProgress: Report) =>
    invoke<BisectReport>('bisect_start', { repo, bad, good, onProgress: channel(onProgress) }),
  bisectMark: (repo: string, verdict: 'good' | 'bad' | 'skip', commit: string | null, onProgress: Report) =>
    invoke<BisectReport>('bisect_mark', { repo, verdict, commit, onProgress: channel(onProgress) }),
  bisectReset: (repo: string, onProgress: Report) => invoke<BisectReport>('bisect_reset', { repo, onProgress: channel(onProgress) }),
  bisectLog: (repo: string) => invoke<BisectReport>('bisect_log', { repo }),
  reflog: (repo: string, reference: string, count: number) => invoke<ReflogReport>('reflog_list', { repo, reference, count }),
}
