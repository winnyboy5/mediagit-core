# Configuration

MediaGit configuration reference.

## Repository Configuration

Located in `.mediagit/config`:

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
