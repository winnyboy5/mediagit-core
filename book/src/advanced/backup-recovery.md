# Backup and Recovery

Strategies for backing up MediaGit repositories and recovering from failures.

## What to Back Up

A MediaGit repository consists of:

```
.mediagit/
├── objects/          # Content-addressable object database (chunks, trees, commits)
├── manifests/        # Chunk manifests (bincode format) — index of all chunks per file
├── refs/             # Branch and tag references
├── HEAD              # Current branch pointer
├── config.toml       # Repository configuration
└── stats/            # Operation statistics (non-critical)
```

The `objects/` directory is the most important — it contains all file content. `manifests/` and `refs/` are smaller but required for correct operation.

---

## Backup Strategies

### 1. Full Repository Archive (Local)

The simplest backup: archive the entire `.mediagit/` directory.

```bash
# Create timestamped backup
tar czf mediagit-backup-$(date +%Y%m%d-%H%M%S).tar.gz .mediagit/

# Verify backup integrity
tar tzf mediagit-backup-*.tar.gz | head -20
```

Schedule daily backups with cron:

```bash
# /etc/cron.d/mediagit-backup
0 2 * * * /usr/bin/tar czf /backups/mediagit-$(date +%Y%m%d).tar.gz /path/to/repo/.mediagit/
```

### 2. Push to a Remote (Recommended)

The safest backup is a live remote that always has the latest state:

```bash
# Configure a backup remote (different server or cloud account)
# In .mediagit/config.toml:
[remotes.origin]
url = "http://primary-server.example.com/my-project"

[remotes.backup]
url = "http://backup-server.example.com/my-project"

# Push to both
mediagit push origin main
mediagit push backup main
```

For cloud storage backends, the provider handles redundancy. S3 and Azure Blob have 99.999999999% (11 nines) durability by default.

### 3. Sync to Cloud Storage

If you use a local filesystem backend and want to also back up to cloud storage, sync the `.mediagit/objects/` directory:

```bash
# Sync to S3
aws s3 sync .mediagit/objects/ s3://my-backup-bucket/mediagit-objects/

# Sync to Google Cloud Storage
gsutil -m rsync -r .mediagit/objects/ gs://my-backup-bucket/mediagit-objects/
```

### 4. Incremental Backup

Because MediaGit uses content-addressable storage, objects are immutable once written. Incremental backup is straightforward — only new files need to be copied:

```bash
# rsync is efficient: only copies new objects
rsync -av --checksum .mediagit/objects/ backup-host:/backup/mediagit-objects/
```

---

## Before Critical Operations

Always verify repository integrity and create a backup before major operations:

```bash
# Verify integrity
mediagit fsck

# Create safety snapshot
tar czf .mediagit-backup-$(date +%Y%m%d-%H%M%S).tar.gz .mediagit/

# Proceed with operation
mediagit rebase main
```

---

## Recovery Procedures

### Restore from Archive

```bash
# Remove damaged repository
rm -rf .mediagit/

# Restore from backup
tar xzf mediagit-backup-20260101-020000.tar.gz

# Verify restored repo
mediagit fsck
mediagit log --oneline -5
```

### Restore from Remote

If your local repository is damaged but the remote is intact:

```bash
# Re-clone from remote
mediagit clone http://media-server.example.com/my-project my-project-restored
cd my-project-restored
mediagit fsck
```

### Repair Corrupt Objects

MediaGit's fsck can attempt automatic repair:

```bash
# Check what's damaged
mediagit fsck

# Attempt repair
mediagit fsck --repair

# If objects are missing, fetch from remote
mediagit fetch
mediagit fsck
```

If the remote has the correct objects, a fetch followed by fsck will usually restore a damaged local repository.

### Recover Deleted Branch

Deleted branches may still be reachable via the reflog:

```bash
# Show recent ref history
mediagit reflog

# The deleted branch's last commit will appear
# Recreate the branch
mediagit branch create recovered-branch <commit-hash>
```

### Lost-Found Recovery

If objects become dangling (unreferenced) after a failed operation:

```bash
# Find dangling objects
mediagit fsck --dangling

# Save them to lost-found
mediagit fsck --lost-found

# Inspect what was saved
ls .mediagit/lost-found/
```

---

## Disaster Recovery Plan

For production environments managing large media repositories:

1. **Regular pushes** to a remote on a different server or cloud provider
2. **Daily archives** of `.mediagit/` directory stored offsite
3. **Weekly `mediagit fsck`** to detect corruption early
4. **Monthly restore test** — actually restore from backup to confirm it works
5. **Document recovery procedures** for your team

---

## Estimating Backup Size

The backup size equals the repository's stored object size. Check with:

```bash
du -sh .mediagit/objects/
du -sh .mediagit/manifests/
du -sh .mediagit/   # total
```

For cloud backends, the objects are already stored remotely — only the `.mediagit/refs/`, `.mediagit/HEAD`, and `.mediagit/config.toml` need local backup (these are typically a few KB).

---

## See Also

- [mediagit fsck](../cli/fsck.md) — repository integrity verification
- [mediagit verify](../cli/verify.md) — verify specific objects
- [mediagit gc](../cli/gc.md) — garbage collection before backup
- [Storage Backend Configuration](../guides/storage-config.md)
