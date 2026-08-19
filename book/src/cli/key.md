# mediagit key

Manage this repository's at-rest encryption key.

Encryption is **per repository** and **enabled at creation or not at all**. There is no global switch and no server setting that turns it on for you.

```bash
mediagit key <SUBCOMMAND>
```

## The one thing to know first

`mediagit key init` refuses on a repository that already holds objects.

Sealing what is already there would mean rewriting every object, and a repository that reports itself as encrypted while most of its contents are not cannot make the promise the word implies. So the decision belongs at the start:

```bash
mediagit init my-project
cd my-project
mediagit key init          # <- here, before the first commit
mediagit add .
mediagit commit -m "first"
```

Run `key init` after the first commit and it stops with an error, changes nothing, and tells you to start from a fresh repository. Encrypting an existing repository is not supported. Neither is turning encryption off, or replacing the repository key: all three would have to rewrite every object.

## Subcommands

### `init`

Generates this repository's encryption key, wraps it under a master key, and writes it to `.mediagit/encryption-key`.

```bash
mediagit key init
```

Prints a one-time **recovery code**. Write it down. It is the only way back in if the master key is lost.

The master key comes from the first available of: the file named by `MEDIAGIT_ENCRYPTION_KEYFILE`, the OS keychain, or a passphrase you are prompted for.

```bash
# Keep the master key on removable media instead of the OS keychain
MEDIAGIT_ENCRYPTION_KEYFILE=/media/usb/mediagit.key mediagit key init
```

### `status`

Reports whether this repository is encrypted and how it unlocks.

```bash
mediagit key status
```

### `recover`

Unlocks with the recovery code and re-wraps the key under this machine's master key. Use it when the master key is gone.

```bash
mediagit key recover
```

The code is prompted for if omitted, which is the better way to supply it — passing it as an argument leaves it in shell history.

### `rotate-master`

Re-locks the repository key under a **new master key**.

```bash
mediagit key rotate-master
mediagit key rotate-master --new-keyfile /media/usb/new.key
```

Use it after a lost or stolen laptop, to change a passphrase, or to move between the OS keychain and a key file.

This changes only what *protects* the key, not the key itself — so no object is rewritten and the existing recovery code keeps working.

`--new-keyfile` is needed when rotating from one key file to another: unwrapping the old master and choosing the new one both read `MEDIAGIT_ENCRYPTION_KEYFILE`, so the destination has to be named separately. Omit it to take the new master from the usual sources.

## Working with a remote

`push` escrows the repository key with the server the first time, so the server can serve objects it never sees in the clear. `clone` receives the key back and stores it under this machine's master key, so a clone of an encrypted repository just works.

MediaGit refuses, before anything is transferred, when the two sides disagree:

- pushing to a remote holding a **different** key
- pushing from an **unencrypted** repository to a remote that holds a key
- pulling sealed objects into a repository that has **no** key

## Losing your keys

If both the master key and the recovery code are lost, the objects cannot be recovered — not by MediaGit, and not by anyone else. That is what at-rest encryption means.

## Environment

| Variable | Effect |
| --- | --- |
| `MEDIAGIT_ENCRYPTION_KEYFILE` | Path to a file holding the master key, used instead of the OS keychain |
| `MEDIAGIT_NO_KEYRING` | Skip the OS keychain entirely |

## See also

- [Security architecture](../architecture/security.md)
