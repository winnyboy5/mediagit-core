# Configuration

MediaGit configuration reference.

## Credential Resolution

When connecting to a remote server, MediaGit resolves credentials in this order:

```mermaid
flowchart TD
    A["Remote command<br/>push/pull/clone"] --> B{"Check<br/>MEDIAGIT_TOKEN<br/>environment?"}
    B -->|Found| C["Use as<br/>Bearer token"]
    B -->|Not found| D{"Check<br/>MEDIAGIT_API_KEY<br/>environment?"}
    D -->|Found| E["Use as<br/>API key"]
    D -->|Not found| F{"Check<br/>OS keychain<br/>for remote?"}
    F -->|Found| G["Use cached<br/>credential"]
    F -->|Not found| H{"Check config.toml<br/>remotes.name<br/>token/api_key?"}
    H -->|Found| I["Use from config"]
    H -->|Not found| J["No credentials<br/>401 if protected"]
    
    C --> K["Send request"]
    E --> K
    G --> K
    I --> K
    J --> K
    
    style A fill:#e3f2fd
    style C fill:#c8e6c9
    style E fill:#c8e6c9
    style G fill:#c8e6c9
    style I fill:#c8e6c9
    style J fill:#ffcdd2
```

The moment any credential succeeds, it's written through to the OS keychain so the next invocation resolves faster.

## Repository Configuration

Located in `.mediagit/config.toml`:

```toml
[storage]
backend = "filesystem"
base_path = "./data"

[compression]
algorithm = "zstd"
level = 3

[author]
name = "Your Name"
email = "your.email@example.com"
```

See [Configuration Reference](./reference/config.md) for all options.
