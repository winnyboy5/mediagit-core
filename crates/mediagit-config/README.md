# MediaGit Configuration Management System

A comprehensive configuration management system for MediaGit Core with support for multiple file formats, environment variable overrides, validation, and migration capabilities.

## Features

- **Multi-Format Support**: Load configuration from TOML, YAML, or JSON files
- **Comprehensive Validation**: Detailed error messages for invalid configurations
- **Configuration Migration**: Framework for handling schema version updates
- **Flexible Storage Backends**: Support for filesystem, AWS S3, Azure Blob, Google Cloud Storage, and multi-backend configurations
- **Performance Tuning**: Cache and concurrency configurations
- **Observability**: Logging, metrics, and tracing configurations
- **Security Settings**: TLS, encryption, CORS, rate limiting, and authentication options

## Quick Start

### Loading Configuration from a File

```rust
use mediagit_config::ConfigLoader;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let loader = ConfigLoader::new();
    let config = loader.load_file("config.toml").await?;

    println!("App: {}", config.app.name);
    println!("Port: {}", config.app.port);

    Ok(())
}
```

There is no environment-variable overlay on top of a loaded file. A
`load_with_overrides` method used to exist for this, but it had no caller
outside this crate's own tests and was removed in v0.4.0 along with the
fourteen `MEDIAGIT_*` variables it alone read — see
[`env-knobs.md`](../../env-knobs.md#server-app-config-overrides--removed) at
the repo root. Use `load_file` (above) for both the client and the server.

### Loading from String

```rust
let json_config = r#"
{
  "app": {
    "name": "mediagit",
    "port": 8080,
    "host": "0.0.0.0",
    "environment": "production"
  }
}
"#;

let config = loader.load_from_string(json_config, ConfigFormat::Json)?;
```

### Merging Multiple Configuration Files

```rust
let config = loader.load_and_merge(&[
    "config/base.toml",
    "config/production.toml"
]).await?;

// Later files override earlier ones
```

## Configuration Structure

### Top-Level Sections

```toml
[app]           # Application metadata
[storage]       # Storage backend configuration
[performance]   # Performance tuning
[observability] # Logging, metrics, tracing
[security]      # TLS, encryption, authentication
[custom]        # Custom application-specific settings
```

## Storage Backends

### Filesystem (Default)

```toml
[storage]
backend = "filesystem"
base_path = "./data"
create_dirs = true
sync = false
file_permissions = "0644"
```

### AWS S3

```toml
[storage]
backend = "s3"
bucket = "my-bucket"
region = "us-east-1"
prefix = "media/"
access_key_id = "AKIA..."
secret_access_key = "..."
```

Credentials come from this file only. There are no `MEDIAGIT_S3_*` variables,
and no IAM-role fallback: the backend rejects an empty access key. (`S3Storage`
also has no `encryption` / `encryption_algorithm` fields — earlier revisions of
this README showed them, but the client config is not `deny_unknown_fields`, so
they parse silently and do nothing.)

### Azure Blob Storage

```toml
[storage]
backend = "azure"
container = "media"
prefix = "files/"
auth = { type = "account_key", account_name = "mystorageaccount", account_key = "..." }
```

Credentials are a tagged `auth` block (`config_version` 3+): one of
`account_key`, `connection_string { value }`, `sas { account_name, token }`,
or `emulator` (local Azurite). Pre-v3 flat configs are migrated automatically
on first open. The credential comes from this block only — no MediaGit code
path reads `AZURE_STORAGE_KEY` or `AZURE_STORAGE_ACCOUNT`.

### Google Cloud Storage

```toml
[storage]
backend = "gcs"
bucket = "my-bucket"
project_id = "my-project"
credentials_path = "/path/to/credentials.json"
```

Leave `credentials_path` unset to use **Application Default Credentials**,
which resolve `GOOGLE_APPLICATION_CREDENTIALS` from the environment. That is
the only environment path any backend has. There is no
`MEDIAGIT_GCS_CREDENTIALS_PATH`.

There is no `[compression]` section as of v0.4.0 — it was removed from the
schema as a dead knob. `SmartCompressor` (in `mediagit-versioning`) chooses
algorithm and level automatically per file type; there is no config surface
for it.

## Performance Configuration

```toml
[performance]
buffer_size = 65536         # 64KB

[performance.cache]
enabled = true
cache_type = "memory"       # memory, disk, redis
max_size = 536870912        # 512MB
ttl = 3600                  # 1 hour
```

`upload_concurrency`, `download_concurrency` and `pack_workers` are also live
`[performance]` keys (each falls back to a `MEDIAGIT_*` env var, then an
internal default — see `env-knobs.md`). `max_concurrency`,
`[performance.connection_pool]` and `[performance.timeouts]` were removed
from the schema in v0.4.0 as dead knobs with no read site outside this crate.

## Observability Configuration

```toml
[observability]
log_level = "info"          # debug, info, warn, error, trace
log_format = "json"         # json or text
tracing_enabled = true
sample_rate = 0.1           # 10% of traces

[observability.metrics]
enabled = true
port = 9090
endpoint = "/metrics"
interval = 60               # seconds
```

## Security Configuration

```toml
[security]
https_enabled = false
tls_cert_path = "/path/to/cert.pem"
tls_key_path = "/path/to/key.pem"
api_key = "your-secret-key"      # or use MEDIAGIT_API_KEY
auth_enabled = false
cors_origins = ["http://localhost:3000"]

[security.rate_limiting]
enabled = false
requests_per_second = 1000
burst_size = 2000
```

## Environment Variable Overrides

There is no environment-variable overlay in this crate as of v0.4.0. It used
to expose `load_with_overrides()`, which layered `MEDIAGIT_*` variables onto a
parsed config via `apply_env_overrides()`, but that method had no caller in
the workspace outside this crate's own tests, and the real config path —
`Config::load()` (`schema.rs:174`) for the client, `ServerConfig` for the
server — parsed TOML directly and never applied the overlay. Both methods and
the fourteen variables they alone read were removed in v0.4.0 rather than
wired up (FUTURE_TODOS item 22). All removed, no such variables now:
`MEDIAGIT_APP_*` / `MEDIAGIT_COMPRESSION_*` / `MEDIAGIT_METRICS_*` /
`MEDIAGIT_LOG_LEVEL` — removed — `MEDIAGIT_MAX_CONCURRENCY` /
`MEDIAGIT_BUFFER_SIZE` / `MEDIAGIT_HTTPS_ENABLED` / `MEDIAGIT_AUTH_ENABLED`,
none of which exist.

Two variables read by that same now-deleted function are live via other, real
read sites, and are the only ones worth setting:

- **`MEDIAGIT_API_KEY`** — read directly at `mediagit-cli/src/repo.rs:187`;
  supplies the auth token for push/pull.
- **`MEDIAGIT_CHUNK_WRITE_CONCURRENCY`** — read directly at
  `mediagit-versioning/src/odb/chunks.rs:646`; sets chunk-write parallelism.

For the full catalogue of variables that actually work, see `env-knobs.md` and
`CONFIGURATION.md` at the repo root.

## Validation

The configuration system validates all settings automatically. Invalid configurations will produce detailed error messages:

```rust
use mediagit_config::Validator;

let config = loader.load_file("config.toml").await?;
config.validate()?;  // Validates all settings
```

### Validation Rules

- **App Port**: Must be between 1 and 65535
- **Environment**: Must be one of: development, staging, production
- **Cache Type**: Must be one of: memory, disk, redis
- **Log Level**: Must be one of: debug, info, warn, error, trace
- **S3 Bucket**: 3-63 characters, lowercase letters, digits, hyphens, dots
- **Azure Container**: 3-63 characters
- **Metrics Port**: Valid port number (1-65535)
- **Sample Rate**: Between 0.0 and 1.0
- **TLS Certificates**: Files must exist if HTTPS is enabled
- **Encryption Keys**: Files must exist if encryption is enabled

## Configuration Migration

The configuration system supports schema version upgrades through migrations:

```rust
use mediagit_config::{MigrationManager, MigrationV0ToV1};

let mut manager = MigrationManager::new();
manager.register(Box::new(MigrationV0ToV1));

let config_json = serde_json::to_value(&old_config)?;
let migrated = manager.migrate(config_json, 0, 1)?;
let new_config: Config = serde_json::from_value(migrated)?;
```

### Available Migrations

- **v0 → v1**: Adds default metrics configuration

## Example Files

The package includes three example configuration files:

- `examples/config.toml` - TOML format with all options
- `examples/config.yaml` - YAML format with all options
- `examples/config.json` - JSON format with all options

## Testing

The configuration system includes comprehensive tests:

```bash
# Run all tests
cargo test --package mediagit-config

# Run unit tests only
cargo test --package mediagit-config --lib

# Run integration tests only
cargo test --package mediagit-config --test integration_tests
```

### Test Coverage

- Format detection and parsing (TOML, YAML, JSON)
- File loading and error handling
- Configuration merging
- Environment variable overrides
- Validation of all configuration sections
- Serialization roundtrips
- Migration framework

## Integration with Other Crates

The configuration system is designed to integrate seamlessly with other MediaGit crates:

### With mediagit-storage

```rust
use mediagit_config::StorageConfig;
use mediagit_storage::StorageBackend;

let storage = match &config.storage {
    StorageConfig::FileSystem(fs_config) => {
        StorageBackend::filesystem(&fs_config.base_path)
    }
    StorageConfig::S3(s3_config) => {
        StorageBackend::s3(s3_config)
    }
    // ... other backends
};
```

## Custom Configuration

Additional application-specific configuration can be added to the `custom` section:

```toml
[custom]
feature_flag = "enabled"
custom_timeout = 120
my_setting = "value"
```

Access via:

```rust
if let Some(value) = config.custom.get("feature_flag") {
    println!("Feature flag: {}", value);
}
```

## Performance Considerations

- Configuration is loaded once at startup
- Validation is performed during loading (can be disabled for performance)
- Environment variable overrides are applied lazily
- Configuration is immutable after loading

## Error Handling

The configuration system provides detailed error types:

```rust
use mediagit_config::ConfigError;

match loader.load_file("config.toml").await {
    Ok(config) => { /* use config */ }
    Err(ConfigError::FileNotFound(path)) => {
        eprintln!("Configuration file not found: {}", path.display());
    }
    Err(ConfigError::ValidationError(msg)) => {
        eprintln!("Configuration validation failed: {}", msg);
    }
    Err(e) => {
        eprintln!("Configuration error: {}", e);
    }
}
```

## Best Practices

1. **Version Configuration Files**: Keep configuration in version control
2. **Keep Secrets Out of Version Control**: credentials live in the repo's own `.mediagit/config.toml`, which is not a file to commit. (Overriding them via environment does not work — see the note above.)
3. **Validate on Startup**: Always validate configuration after loading
4. **Provide Example Files**: Include example configurations in documentation
5. **Document Custom Settings**: Document any custom configuration your application adds
6. **Use Defaults Wisely**: Ensure defaults are safe for development
7. **Log Configuration on Startup**: Aid debugging by logging loaded configuration (without secrets)

## Contributing

To extend the configuration system:

1. Add new configuration fields to the schema in `schema.rs`
2. Implement validation in `validation.rs` if needed
3. Add tests in `tests/integration_tests.rs`
4. Update example files and documentation
5. Create a migration if the change affects existing configurations

## License

BUSL-1.1 (Business Source License 1.1) - See the LICENSE file at the repository root for details
