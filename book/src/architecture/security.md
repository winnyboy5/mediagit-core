# Security

MediaGit security model and best practices.

## Authentication
- **Local**: File system permissions
- **Server mode**: JWT tokens + API key authentication
- **Cloud**: IAM roles, service principals, API keys

## Data Integrity
- SHA-256 hashing for all objects
- Cryptographic verification on read
- `mediagit verify` for repository health

## Encryption
- **At-rest (client-side)**: AES-256-GCM with Argon2id key derivation
- **At-rest (cloud)**: Cloud provider encryption (SSE-S3, Azure SSE)
- **In-transit**: TLS 1.3 for network operations

## Best Practices
1. Use IAM roles (avoid hardcoded keys)
2. Enable bucket versioning
3. Regular `mediagit verify` checks
4. Restrict branch protection rules
5. Audit logs for sensitive repositories

See [Configuration Reference](../reference/config.md) for security settings.
