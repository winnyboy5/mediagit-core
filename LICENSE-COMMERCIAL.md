# Commercial Licensing

MediaGit is **source available** under the [Business Source License 1.1](LICENSE).
It is not open source. This page explains where the line falls, so you can tell
in a minute whether you need to talk to us.

## Free for small organisations

The Additional Use Grant in `LICENSE` permits **production use by Small
Organisations**: organisations that, together with their affiliates, have
**fewer than 250 people** (employees plus full-time contractors) **and less than
USD 10 million** in annual revenue. If that is you, all of the following are free:

- Running MediaGit to version **your own** media: films, games, campaigns,
  archives, whatever you make.
- Self-hosting `mediagit-server` on your own infrastructure or your own cloud
  account, for your own team.
- Commercial work of any kind.
- Building tools, plugins and integrations on top of it, and selling them.

**Free for everyone, whatever your size:**

- Reading, modifying, forking and redistributing the source.
- Non-production use of every kind: evaluation, pilots, development, testing,
  CI, research and teaching.
- Consulting, deploying, configuring or supporting MediaGit for a client, where
  the client runs their own instance. The client needs its own licence if it is
  not a Small Organisation.

If you are an indie or mid-size studio, a startup or a freelancer using MediaGit
for your own work, you are covered. You owe nothing and need to ask no one.

## When you need a commercial licence

You need a commercial licence in two cases.

**1. Your organisation is not a Small Organisation.** That means 250 or more
people, or USD 10 million or more in annual revenue, counted together with
your affiliates. Production use then needs an **Enterprise licence**: an
annual contract that includes the enterprise add-on module and an SLA.

**2. You offer MediaGit as a service to others,** whatever your size:

> Offering the Licensed Work, **or a service whose primary value derives from the
> Licensed Work**, to third parties as a **hosted, managed, or
> software-as-a-service offering**.

In plain terms, this covers:

- Running MediaGit **as a service for other people**: a hosted "MediaGit
  cloud", a managed offering, or MediaGit operated on behalf of someone outside
  your own organisation.
- Selling a product whose **primary value comes from MediaGit**: wrapping the
  versioning, dedup or delta engine in a thin layer and selling access to it.

If you are unsure which side of the line you are on, ask. It costs nothing to
check.

## What we sell

- **Studio support:** priority help, onboarding and an SLA, for teams of any
  size.
- **Enterprise licence:** for organisations above the Small Organisation
  threshold. It includes the enterprise add-on module (SSO/SAML, fine-grained
  RBAC, audit logging and compliance reporting) and an SLA.
- **MediaGit Cloud:** hosted, managed and backed up, with no infrastructure to
  run.
- **Commercial licences** for the hosted and embedded use reserved above.

## Which versions this applies to

The Small Organisation threshold applies to every release published after
`v0.4.0-rc.1`. Releases up to and including `v0.4.0-rc.1` keep the grant they
were published under, which permitted production use at any scale. Releases
`v0.1.0` to `v0.2.8-beta.1` remain under AGPL-3.0-or-later.

## Eventual open source

Every released version converts to the **Change License, AGPL-3.0-or-later,
four years after that version is published**, automatically, under the terms of
`LICENSE`. Each version carries its own clock. Nothing is withheld permanently.

## Contact

**licensing@mediagit.dev**

Please include what you are building, roughly how you intend to deploy MediaGit,
your organisation's size, and who the end users are. That is normally enough to
answer the licensing question in one reply.

---

*This page is a plain-language summary provided for convenience. It is not the
licence and it is not legal advice. Where this summary and [`LICENSE`](LICENSE)
disagree, `LICENSE` governs.*
