# Desktop App

> **Preview** (targets v0.8.0, release candidate). The desktop app is a window around the `mediagit` command line. Everything it does, you can also do with the CLI, and it works on the same repositories.

The app runs one `mediagit` command for each action, so results match the CLI exactly. Nothing is sent anywhere except to the remotes and servers you set up yourself.

## Finding your way around

The sidebar has four main areas and Settings. Each area has a few sections, shown as tabs along the top.

| Area | Sections | Use it to |
|------|----------|-----------|
| **Files** | All files, Changes | See every file in your project, choose what to include, and save a version |
| **History** | Versions, Compare, Tags | Browse saved versions, compare two of them, name important ones |
| **Branches** | Branches, Merge, Sync, Stash | Work on parallel lines of work, bring them together, share with your team |
| **Team** | Locks, Remote files, Server | Lock files while you edit, browse a remote without downloading, run or manage a server |
| **Settings** | General, Storage, Maintenance, Security, Advanced tools | Your name and email, repository size, clean-up and checks, encryption, power tools |

Fetch, Get latest and Send buttons sit in the header of the first four areas, so you can sync from wherever you are. Older bookmarks and links still work: each old page name opens its new place.

## Welcome

The first screen. Open a folder that is already a repository, create a new repository, or clone one from an `http(s)` address or a local path. Clone shows progress and can be cancelled. Recent repositories are listed so you can reopen them. Once a repository is open, the repository menu at the top of the sidebar can also create or clone another one.

## Files

### All files

A browsable tree of everything in your working folder, not only what changed. Each file shows whether it is unchanged, modified, new, or deleted. Folders show how many files they hold, how many of those changed, and their total size. The list stays fast with 100,000 files or more.

The app watches the folder: if you save a file in Blender or Photoshop, the list updates by itself a moment later, with no refresh needed. Select a file to see its details on the right (see Inspector previews below).

### Changes and Include

The Changes section lists only what differs from your last saved version. Every row has an **Include** checkbox. Ticked rows go into your next version; unticked rows stay as they are, and nothing is lost either way. **Include all** ticks everything at once. In All files, the checkbox appears on changed files only.

Below the list, describe what changed and choose **Save version**. The button stays off until at least one change is included and, where needed, a message is written. More options, such as also including paths that are not listed, are behind **More options**.

While files are being added, a progress panel shows the number of files and bytes done, the transfer rate, and the time left. These numbers are measured, never estimated before there is data to measure.

## History

Saved versions, newest first, drawn as a graph with branch and tag labels. Select a version to see the files it changed as thumbnails, or browse every file as it was at that version.

- **Compare**: pick two different versions (or your working folder against the latest version) to list changed files. Select a file to see both sides, with a media preview you can slide between before and after and a table of properties that differ. Line-by-line changes are not shown yet.
- **Tags** name a version, such as a delivery. Tag the latest version, or delete a tag; only the name is removed.

### Undo menu

In Versions, the undo menu on a version offers:

- **Undo this version**: adds a new version that reverses it. Nothing is deleted. If it conflicts you can continue, skip or abandon the undo.
- **Move branch back**: moves the branch to that version. Keeping your files is the safe choice; the option that overwrites your working folder is separate and asks you to confirm that unsaved work will be lost.
- **Restore file**: overwrites one working file with its content at that version (its unsaved edits are lost).

### Inspector previews

Select a file in Files or Versions to see its details panel:

- a preview for images, video and audio, including files from past versions (the file is extracted to a temporary folder that is cleared when the app starts)
- media metadata for the file
- the file's lock state and its history

## Branches

- **Branches** lists all branches with how far each is ahead of or behind its remote. Create a branch from the version you are on, switch to it (progress shows while files update), rename it, or delete it.
- **Merge** brings another branch's work into the branch you are on. Choose the branch and what to do when files clash: blend (`recursive`), keep ours, or keep theirs. If files conflict, they are listed; for each one choose your version or theirs, then **Finish merge**. **Cancel merge** discards your choices and puts your files back as they were before the merge started.
- **Sync** shares work with your team through a remote (add or remove remotes here):

  | Button | Command | What it does |
  |--------|---------|--------------|
  | Fetch | `fetch` | Check what is new without changing your files |
  | Get latest | `pull` | Bring in your team's new versions |
  | Send | `push` | Share your saved versions |

  Ahead and behind counts show `--` until you fetch. When you add a remote with an access token, the token is kept in your computer's keychain, not in the repository's settings file. If this computer has no usable keychain the remote is not added and you get a `keychain_unavailable` error, rather than the token being saved as plain text.
- **Stash** parks unsaved changes so you can switch tasks. You can stash everything or only chosen files. Apply brings them back and keeps the entry; pop applies and removes it; drop removes one entry; remove all deletes every entry for good.

## Team

### Locks

Lock a file before you edit it so nobody else saves changes to it at the same time. The list can show your locks or everyone's. Release your own lock any time. **Force unlock** releases someone else's lock, which can lose their unsaved work, and needs permission to manage the project. Locks need a remote server.

### Remote files

Browse the folders and files of a remote at a chosen version without cloning. Select a file to preview it, or **Download to...** to save a single file; the download can be cancelled and does not wait behind other actions on your repository. To get everything, clone from Welcome. Add a remote in Branches, Sync first.

### Server

Run and manage a MediaGit server from the app. You can also point it at a server someone else runs. This section works without an open repository.

**Run a server on this computer**

1. Open Team, then Server, and choose **Set up local server**.
2. Pick the server folder. It is where the server keeps its data. It must be empty or already hold a server, and it must not be inside one of your projects. You can change the folder only while the server is stopped; **Use default** returns to the app's own folder.
3. Create the first admin account (username, email, password).
4. Under storage, choose where new repositories keep their files:
   - **This computer's folder** (the default).
   - **Amazon S3 or MinIO**: bucket, access key ID and secret key. For MinIO or another S3-compatible service also enter its endpoint address, for example `http://localhost:9000`. A folder inside the bucket is optional.
   - **Google Cloud**: bucket and project. Add a service account key file, or leave it empty to use the Google sign-in already on this computer.
   - **Azure Blob**: container, storage account and account key. Azure support is in preview.
5. Choose **Test connection** before you continue. The app writes a small test file to the bucket, reads it back and deletes it, then tells you whether the credentials work.
6. Choose **Set up and start**.

Secrets you type here travel to the server program on its standard input only, never on a command line or in a log.

The server listens on this computer only. Tick **Let other computers on my network connect** (while the server is stopped) to let others on your network use it; sign-in is always required. The Overview shows the address to share, what the server printed, and your own account. **Stop server** shuts it down cleanly; the app only ever stops a server that it started, and stops it when you quit.

**Other tabs**

- **People**: add people, change their role, reset a password, disable or enable an account.
- **Access**: give or remove access to a repository, and see who can reach a repository or what one person can reach.
- **API keys**: keys for scripts and CI.
- **Repositories**: create repositories, or delete one by typing its name to confirm. If the server stores data in a bucket, the dialog says up front that deleting a repository does not delete its data from the bucket.

To work with a server run by someone else, pick it from the server list or add it by address, then sign in. Managing people and repositories needs admin rights on that server.

## Settings

- **General**: theme (light, dark or follow the system), density, your author name and email, other repository settings, and sparse-checkout patterns. Each shows plain wording first; a "Not saved" marker appears until you save, and leaving with unsaved changes asks first.
- **Storage**: repository size and statistics: how much space your versions use and how much is saved by compression and deduplication.
- **Maintenance**: **Clean up** removes data no version needs; it asks for confirmation first because removed data cannot be recovered. **Check** looks for damaged or missing data. **Verify** runs a quick integrity check. Each shows a report and live progress, and if a check finds problems the report lists them.
- **Security**: sign in to and out of a remote server, manage encryption keys, and unlock an encrypted repository. Creating a key shows a one-time recovery code in a dialog that stays open until you choose **I saved it**; the code is not stored anywhere else. The recovery row reads **Saved** or **Not set up**.
- **Advanced tools**: **Rebase** and **Cherry-pick** replay versions onto the current branch (the cherry-pick picker lists versions from every branch; for conflicts choose your side or theirs per file, then continue, skip or abort; a pick that would change nothing is refused with an "empty commit" error). **Bisect** is a guided search for the version that introduced a problem. **Reflog** lists where a branch used to point; **Restore to here** moves the branch back through the same confirmation as the undo menu.

Many actions have a **More options** disclosure that exposes the rest of what the command line can do, in plain words with the flag name as small secondary text. Risky options sit behind an explicit confirmation.

### Unlocking encrypted repositories

When an action needs the passphrase of an encrypted repository, the app asks for it once and then runs the action. The passphrase is kept in memory for that repository for the rest of the session, so you are not asked again. It is sent to the `mediagit` program on its standard input only, never on the command line or in logs. Choose **Lock** in the sidebar or the Security section to forget it. Switching or closing the repository, or closing the app, also forgets it.

## Activity drawer

Long actions (clone, fetch, pull, push, adding files, switching branches) appear in the activity drawer with live progress bars, so you can keep working elsewhere.

## Installing

Release candidates are attached to the `desktop-v*` releases on GitHub:

| System | File |
|--------|------|
| Windows | NSIS installer (`.exe`) |
| macOS | disk image (`.dmg`) |
| Linux | `.deb` package and AppImage |

Each one bundles the `mediagit` and `mediagit-server` programs, so nothing else is needed. The app only uses the bundled `mediagit`; it never looks for one on your `PATH`. Opening a second copy brings the first window to the front instead.

**These builds are not code-signed yet**, so your system will warn the first time you run them. The warnings are expected for a release candidate:

- **Windows**: SmartScreen shows "Windows protected your PC". Choose **More info**, then **Run anyway**.
- **macOS**: Gatekeeper says it cannot verify the app. Open it with a right-click, **Open**, or allow it under System Settings, Privacy & Security.
- **Linux**: mark the AppImage as executable first (`chmod +x`), or install the `.deb` with your package manager.

Only install builds you downloaded from the project's GitHub releases.

## Cancelling and queueing

- **Cancel** is available for clone, fetch, pull, push and single-file downloads. It stops the operation immediately. A cancelled transfer leaves the repository intact; run it again to continue.
- Actions that change a repository run one at a time per repository, in the order you started them. A second action waits for the first. Read-only views keep working meanwhile.

## If something goes wrong

Errors appear with the message from the CLI. If the app cannot find the bundled `mediagit` program next to it, or the two were built at different versions, it says so on the first action (error `cli_not_found`). Run the same step in a terminal to see full output. The app also writes a log file in its log folder, including crashes, which is useful when reporting a problem.

Developers can point the app at a different build with the `MEDIAGIT_DESKTOP_CLI`, `MEDIAGIT_DESKTOP_SERVER` and `MEDIAGIT_DESKTOP_SERVER_DIR` environment variables; see `env-knobs.md`.
