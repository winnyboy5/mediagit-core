# Desktop App

> **Preview** (targets v0.8.0). The desktop app is a window around the `mediagit` command line. Everything it does, you can also do with the CLI, and it works on the same repositories.

The app runs one `mediagit` command for each action, so results match the CLI exactly.

## Views

Pick a view from the activity bar on the left. A details panel on the right shows the selected file (Changes and History).

### Welcome

The first screen. Open a folder that is already a repository, create a new repository (`init`), or clone one from an `http(s)` address or a local path. Clone shows progress and can be cancelled. Recent repositories are listed so you can reopen them.

### Changes

Your working folder compared with the last saved version.

- Stage files with the checkboxes (`add`), or unstage them all.
- Write a message and choose **Save version** (`commit`).

### History

Saved versions, newest first, drawn as a commit graph with branch and tag labels. Select a version to see the files it changed, or browse every file as it was at that version.

### Inspector previews

Select a file in Changes or History to see its details panel:

- a preview for images, video and audio, including files from past versions (the file is extracted to a temporary folder that is cleared each time the app starts)
- media metadata for the file (`media info`)
- the file's lock state and its history

### Branches

All branches with how far each is ahead of or behind its remote. Create a branch from the version you are on, switch to it (progress shows while files update), rename it, or delete it. Tags are listed alongside.

### Locks

Lock a file before you edit it so nobody else saves changes to it at the same time. The list can show your locks or everyone's. Release your own lock any time. **Force unlock** releases someone else's lock, which can lose their unsaved work, and needs permission to manage the project. Locks need a remote server.

### Sync

Share work with your team through a remote (add or remove remotes here):

| Button | Command | What it does |
|--------|---------|--------------|
| Fetch | `fetch` | Check what is new without changing your files |
| Get latest | `pull` | Bring in your team's new versions |
| Send | `push` | Share your saved versions |

Ahead and behind counts show `--` until you fetch.

### Merge & conflicts

Bring another branch's work into the branch you are on. Choose the branch and what to do when files clash: blend (`recursive`), keep ours, or keep theirs. You can also force a merge version even when a fast-forward is possible. If files conflict, they are listed; for each one choose your version or theirs, then **Finish merge**. **Cancel merge** discards your choices and puts your files back as they were before the merge started.

### Compare

Pick two different versions (or compare your working folder with the latest version) to list the changed files. Select a file to see both sides: a media preview you can slide between before and after, and a table of properties that differ. Line-by-line changes are not shown yet.

### Tags & Stash

- **Tags** name a version, such as a delivery. Tag the latest version, or delete a tag; only the name is removed.
- **Stash** parks unsaved changes so you can switch tasks. Apply brings them back and keeps the entry; pop applies and removes it; drop removes one entry; remove all deletes every entry for good.

### Undo menu

In History, the undo menu on a version offers:

- **Undo this version**: adds a new version that reverses it. Nothing is deleted. If it conflicts you can continue, skip or abandon the undo.
- **Move branch back**: moves the branch to that version. Keeping your files is the safe choice; the option that overwrites your working folder is separate and asks you to confirm that unsaved work will be lost.
- **Restore file**: overwrites one working file with its content at that version (its unsaved edits are lost).

### Remote Browser

Browse the folders and files of a remote at a chosen version without cloning. Select a file to preview it, or **Download to...** to save a single file; the download can be cancelled and does not wait behind other actions on your repository. To get everything, clone from Welcome. Add a remote on the Sync page first.

### Activity bar and drawer

Long actions (clone, fetch, pull, push, switching branches) appear in the activity drawer with live progress bars, so you can keep working in another view.

## Cancelling and queueing

- **Cancel** is available for clone, fetch, pull, push and single-file downloads. It stops the operation immediately. A cancelled transfer leaves the repository intact; run it again to continue.
- Actions that change a repository run one at a time per repository, in the order you started them. A second action waits for the first. Read-only views keep working meanwhile.

## If something goes wrong

Errors appear with the message from the CLI. If the app cannot find the `mediagit` program next to it, or the two were built at different versions, it says so on the first action. Run the same step in a terminal to see full output.
