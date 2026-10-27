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
