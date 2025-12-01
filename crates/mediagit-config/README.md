# MediaGit Configuration Management System

A comprehensive configuration management system for MediaGit Core with support for multiple file formats, environment variable overrides, validation, and migration capabilities.

## Features

- **Multi-Format Support**: Load configuration from TOML, YAML, or JSON files
- **Environment Variable Overrides**: Override any configuration value using environment variables with `MEDIAGIT_` prefix
- **Comprehensive Validation**: Detailed error messages for invalid configurations
- **Configuration Migration**: Framework for handling schema version updates
- **Flexible Storage Backends**: Support for filesystem, AWS S3, Azure Blob, Google Cloud Storage, and multi-backend configurations
- **Performance Tuning**: Cache, connection pool, and timeout configurations
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

### Loading with Environment Variable Overrides

```rust
let config = loader.load_with_overrides("config.toml").await?;

// Environment variables with MEDIAGIT_ prefix will override file settings:
// export MEDIAGIT_APP_PORT=9000
// export MEDIAGIT_LOG_LEVEL=debug
```

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
[compression]   # Compression settings
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
encryption = true
encryption_algorithm = "AES256"  # or "aws:kms"
```

Credentials can be provided via environment variables:
- `MEDIAGIT_S3_ACCESS_KEY_ID`
- `MEDIAGIT_S3_SECRET_ACCESS_KEY`

### Azure Blob Storage

```toml
[storage]
backend = "azure"
account_name = "mystorageaccount"
container = "media"
prefix = "files/"
```

Credentials via environment variables:
- `MEDIAGIT_AZURE_ACCOUNT_KEY`

### Google Cloud Storage

```toml
[storage]
backend = "gcs"
bucket = "my-bucket"
project_id = "my-project"
credentials_path = "/path/to/credentials.json"
```

Or use environment variable:
- `MEDIAGIT_GCS_CREDENTIALS_PATH`

## Compression Configuration

```toml
[compression]
enabled = true
algorithm = "zstd"          # zstd, brotli, or none
level = 3                   # zstd: 1-22, brotli: 0-11
min_size = 1024             # minimum file size to compress

[compression.algorithms.zstd]
level = 3

[compression.algorithms.brotli]
level = 4
```

## Performance Configuration

```toml
[performance]
max_concurrency = 4         # CPU cores or explicit number
buffer_size = 65536         # 64KB

[performance.cache]
enabled = true
cache_type = "memory"       # memory, disk, redis
max_size = 536870912        # 512MB
ttl = 3600                  # 1 hour

[performance.connection_pool]
min_connections = 1
max_connections = 10
timeout = 30                # seconds

[performance.timeouts]
request = 60                # seconds
read = 30
write = 30
connection = 30
```

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
encryption_at_rest = false

[security.rate_limiting]
enabled = false
requests_per_second = 100
burst_size = 200
```

## Environment Variable Overrides

All configuration values can be overridden using environment variables with the `MEDIAGIT_` prefix.

### Common Overrides

```bash
# App Configuration
export MEDIAGIT_APP_NAME="my-app"
export MEDIAGIT_APP_PORT=9000
export MEDIAGIT_APP_HOST="0.0.0.0"
export MEDIAGIT_APP_ENVIRONMENT="production"
export MEDIAGIT_APP_DEBUG=false

# Observability
export MEDIAGIT_LOG_LEVEL=debug
export MEDIAGIT_METRICS_ENABLED=true
export MEDIAGIT_METRICS_PORT=9091

# Compression
export MEDIAGIT_COMPRESSION_ENABLED=true
export MEDIAGIT_COMPRESSION_LEVEL=5

# Performance
export MEDIAGIT_MAX_CONCURRENCY=8
export MEDIAGIT_BUFFER_SIZE=131072

# Security
export MEDIAGIT_API_KEY="secret-key"
export MEDIAGIT_HTTPS_ENABLED=true
export MEDIAGIT_AUTH_ENABLED=true
```

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
- **Compression Level**:
  - Zstd: 1-22
  - Brotli: 0-11
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

- **v0 → v1**: Adds default metrics configuration and compression algorithm

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

### With mediagit-compression

```rust
use mediagit_config::Config;
use mediagit_compression::CompressionEngine;

let config = loader.load_file("config.toml").await?;
let engine = CompressionEngine::new(
    config.compression.algorithm,
    config.compression.level
)?;
```

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
2. **Use Environment Variables in Production**: Override sensitive values via environment
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

AGPL-3.0 - See LICENSE file for details
