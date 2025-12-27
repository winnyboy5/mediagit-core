# Security

MediaGit security model and best practices.

## Authentication
- **Local**: File system permissions
- **Cloud**: IAM roles, service principals, API keys

## Data Integrity
- SHA-256 hashing for all objects
- Cryptographic verification on read
- `mediagit verify` for repository health

## Encryption
- **At-rest**: Cloud provider encryption (SSE)
- **In-transit**: TLS 1.2+ for network operations
- **Future**: Client-side encryption support

## Best Practices
1. Use IAM roles (avoid hardcoded keys)
2. Enable bucket versioning
3. Regular `mediagit verify` checks
4. Restrict branch protection rules
5. Audit logs for sensitive repositories

See [Configuration Reference](../reference/config.md) for security settings.
