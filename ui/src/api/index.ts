// The only module that talks to the Tauri backend. Views and stores call
// these wrappers, so a later HTTP transport (server web UI) swaps in here.
import { Channel, convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  ApiError,
  BranchActionReport,
  BranchListReport,
  EngineInfo,
  Extracted,
  FetchReport,
  LocksReport,
  LockReport,
  LogReport,
  MediaInfo,
  Progress,
  PullReport,
  PushReport,
  RemotesReport,
  ShowCommitReport,
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
}
