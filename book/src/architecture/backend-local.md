# Local Storage Backend

Local file system storage for development and small teams.

## Configuration

```toml
[storage]
backend = "filesystem"
base_path = "./data"
create_dirs = true
```

## Usage

```bash
mediagit init
# Automatically uses local backend
```

## Performance
- **Read**: Direct file system access
- **Write**: Direct file system writes
- **Latency**: <1ms
- **Throughput**: Limited by disk I/O

## Best For
- Development
- Single machine workflows
- Small teams with network shares
