# Branching Strategies

How to use branches in a media project. Everything here uses commands that
exist today; each section links to the command reference.

The one rule that shapes all of it: **binary media cannot be merged line by
line.** When two branches change the same `.psd`, `.blend` or video file, a
merge keeps one side and you redo the other side's work by hand. So a good
strategy for media is less about how branches are named and more about making
sure two people never edit the same binary file in parallel.

## Feature branches (recommended default)

Keep `main` as the version everyone builds on. Do each piece of work on its
own short-lived branch and merge it back when it is done.

```bash
mediagit branch create shot-042-relight      # start from the current version
mediagit branch switch shot-042-relight
# ... edit, then:
mediagit add renders/shot_042/
mediagit commit -m "Relight shot 042"

mediagit branch switch main
mediagit pull                                # get everyone else's work first
mediagit merge shot-042-relight
mediagit push
mediagit branch delete shot-042-relight
```

Keep feature branches short. The longer a branch lives, the more likely
someone else changes the same binary file on `main`, and the more work a
binary conflict throws away.

See [branch](../cli/branch.md), [merge](../cli/merge.md),
[push](../cli/push.md).

## Lock binary files before you edit them

Locks are what make parallel work on binary media safe. Lock a file before
you start editing it; release the lock when your change is pushed.

```bash
mediagit lock create characters/hero.blend
# ... edit, commit, push ...
mediagit lock unlock characters/hero.blend
mediagit lock list                           # who is editing what
```

Locks live on the server. With enforcement on (the server default), a push
that touches a file someone else has locked is rejected, on every branch. A
lock does not stop anyone editing their local copy, so check `lock list` (or
the Locks view in the desktop app) before you start.

See [lock](../cli/lock.md) for how enforcement works and when it is skipped.

## Protect `main`

```bash
mediagit branch protect main
```

A protected branch cannot be deleted or renamed over with `branch delete` /
`branch rename`. Protection is recorded in the repository config
(`[protected_branches]`). It does not currently block a force push, and it
does not require reviews; use locks and agreed habits for those.

## Merge or rebase?

- **Merge** (`mediagit merge <branch>`) keeps both histories and adds a merge
  commit. Use it to bring a finished feature branch into `main`.
- **Rebase** (`mediagit rebase main`) replays your branch's commits on top of
  `main`, giving a straight history. Use it on your own branch before merging,
  never on a branch other people have pulled.
  `mediagit rebase main feature` rebases `feature` without switching to it
  first.

When either one hits a binary conflict, one side is written to the working
folder unmarked. Replace it with the version you want, `mediagit add` it, and
continue (`merge --continue` / `rebase --continue`).

See [merge](../cli/merge.md), [rebase](../cli/rebase.md).

## Release branches

When a delivery needs to freeze while work continues, branch from the version
you deliver and tag it:

```bash
mediagit branch create release-2026-12 <commit>
mediagit tag create v1.0 <commit>
```

Fix problems on the release branch, then copy each fix back to `main` without
switching branches:

```bash
mediagit cherry-pick --onto main <fix-commit>
```

`--onto` refuses instead of writing conflict markers if the fix does not apply
cleanly; switch to `main` and run a normal `cherry-pick` to resolve it.

See [tag](../cli/tag.md), [cherry-pick](../cli/cherry-pick.md).

## Git Flow

Git Flow (separate `develop`, `release/*` and `hotfix/*` branches) works with
the same commands, but its long-lived parallel branches make binary conflicts
more likely. For media projects, prefer feature branches off `main` plus
release branches only when a delivery has to freeze.
