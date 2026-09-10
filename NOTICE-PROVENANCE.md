# Code Provenance

This statement records where MediaGit's code came from. It exists because
intellectual-property due diligence routinely asks the question, and answering it
in writing beforehand is cheaper than reconstructing it later.

## Copyright and chain of title

**All copyright in MediaGit is held by Aswin Krishnamoorthy.**

As of 2026-08-27 the repository contains commits from a single human author,
appearing under two spellings of the same name (`winnyboy5` and
`Aswin Krishnamoorthy`) on one email address. There are **no third-party
contributors**, and no code has been accepted from anyone else.

This unbroken chain of title is what makes the dual BUSL-1.1 / commercial
licensing model legally possible. It is also fragile: accepting a single outside
contribution without an agreement in place would permanently fragment it. That is
why [`CLA.md`](CLA.md) is required before any external pull request is merged, and
why the requirement is enforced by CI rather than by convention.

## AI-assisted development

Parts of MediaGit were written with AI assistance, and some commits are attributed
to `Claude <noreply@anthropic.com>` in the git history (15 as of 2026-08-27).

That attribution is accurate and deliberately preserved rather than rewritten. The
substance of it:

- **All AI-assisted work was directed, reviewed and accepted by the copyright
  holder.** No code entered the repository without human review.
- **Anthropic's Commercial Terms of Service assign ownership of outputs to the
  customer.** Anthropic asserts no rights in the resulting code.
- The AI attribution reflects *how* a change was produced, not who holds rights in
  it. Rewriting that history to hide it would make the record less accurate, not
  more, and the old commit hashes would remain discoverable regardless.

## Third-party code

MediaGit depends on 648 third-party packages, all under permissive or weak-copyleft
licences. These are catalogued with their licences in
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).

Two points a reviewer will want:

- **No GPL, AGPL, SSPL, EUPL, CDDL or CC-BY-SA dependency exists anywhere in the
  tree.** Nothing in the dependency graph imposes copyleft obligations on
  MediaGit's own code.
- **17 MPL-2.0 crates** (the `symphonia` audio family and `mp4parse`) ship inside
  the `mediagit` binary via `mediagit-media`. MPL-2.0 is file-level copyleft:
  those files remain under MPL-2.0 and their source must remain available, but the
  obligation does not extend to MediaGit's own code.

Dependency licences are enforced automatically in CI by `cargo-deny` against the
allowlist in [`deny.toml`](deny.toml); a dependency introducing a disallowed
licence fails the build rather than being discovered later.

## Licence history

- `v0.1.0` through `v0.2.8-beta.1` were published under **AGPL-3.0-or-later** while
  the repository was public. That grant is irrevocable: those versions remain
  available under AGPL to anyone who obtained them, permanently.
- From the relicensing commit onward, MediaGit is **BUSL-1.1** with a Change
  Licence of AGPL-3.0-or-later. See [`CHANGELOG.md`](CHANGELOG.md).

---

*Last reviewed: 2026-08-27. Update this file when contributors are added, when the
dependency licence profile changes materially, or on any further licence change.*
