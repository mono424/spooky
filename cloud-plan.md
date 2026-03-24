# Sp00ky Cloud - Comprehensive Plan

## Context

Sp00ky is a local-first, real-time sync framework built on SurrealDB with DBSP-powered incremental view maintenance. It already supports singlenode and cluster deployment modes (SurrealDB + Scheduler + N SSP instances via NATS). The goal is to build a **managed cloud service** so users can deploy Sp00ky clusters via CLI, pay through Stripe, and never touch infrastructure. The API will be written in **Golang**, tenant workloads will run in **Firecracker VMs** for hard isolation, and the entire experience will be **CLI-first** (no web dashboard).

---

## Architecture Overview

### Control Plane vs Data Plane

```
Internet
  |
  v
[Load Balancer] ── TLS termination, subdomain routing
  |         |
  v         v
Control Plane (Go API + Postgres)     Data Plane (Firecracker VMs)
  - Auth, projects, billing             - Per-tenant VM groups
  - VM orchestration                    - SurrealDB VM
  - Stripe webhooks                     - Scheduler VM
  - Host fleet management               - SSP VMs (1-N)
                                        - Isolated tenant networks
```

**Control Plane**: Single Go binary + Postgres. Runs on 3 nodes.
**Data Plane**: Firecracker VMs on bare-metal hosts, managed by a host agent (Go binary on each node).

### Multi-Tenancy: Strict Isolation

Each project gets its own VM group -- no shared databases, no shared processes. Only shared infrastructure is the host machines and control plane.

---

## Firecracker VM Strategy

### Why Firecracker

- Hard tenant isolation (separate kernels, no container escape risk)
- Sub-second boot (~200ms) -- close to container speed
- SurrealDB/SSP/Scheduler are single binaries, ideal for minimal rootfs
- Deterministic resource allocation via cgroups v2
- Network isolation per VM via tap devices

### What Runs Where

| In Firecracker VMs (per tenant) | Native on Host |
|---|---|
| SurrealDB | Go API (control plane) |
| Scheduler | Postgres (control plane) |
| SSP instances (1-N) | Host Agent (Go, manages VMs) |
| NATS (embedded in Scheduler) | Reverse proxy (Envoy/nginx) |
| | Monitoring (Prometheus/Grafana/Loki) |

### VM Types Per Project

| Role | Base Resources | Scaling |
|------|---------------|---------|
| SurrealDB | 2 vCPU, 2GB RAM, 20GB persistent disk | Vertical (resize) |
| Scheduler | 1 vCPU, 1GB RAM, 5GB disk (RocksDB) | Single instance |
| SSP | 1 vCPU, 512MB RAM, no persistent disk | Horizontal (1-N) |

### Rootfs Snapshots

Pre-built minimal images (~40-50MB each):
- `sp00ky-surrealdb.ext4` -- Alpine + SurrealDB binary
- `sp00ky-scheduler.ext4` -- Alpine + Scheduler binary
- `sp00ky-ssp.ext4` -- Alpine + SSP binary

Config passed via Firecracker MMDS (metadata service). Persistent data on separate attached drives.

### Per-Tenant Networking

```
Host Machine
  br-tenant-{slug}           (bridge, 10.100.{id}.0/24)
    tap-surrealdb-{slug}     (10.100.{id}.10)
    tap-scheduler-{slug}     (10.100.{id}.20)
    tap-ssp1-{slug}          (10.100.{id}.30)
    tap-ssp2-{slug}          (10.100.{id}.31)
```

No routing between tenant bridges. External access via reverse proxy:
- `{slug}.db.cloud.sp00ky.dev` -> SurrealDB WebSocket
- `{slug}.ssp.cloud.sp00ky.dev` -> Scheduler endpoint

---

## Golang API Design

### Project Structure

```
github.com/sp00kycloud/cloud-api/
  cmd/
    api/main.go              # HTTP server
    worker/main.go           # Background jobs (billing, VM health)
    agent/main.go            # Host agent (runs on each bare-metal node)
  internal/
    auth/                    # JWT + API key middleware
    projects/                # Project CRUD
    deployments/             # Deploy lifecycle FSM
    vms/                     # Firecracker orchestration
    billing/                 # Stripe integration + metering
    monitoring/              # Log/metrics forwarding
    hosts/                   # Fleet management + VM placement
  pkg/
    api/                     # Shared HTTP types
    grpc/                    # Proto defs for agent communication
  migrations/                # Postgres migrations (goose)
```

Single Go binary with internal domain packages -- not microservices.

### Key API Endpoints

**Auth:**
- `POST /v1/auth/login` -- Device auth flow, returns JWT
- `POST /v1/auth/keys` -- Create API key (`spk_live_*` prefix)
- `DELETE /v1/auth/keys/:id` -- Revoke key

**Projects:**
- `POST /v1/projects` -- Create project
- `GET /v1/projects` -- List projects
- `GET /v1/projects/:id` -- Project details
- `DELETE /v1/projects/:id` -- Destroy project + all VMs

**Deployments:**
- `POST /v1/projects/:id/deploy` -- Deploy/redeploy
- `GET /v1/projects/:id/deployment` -- Status
- `POST /v1/projects/:id/scale` -- Scale SSP count
- `GET /v1/projects/:id/logs` -- Stream logs (SSE)
- `POST /v1/projects/:id/schema/push` -- Upload sp00ky.yml + schema
- `POST /v1/projects/:id/migrations/apply` -- Apply migrations

**Billing:**
- `POST /v1/billing/checkout` -- Get Stripe Checkout URL
- `POST /v1/billing/portal` -- Get Stripe billing portal URL
- `GET /v1/billing/usage` -- Current usage

### VM Orchestration Flow

```
CLI -> API -> Postgres (write deployment intent)
  -> Worker picks up job
  -> Worker calls Host Agent via gRPC
  -> Agent creates Firecracker VM
  -> Agent reports status back
  -> Worker updates Postgres
  -> CLI polls /deployment status
```

The API never directly calls Firecracker -- only the host agent does.

### Authentication

1. **CLI Login**: Device authorization flow. CLI opens browser, user authenticates, API returns JWT + refresh token stored in `~/.sp00ky/credentials.json`
2. **API Keys**: Long-lived, scoped (`spk_live_*`), stored SHA-256 hashed in Postgres

---

## Control Plane Database (Postgres)

### Key Tables

```sql
accounts          -- id, email, stripe_customer_id
api_keys          -- account_id, prefix, key_hash, scopes
projects          -- account_id, slug, plan, mode, config (JSONB), status, stripe_subscription_id
deployments       -- project_id, version, status (pending/provisioning/running/failed/destroyed)
vms               -- deployment_id, host_id, role, internal_ip, status, resources (JSONB)
hosts             -- hostname, ip, region, capacity (JSONB), allocated (JSONB)
usage_events      -- project_id, metric, value, recorded_at
schema_uploads    -- project_id, schema_hash, bundle (BYTEA)
```

---

## CLI Extensions

New `sp00ky cloud` command group added to `apps/cli/src/main.rs`:

```
sp00ky cloud login              # Browser-based device auth
sp00ky cloud logout             # Clear credentials
sp00ky cloud create             # Create project (interactive)
sp00ky cloud deploy             # Deploy current project
sp00ky cloud status             # Deployment status
sp00ky cloud logs [--service X] # Tail logs
sp00ky cloud scale --ssp N      # Scale SSP instances
sp00ky cloud destroy            # Tear down
sp00ky cloud billing            # Open Stripe portal in browser
sp00ky cloud billing usage      # Show usage in terminal
sp00ky cloud migrate apply      # Apply migrations to cloud
sp00ky cloud migrate status     # Check migration status
```

### Stripe Billing Flow

```
1. `sp00ky cloud create` -> project created (status: pending_payment)
2. CLI prints: "Run `sp00ky cloud billing` to set up payment"
3. `sp00ky cloud billing` -> API creates Stripe Checkout Session -> opens browser
4. User completes payment in Stripe
5. Stripe webhook -> API marks project active
6. `sp00ky cloud deploy` now works
```

---

## Billing & Stripe Integration

### Pricing Plans

| Plan | SurrealDB | Scheduler | SSP | Price |
|------|-----------|-----------|-----|-------|
| Starter | 1 vCPU, 1GB | 1 vCPU, 512MB | 1 | $29/mo |
| Pro | 2 vCPU, 4GB | 1 vCPU, 1GB | 3 | $99/mo |
| Business | 4 vCPU, 8GB | 2 vCPU, 2GB | 5+ | $299/mo |

### Metered Overages

- Additional storage: $0.10/GB/month
- Additional SSP instances: $15/instance/month
- Bandwidth egress: $0.05/GB (first 10GB free)

Worker process collects usage every 5 minutes, pushes to Stripe as metered usage at billing cycle end.

---

## Hosting & Operations

### Phase 1: Hetzner Bare-Metal

- 3x control plane nodes (8-core, 64GB) -- API + Postgres (Patroni cluster)
- 5x data plane nodes (12-core, 128GB, 2x NVMe) -- Firecracker VMs
- 1x monitoring node -- Prometheus, Grafana, Loki

Firecracker requires `/dev/kvm` -- dedicated hardware is required.

### Backups

- **SurrealDB volumes**: Daily LVM snapshots -> S3-compatible object storage
- **Postgres**: WAL-G continuous archiving, point-in-time recovery
- **Schema bundles**: Stored in Postgres, replicated

### Monitoring

- Prometheus scraping host agents + VM health endpoints (Scheduler already exposes `/metrics`)
- Loki for logs (collected from VM serial consoles by host agents)
- Grafana dashboards for ops + tenant-facing metrics via API

---

## Security

- **VM isolation**: Firecracker jailer mode (seccomp + namespaces)
- **Network isolation**: Per-tenant bridge, no inter-tenant routing
- **Secrets**: Per-project auth secrets, generated with `crypto/rand`, stored AES-256-GCM encrypted in Postgres, passed via MMDS
- **Mutual TLS**: Between API and host agents
- **API auth**: JWT (RS256, 1hr expiry) + API keys with scoped permissions
- **Rate limiting**: 100 req/min authenticated, 10 req/min unauthenticated

---

## Client Connectivity (No SDK Changes Needed)

```typescript
// Sp00kyClient already supports arbitrary endpoints
const client = new Sp00kyClient({
  database: {
    endpoint: "wss://abc123.db.cloud.sp00ky.dev",
    namespace: "main",
    database: "main",
    token: "<jwt>"
  },
  // ... rest of config unchanged
});
```

The `endpoint` field in `Sp00kyConfig` (`packages/core/src/types.ts`) already supports this. Migration from self-hosted to cloud is a config change.

---

## Phased Rollout

### Phase 1: MVP (8-10 weeks)
- Go API: auth, project CRUD, basic deployment
- Host agent: Firecracker VM create/destroy
- SurrealDB + Scheduler + SSP provisioning (cluster mode)
- Reverse proxy with subdomain routing
- CLI: `login`, `create`, `deploy`, `status`, `destroy`, `billing`
- Stripe Checkout (fixed pricing, no metering)
- Basic health checks

### Phase 2: Production Hardening (4-6 weeks)
- Automated daily backups
- `sp00ky cloud logs` (SSE streaming)
- `sp00ky cloud migrate` commands
- Usage metering + Stripe metered billing
- Auto-restart on VM failure
- Schema push + hot-reload

### Phase 3: Scale (6-8 weeks)
- `sp00ky cloud scale` for SSP horizontal scaling
- VM placement optimization (cross-host, anti-affinity)
- Tenant metrics dashboard (via API)
- Custom domains (CNAME + Let's Encrypt)
- Webhook notifications

### Phase 4: Enterprise (ongoing)
- Multi-region data plane
- Private networking (WireGuard tunnels)
- SOC2 compliance
- SLA guarantees
- Web dashboard

---

## Critical Files

| File | Relevance |
|------|-----------|
| `apps/cli/src/main.rs` | Extend Commands enum with `cloud` subcommand |
| `apps/cli/src/backend.rs` | Sp00kyConfig struct -- cloud deploy reads this |
| `apps/scheduler/src/config.rs` | Env var pattern to replicate in VM provisioning |
| `example/schema/docker-compose.cluster.yml` | Reference topology for cloud provisioner |
| `packages/core/src/types.ts` | `Sp00kyConfig.database.endpoint` -- client connection point |
| `apps/ssp/src/lib.rs` | SSP env vars and health endpoints |

## Verification

1. **Go API**: `go test ./...` + manual curl against local instance
2. **Host Agent**: Integration test with Firecracker on a KVM-enabled machine
3. **CLI**: `sp00ky cloud login` -> `create` -> `billing` -> `deploy` -> `status` -> `logs` -> `destroy` end-to-end flow
4. **Client connectivity**: Connect a Sp00kyClient to a cloud-deployed instance, verify sync works
5. **Billing**: Stripe test mode end-to-end (checkout -> webhook -> subscription active)
