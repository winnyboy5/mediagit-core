// Report shapes come from the CLI via ts-rs (src/api/generated/); only the
// desktop-side types live here.
export type { StatusReport } from './generated/StatusReport'
export type { BranchReport } from './generated/BranchReport'
export type { UpstreamReport } from './generated/UpstreamReport'
export type { StagedKind } from './generated/StagedKind'
export type { LogReport } from './generated/LogReport'
export type { LogCommitReport } from './generated/LogCommitReport'
export type { LogRefReport } from './generated/LogRefReport'
export type { ShowCommitReport } from './generated/ShowCommitReport'
export type { ShowFileReport } from './generated/ShowFileReport'
export type { ShowTreeEntryReport } from './generated/ShowTreeEntryReport'
export type { BranchListReport } from './generated/BranchListReport'
export type { BranchEntryReport } from './generated/BranchEntryReport'
export type { BranchActionReport } from './generated/BranchActionReport'
export type { TagListReport } from './generated/TagListReport'
export type { TagReport } from './generated/TagReport'
export type { TagShowReport } from './generated/TagShowReport'
export type { TagVerifyReport } from './generated/TagVerifyReport'
export type { VerifyRangeReport } from './generated/VerifyRangeReport'
export type { LocksReport } from './generated/LocksReport'
export type { LockReport } from './generated/LockReport'
export type { RemotesReport } from './generated/RemotesReport'
export type { RemoteReport } from './generated/RemoteReport'
export type { FetchReport } from './generated/FetchReport'
export type { PullReport } from './generated/PullReport'
export type { PushReport } from './generated/PushReport'
export type { MergeReport } from './generated/MergeReport'
export type { MergeConflictReport } from './generated/MergeConflictReport'
export type { DiffReport } from './generated/DiffReport'
export type { DiffFileReport } from './generated/DiffFileReport'
export type { StashReport } from './generated/StashReport'
export type { StashEntryReport } from './generated/StashEntryReport'
export type { ResetReport } from './generated/ResetReport'
export type { RevertReport } from './generated/RevertReport'
export type { TagActionReport } from './generated/TagActionReport'
export type { LsTreeReport } from './generated/LsTreeReport'
export type { LsTreeEntryReport } from './generated/LsTreeEntryReport'
export type { StatsReport } from './generated/StatsReport'
export type { StatsAuthorReport } from './generated/StatsAuthorReport'
export type { GcReport } from './generated/GcReport'
export type { FsckCliReport } from './generated/FsckCliReport'
export type { FsckIssueReport } from './generated/FsckIssueReport'
export type { AuthReport } from './generated/AuthReport'
export type { AuthIdentity } from './generated/AuthIdentity'
export type { KeyReport } from './generated/KeyReport'
export type { ConfigReport } from './generated/ConfigReport'
export type { ConfigEntry } from './generated/ConfigEntry'

export type { ConfigKeySchema } from './generated/ConfigKeySchema'
export type { ConfigSchemaReport } from './generated/ConfigSchemaReport'
export type { SparseReport } from './generated/SparseReport'
export type { RebaseReport } from './generated/RebaseReport'
export type { CherryPickReport } from './generated/CherryPickReport'
export type { BisectReport } from './generated/BisectReport'
export type { ReflogReport } from './generated/ReflogReport'
export type { ReflogAllReport } from './generated/ReflogAllReport'
export type { ReflogMaintainReport } from './generated/ReflogMaintainReport'
export type { BranchProtectReport } from './generated/BranchProtectReport'
export type { ReflogEntryReport } from './generated/ReflogEntryReport'
export type { DownloadReport } from './generated/DownloadReport'

export interface ApiError {
  code: string
  message: string
}

export interface EngineInfo {
  version: string
  contract: number
}

/** One sampled progress bar streamed from the CLI (engine.rs `Progress`). */
export interface Progress {
  id: number
  msg: string
  pos: number
  len: number | null
  unit: 'bytes' | 'items'
  done: boolean
}

/** A blob written to the app's preview folder (`repo_extract`). */
export interface Extracted {
  path: string
  size: number
}

/** `media info` data: the parser's own struct, different per file type. */
export type MediaInfo = Record<string, unknown>

// -- Server panel -----------------------------------------------------------
// Mirrors the CLI's `auth admin|key` (AuthOpReport) and `remote repos` reports and the
// desktop's LocalStatus (server_manager.rs). Swap for src/api/generated/ once exported.

/** The local server as the app sees it. `elsewhere`: something answers, but this app did not start it. */
export interface LocalServerStatus {
  state: 'not_set_up' | 'stopped' | 'running' | 'elsewhere'
  url: string | null
  port: number | null
  pid: number | null
  data_dir: string
  log_path: string
  lan: boolean
}

export type StorageKind = 'filesystem' | 's3' | 'gcs' | 'azure'

/** Where a server keeps repository data. The secret fields travel in one call and are never kept. */
export interface StorageOpts {
  kind: StorageKind
  bucket?: string
  region?: string
  endpoint?: string
  prefix?: string
  container?: string
  account?: string
  project?: string
  access_key_id?: string
  secret_access_key?: string
  account_key?: string
  connection_string?: string
  credentials_path?: string
}

export interface StorageTestResult {
  ok: boolean
  error?: string
}

export type ServerRole = 'read' | 'write' | 'admin'

export interface ServerUser {
  id: string
  username: string
  role: ServerRole
  /** Cannot sign in; the account is kept. */
  disabled?: boolean
}

export interface ServerKey {
  id: string
  name: string
  /** Unix seconds. */
  created_at: number
}

/** `auth admin *` and `auth key *` data. */
export interface AuthOpReport {
  action: string
  server: string
  id: string | null
  name: string | null
  role: string | null
  repo: string | null
  level: string | null
  /** `key-create`: the new key, shown once. */
  key: string | null
  permissions: string[]
  keys: ServerKey[]
  users: ServerUser[]
  /** `list-grants`: what one person can reach. */
  grants?: { repo: string; level: string }[]
  /** `repo-access`: who can reach one repository. */
  access?: { user_id: string; username: string; level: string }[]
  note: string | null
}

export interface HostedRepo {
  name: string
  size_bytes: number
  object_count: number | null
  /** Unix seconds. */
  last_modified: number
}

export interface HostedReposReport {
  server: string
  repos: HostedRepo[]
}

export interface HostedRepoActionReport {
  action: 'create' | 'delete'
  server: string
  name: string
  repo: HostedRepo | null
  deleted: boolean
  /** The storage bucket still holds the repository's data. */
  storage_retained: boolean
  backend: string | null
}
