# Working with Remote Repositories

Guide to remote repository workflows.

## Remote Workflow

```mermaid
flowchart LR
    A["mediagit clone<br/>remote-url<br/>Get full history"] --> B["Local repo"]
    B --> C["mediagit add &<br/>mediagit commit<br/>Make changes"]
    C --> D["mediagit push<br/>origin main<br/>Send commits<br/>& chunks"]
    D --> E["Remote repo<br/>updated"]
    E --> F["Other users<br/>pull your work"]
    F --> G["mediagit pull<br/>origin main<br/>Fetch & merge"]
    G --> B
    
    style A fill:#e3f2fd
    style D fill:#fff3e0
    style G fill:#fff3e0
    style E fill:#c8e6c9
```

## Setup

Add a remote to an existing repo:

```bash
mediagit remote add origin https://host:3000/my-repo
mediagit remote add backup s3://bucket/backup-repo
```

List configured remotes:

```bash
mediagit remote list
```

## Push/Pull

Push your commits to a remote:

```bash
# Push current branch
mediagit push origin main

# Push all branches
mediagit push origin
```

Pull updates from a remote (fetch + merge):

```bash
mediagit pull origin main
```

Or fetch without merging, then merge manually:

```bash
mediagit fetch origin
mediagit merge origin/main
```

## Collaboration