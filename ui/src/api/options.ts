// Option bags for the Sync, Remote, Welcome, Locks and Changes commands. Field names match the
// serde structs in crates/mediagit-desktop/src/commands/views_a.rs (camelCase).

export interface FetchOpts {
  branch?: string
  all?: boolean
  prune?: boolean
}

export interface PullOpts {
  branch?: string
  rebase?: boolean
  dryRun?: boolean
}

/** `overwrite`: `lease` = `--force-with-lease` (safe), `force` = `--force`. Risky options need `confirmed`. */
export interface PushOpts {
  refspecs?: string[]
  all?: boolean
  tags?: boolean
  followTags?: boolean
  dryRun?: boolean
  overwrite?: 'lease' | 'force'
  delete?: boolean
  setUpstream?: boolean
  noTrack?: boolean
  repair?: boolean
  confirmed?: boolean
}

export interface AddOpts {
  all?: boolean
  dryRun?: boolean
  force?: boolean
  update?: boolean
  ignoreRemoval?: boolean
}

export interface CommitOpts {
  messageFile?: string
  all?: boolean
  include?: string[]
  author?: string
  date?: string
  allowEmpty?: boolean
  signoff?: boolean
  dryRun?: boolean
}
