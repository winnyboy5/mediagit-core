# MediaGit Cloud Architecture Reference

> **Production deployment guide for cloud-native MediaGit infrastructure**

---

## Reference Architecture

```mermaid
flowchart TB
    subgraph Clients["🖥️ Clients"]
        CLI1[mediagit CLI]
        CLI2[mediagit CLI]
        CI[CI/CD Pipeline]
    end
    
    subgraph LoadBalancer["⚖️ Load Balancer"]
        LB[NGINX / ALB / Cloud LB]
    end
    
    subgraph Servers["🌐 MediaGit Servers"]
        S1[mediagit-server:3000]
        S2[mediagit-server:3000]
        S3[mediagit-server:3000]
    end
    
    subgraph Storage["💾 Primary Storage"]
        BUCKET[(Cloud Object Storage)]
    end
    
    subgraph Replicated["🔄 DR Region"]
        REPLICA[(Replicated Storage)]
    end
    
    subgraph Monitoring["📊 Observability"]
        PROM[Prometheus]
        GRAF[Grafana]
    end
    
    Clients --> LB
    LB --> Servers
    Servers --> BUCKET
    BUCKET -.->|Replication| REPLICA
    Servers --> PROM
    PROM --> GRAF
```

---

## Storage Backends

### AWS S3

```mermaid
flowchart LR
    subgraph Config["Configuration"]
        CREDS[AWS Credentials]
        REGION[Region]
        BUCKET[Bucket Name]
    end
    
    subgraph Features["Features"]
        MULTI[Multipart Upload<br/>100MB parts]
        RETRY[Exponential Backoff<br/>3 retries]
        CONC[8 Concurrent Parts]
    end
    
    subgraph Security["Security"]
        SSE[SSE-S3 / SSE-KMS]
        SSEC[SSE-C Optional]
        IAM[IAM Policies]
    end
    
    Config --> Features --> Security
```

| Setting | Default | Description |
|---------|---------|-------------|
| `bucket` | Required | S3 bucket name |
| `region` | Auto-detect | AWS region |
| `endpoint` | AWS S3 | Custom endpoint for S3-compatible |
| `part_size` | 100MB | Multipart upload part size |
| `max_concurrent_parts` | 8 | Parallel part uploads |
| `max_retries` | 3 | Retry attempts |

**Credential Chain:**
1. Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
2. IAM role (EC2, ECS, Lambda)
3. AWS profile files (`~/.aws/credentials`)

---

### Azure Blob Storage

Credentials use a tagged `auth` block under `[storage]` (`config_version` 3+):
one of `account_key` (`account_name` + `account_key`), `connection_string`
(`value`), `sas` (`account_name` + `token`), or `emulator` (local Azurite).

| Setting | Description |
|---------|-------------|
| `container` | Blob container name |
| `prefix` | Optional key prefix; lets multiple repos share one container |

Built on Apache OpenDAL (the `azure_storage_blobs` 0.21 line is EOL, and its GA
replacement is Entra-ID-only). Managed-identity / Azure AD auth is not
supported — authenticate by shared key, SAS, or connection string.

**Storage Tiers:**
- **Hot**: Frequently accessed data (active repos)
- **Cool**: Infrequent access (archived branches)
- **Archive**: Long-term retention (compliance)

---

### Google Cloud Storage

| Setting | Description |
|---------|-------------|
| `project` | GCP project ID |
| `bucket` | GCS bucket name |
| `credentials_path` | Service account JSON path |

---

### MinIO (S3-Compatible)

| Setting | Value |
|---------|-------|
| `endpoint` | `http://minio:9000` |
| `access_key_id` | MinIO access key |
| `secret_access_key` | MinIO secret key |
| `bucket` | Bucket name |
| `prefix` | Key prefix (e.g. `media/`) |

**Production Performance:**
- Upload: **108 MB/s**
- Download: **263 MB/s**

---

### Backblaze B2

| Setting | Value |
|---------|-------|
| `endpoint` | `https://s3.{region}.backblazeb2.com` |
| `access_key_id` | B2 Application Key ID |
| `secret_access_key` | B2 Application Key |
| `region` | `us-west-002` |

---

### DigitalOcean Spaces

| Setting | Value |
|---------|-------|
| `endpoint` | `https://{region}.digitaloceanspaces.com` |
| `access_key_id` | Spaces access key |
| `secret_access_key` | Spaces secret key |
| `region` | `nyc3`, `sfo3`, etc. |

---

## Deployment Patterns

### Single-Region

```mermaid
flowchart LR
    subgraph Region["Region: us-east-1"]
        LB[Load Balancer] --> S1[Server 1]
        LB --> S2[Server 2]
        S1 --> B[(S3 Bucket)]
        S2 --> B
    end
```

**Use Case:** Development, small teams, single-geography

---

### Multi-Region (Active-Active)

```mermaid
flowchart TB
    subgraph US["US Region"]
        US_LB[Load Balancer] --> US_S[Servers]
        US_S --> US_B[(Primary Bucket)]
    end
    
    subgraph EU["EU Region"]
        EU_LB[Load Balancer] --> EU_S[Servers]
        EU_S --> EU_B[(Replica Bucket)]
    end
    
    US_B <-->|Cross-Region Replication| EU_B
    DNS[Route 53 / Geo DNS] --> US_LB
    DNS --> EU_LB
```

**Use Case:** Global teams, low latency worldwide, DR

---

### Hybrid (Local + Cloud)

```mermaid
flowchart LR
    subgraph Studio["On-Premise"]
        LOCAL[(Local Storage<br/>Fast Access)]
        SERVER[mediagit-server]
    end
    
    subgraph Cloud["Cloud"]
        CLOUD[(S3 / Azure / GCS<br/>Archive + DR)]
    end
    
    SERVER --> LOCAL
    LOCAL -->|Sync| CLOUD
```

**Use Case:** Media studios, large file workflows, cost optimization

---

## Security Architecture

```mermaid
flowchart LR
    subgraph Client["Client-Side"]
        AES[AES-256-GCM<br/>Before Upload]
    end
    
    subgraph Transit["In Transit"]
        TLS[TLS 1.3<br/>rustls]
    end
    
    subgraph Server["Server-Side"]
        SSE[SSE-S3 / SSE-KMS<br/>At Rest]
    end
    
    Client --> Transit --> Server
```

### Security Layers

| Layer | Implementation | Purpose |
|-------|---------------|---------|
| **Client Encryption** | AES-256-GCM | End-to-end encryption |
| **Transport** | TLS 1.3 (rustls) | In-transit protection |
| **Server Encryption** | SSE-S3 / SSE-KMS | At-rest protection |
| **Authentication** | JWT / API Keys | Access control |
| **Key Derivation** | Argon2 | Password-based keys |
| **Rate Limiting** | tower_governor | DDoS protection |
| **Audit Logging** | Built-in | Compliance |

---

## Data Flow

### Upload Flow (Presigned-Direct)

Client PUTs chunks directly to backend storage using server-minted presigned URLs, bypassing the server for data transfer. If the backend cannot sign (e.g. GCS with ADC), the server returns `null` and the client falls back to proxy upload.

```mermaid
sequenceDiagram
    participant Client
    participant Server
    participant Storage

    Client->>Server: POST /chunks/check (chunk OIDs)
    Server->>Storage: Check existence
    Storage-->>Server: Missing chunks list
    Server-->>Client: Missing chunks list

    Client->>Server: POST /:repo/chunks/upload-urls (missing OIDs)
    Server-->>Client: Presigned PUT URLs (or null → proxy fallback)

    alt Presigned URL available
        Client->>Storage: PUT chunk directly (presigned URL)
        Storage-->>Client: 200 OK
    else Proxy fallback
        Client->>Server: PUT /chunks/:id
        Server->>Storage: Store chunk
        Server-->>Client: 201 Created
    end

    Note over Client,Storage: Large chunks: presigned MPU via<br/>POST /:repo/chunks/mpu/start|complete|abort

    Client->>Server: PUT /manifests/:oid
    Server->>Storage: Store manifest
    Server-->>Client: OK
```

### Download Flow (Presigned-Direct)

Client GETs chunks directly from backend storage using server-minted presigned GET URLs. On 404 or `null` (unsigned), falls back to proxy GET. Pack-mode pull uses client-side Range-GET coalescing.

**Signing capability**: S3, MinIO, and Azure sign natively. GCS requires a service-account key JSON; ADC `authorized_user` credentials have no private key, so presign returns `null` and the client runs at proxy speed.

```mermaid
sequenceDiagram
    participant Client
    participant Server
    participant Storage

    Client->>Server: GET /objects/pack
    Server->>Storage: Get pack + manifests
    Storage-->>Server: Pack data
    Server-->>Client: Pack (X-Chunked-Objects header)

    Client->>Server: POST /:repo/chunks/download-urls (OID list)
    Server-->>Client: Presigned GET URLs (or null → proxy fallback)

    alt Presigned URL available
        Client->>Storage: GET chunk directly (presigned URL)
        Storage-->>Client: Chunk data
    else Proxy / 404 fallback
        Client->>Server: GET /chunks/:id
        Server->>Storage: Get chunk
        Server-->>Client: Chunk data
    end

    Note over Client,Storage: Pack-mode pull: client Range-GET coalescing<br/>via POST /:repo/packs/presign-download-urls
```

---

## Cloud Packs

MediaGit bundles chunks into **cloud pack objects** before uploading (Phase-3 Track F). Instead of storing thousands of individual chunk objects, the client packs up to 1,024 chunks (≤ 64 MiB) into a single object with an embedded index. This dramatically cuts API request count and cost, and speeds clones on small-chunk repos.

- **Object count reduction**: ~10,000 individual chunk objects → hundreds of pack objects (deep tests: 463–467 chunked + 35 delta objects per backend)
- **Clone path**: pack-locate via embedded index → Range-GET to fetch only needed slices
- **Integrity (F8)**: per-slice compressed-hash verification on every pull — all backends clean in deep tests

```mermaid
flowchart LR
    subgraph Client["Client — Push"]
        C1[Chunks] --> PACK[Pack Builder\n≤64 MiB / ≤1024 chunks]
        PACK --> IDX[Embedded Index]
    end

    subgraph Server["Server"]
        UP[POST /:repo/packs/upload-urls]
    end

    subgraph Storage["Cloud Backend"]
        OBJ[(Pack Object\n+ Embedded Index)]
    end

    PACK -->|presigned PUT| OBJ
    Client -->|request URLs| UP
    UP -->|presigned URLs| Client
```

---

## Cross-Backend Deep-Test Parity (2026-06-02)

614/614 tests passing across all four backends. All fsck + F8 compressed-hash checks clean.

| Backend | Tests | Cloud Savings | Push Elapsed | Clone Elapsed | Server Mem Δ | fsck / F8 |
|---------|-------|--------------|-------------|--------------|-------------|-----------|
| **MinIO** | 151/151 | 8.4%* | 76 s | 87 s | ±3.7 MB | ✅ Clean |
| **AWS S3** | 150/150 | 26.3% | 154 s | 225 s | +26 MB | ✅ Clean |
| **Azure Blob** | 154/154 | 26.5% | 304 s | 178 s | +35 MB | ✅ Clean |
| **GCS** | 159/159 | 26.4% | 178 s | 212 s | +30 MB | ✅ Clean |

> \* MinIO 8.4% = cumulative multi-push fixture (lower savings expected by design). AWS/Azure/GCS 26.3–26.5% are single-corpus savings.
> AWS/Azure/GCS push and clone are WAN-bound (~62 Mbps to ap-south-1). Reports: `dev-tests/deep-tests/reports/`.

---

## High Availability & DR

### Server Availability

| Component | Strategy |
|-----------|----------|
| **Servers** | **Single instance per directory — enforced at boot.** See [Server topology](#server-topology) |
| **Load Balancer** | Health checks; failover to a standby that owns its own `repos_dir` |
| **Storage** | Cloud-managed durability (11 9s) |

### Server topology

**Supported: one `mediagit-server` process per `repos_dir` (and per
`auth_store_dir`).** The server takes an exclusive lock on both at startup and
refuses to start if another process holds one. The lock lives on an open file
handle, so the OS releases it on any exit — a crashed server does not block its
own restart.

This is a real constraint, not caution. The server keeps state that is
per-process, and a second instance sharing a directory corrupts it *silently*:

| State | What a second instance does |
|---|---|
| `users.jsonl` / `grants.jsonl` | each loads at boot and full-rewrites on mutation; the slower writer's snapshot wins and the other's users and grants vanish |
| `locks.jsonl` | same full rewrite, and the double-lock 409 guard is per-process, so two clients can both hold one path |
| circular chunk-delta guard | in-process only; two instances can write A→B and B→A and leave the repo unpushable |
| pack verification | the same pack is verified twice over the WAN |
| token revocation | logout does not propagate; the sibling keeps accepting a revoked JWT |
| rate limiter | N instances serve N× the configured budget |
| want-cache | negotiation state is per-process, so clone/push negotiation breaks |

None of those raise an error, which is why the enforcement is at boot rather
than at each site.

To scale out today, give each instance its own `repos_dir` and `auth_store_dir`
and shard repositories across them at the load balancer. True horizontal scale —
several instances over one shared store — needs the state above moved to a
shared store (Postgres/Redis) and is **not implemented**.

`MEDIAGIT_ALLOW_MULTI_INSTANCE=1` downgrades the refusal to a warning. It exists
for an operator who has genuinely separated every directory and is only tripping
over a lock file on a shared mount. It does not make the sharing safe.

### Disaster Recovery

| Strategy | RPO | RTO | Cost |
|----------|-----|-----|------|
| **Cross-Region Replication** | Minutes | Minutes | High |
| **Daily Snapshots** | 24 hours | Hours | Medium |
| **Backup to Different Provider** | Hours | Hours | Medium |

---

## Monitoring & Observability

### Prometheus Metrics

```mermaid
flowchart LR
    subgraph Servers["MediaGit Servers"]
        S1["Server :9090/metrics"]
        S2["Server :9090/metrics"]
    end
    
    PROM[Prometheus] --> S1
    PROM --> S2
    PROM --> GRAF[Grafana]
```

### Key Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `mediagit_bytes_uploaded_total` | Counter | Total bytes uploaded |
| `mediagit_bytes_downloaded_total` | Counter | Total bytes downloaded |
| `mediagit_objects_stored_total` | Counter | Total objects stored |
| `mediagit_dedup_ratio` | Gauge | Deduplication ratio |
| `mediagit_compression_ratio` | Gauge | Compression ratio |
| `mediagit_request_duration_seconds` | Histogram | Request latency |

---

## Cost Optimization

### Storage Cost Reduction

| Strategy | Savings |
|----------|---------|
| **Deduplication** | 60-95% (chunk-level) |
| **Compression** | 50-93% (type-aware) |
| **Delta Encoding** | Up to 95% (versioning) |
| **Infrequent Access Tier** | 40-50% (old refs) |

### Egress Optimization

| Strategy | Benefit |
|----------|---------|
| **Compression** | Reduced transfer size |
| **Have/Want Protocol** | Only transfer missing objects |
| **Chunked Transfer** | Resume interrupted downloads |
| **Regional Caching** | Reduce cross-region egress |

---

## Configuration Examples

### AWS S3 (mediagit.toml)

All storage backend fields go directly in `[storage]` alongside the `backend` key:

```toml
[storage]
backend = "s3"
bucket = "my-mediagit-repo"
region = "us-east-1"
# Uses AWS credential chain by default (IAM role, ~/.aws/credentials, env vars)
# Optional: custom endpoint for S3-compatible services
# endpoint = "https://s3.us-east-1.amazonaws.com"
```

### Azure Blob (mediagit.toml)

```toml
[storage]
backend = "azure"
container = "repos"
auth = { type = "account_key", account_name = "mediagitstorage", account_key = "..." }
# variants: connection_string { value }, sas { account_name, token }, emulator {}
# account_key may also come from the AZURE_STORAGE_KEY env var
```

### MinIO (mediagit.toml)

```toml
[storage]
backend = "s3"
bucket = "mediagit"
endpoint = "http://minio.internal:9000"
access_key_id = "minioadmin"        # field: access_key_id
secret_access_key = "minioadmin"    # field: secret_access_key
region = "us-east-1"                # any value; MinIO ignores region
```

### Multi-Backend (mediagit.toml)

```toml
[storage]
backend = "multi"
primary = "primary-s3"
replicas = ["replica-local"]

[storage.backends.primary-s3]
backend = "s3"
bucket = "mediagit-backup"
region = "us-west-2"

[storage.backends.replica-local]
backend = "filesystem"
base_path = "/fast-storage/mediagit"
```

---

## Server Configuration

### Environment Variables

```bash
# Server
MEDIAGIT_PORT=3000
MEDIAGIT_HOST=0.0.0.0

# TLS
MEDIAGIT_TLS_CERT=/certs/server.crt
MEDIAGIT_TLS_KEY=/certs/server.key

# Auth
MEDIAGIT_JWT_SECRET=your-secret-key
MEDIAGIT_API_KEY_ENABLED=true

# Storage (AWS)
AWS_ACCESS_KEY_ID=AKIA...
AWS_SECRET_ACCESS_KEY=...
AWS_REGION=us-east-1

# Metrics
MEDIAGIT_METRICS_ENABLED=true
MEDIAGIT_METRICS_PORT=9090
```

### Docker Compose Example

```yaml
version: '3.8'
services:
  mediagit:
    image: mediagit/server:latest
    ports:
      - "3000:3000"
      - "9090:9090"
    environment:
      - AWS_REGION=us-east-1
      - MEDIAGIT_METRICS_ENABLED=true
    volumes:
      - ./config:/etc/mediagit
    deploy:
      replicas: 3
      
  prometheus:
    image: prom/prometheus
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      
  grafana:
    image: grafana/grafana
    ports:
      - "3001:3000"
```

---

## Performance Tuning

### S3 Backend

| Setting | Recommended | Impact |
|---------|-------------|--------|
| `part_size` | 100MB (default) | Larger = fewer API calls |
| `max_concurrent_parts` | 8-16 | Higher = faster uploads |
| `max_retries` | 3-5 | More resilience |

### Server

| Setting | Recommended | Impact |
|---------|-------------|--------|
| Worker threads | CPU cores x 2 | Throughput |
| Connection pool | 100-500 | Concurrent requests |
| Request timeout | 300s | Large file handling |

---

## Summary

| Aspect | Recommendation |
|--------|----------------|
| **Primary Storage** | AWS S3 / Azure Blob / GCS |
| **Cost-Optimized** | Backblaze B2, MinIO |
| **Enterprise** | Multi-region with replication |
| **Studios** | Hybrid (Local + Cloud) |
| **Security** | TLS + Client-side AES-256-GCM |
| **Monitoring** | Prometheus + Grafana |
