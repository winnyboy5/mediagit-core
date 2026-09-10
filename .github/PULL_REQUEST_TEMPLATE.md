## What this changes

<!-- What does this PR do, and why? Link an issue if there is one. -->

## How it was verified

<!-- Not "tests pass" -- say what you actually ran and what it proved.
     If you added a guard, say how you confirmed it FIRES, not just that it
     stays quiet. A guard proven in only one direction is untested. -->

## Checklist

- [ ] I have read and agree to the [Contributor License Agreement](../CLA.md)
- [ ] New `.rs` files start with the two-line SPDX header:
      `// SPDX-License-Identifier: BUSL-1.1` / `// Copyright (C) 2025-2026 Aswin Krishnamoorthy`
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` are clean
- [ ] `cargo test --workspace --all-features` passes
- [ ] Docs updated if behaviour or flags changed
- [ ] `CHANGELOG.md` updated if this is user-visible

## Licensing

By submitting this pull request you confirm that your contribution is your own
work (or you have the right to submit it) and you agree to license it under the
terms of [`CLA.md`](../CLA.md).

**Why we ask:** MediaGit is offered under both [BUSL-1.1](../LICENSE) and a
[commercial licence](../LICENSE-COMMERCIAL.md). Without the CLA, contributed code
could never appear in a commercially licensed build — permanently.
