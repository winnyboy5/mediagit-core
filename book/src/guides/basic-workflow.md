# Basic Workflow

See [Quickstart Guide](../quickstart.md) for complete installation and setup instructions. This guide covers the core commit workflow.

## The Commit Cycle

```mermaid
flowchart LR
    A["mediagit init<br/>Initialize repo"] --> B["mediagit add files<br/>Stage changes"]
    B --> C["mediagit status<br/>Review staged"]
    C --> D{"Changes<br/>correct?"}
    D -->|No| E["mediagit reset<br/>Unstage & adjust"]
    E --> B
    D -->|Yes| F["mediagit commit<br/>Create commit"]
    F --> G["mediagit log<br/>View history"]
    G --> H["Start next cycle"]
    
    style A fill:#e3f2fd
    style F fill:#c8e6c9
    style G fill:#fff9c4
    style H fill:#f3e5f5
```

## Typical Session

```bash
# Start a session (once per repo)
mediagit init my-project
cd my-project

# Add or modify files
echo "Hello" > greeting.txt
mediagit add greeting.txt

# Check what you staged
mediagit status

# Commit your work
mediagit commit -m "Add greeting file"

# View your commits
mediagit log
```

For team collaboration using remotes, see [Working with Remote Repositories](./remote-repos.md).