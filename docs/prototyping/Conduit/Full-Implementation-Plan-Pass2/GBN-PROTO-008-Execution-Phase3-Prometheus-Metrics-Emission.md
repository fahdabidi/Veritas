# GBN-PROTO-008 - Execution Phase 3 Detailed Plan: Prometheus Metrics Emission + OTLP Tracing

**Status:** Implemented - local k8s smoke and workspace validation passed; direct observability query validation blocked by WSL Docker restarts
**Primary Goal:** add a `/metrics` HTTP endpoint to each Conduit V2 service binary using
the `prometheus` Rust crate, exposing the same counter set as the AWS variant
(`AuthorityMetricsSnapshot`, new `ReceiverMetricsSnapshot`, `BridgeMetricsSnapshot`).
Also add an OpenTelemetry tracing layer that emits spans to the Tempo OTLP endpoint
configured in GBN-PROTO-008 Phase 2, with `chain_id` as a span attribute so Grafana →
Tempo Explore can reconstruct distributed traces. Replaces the AWS-specific CloudWatch
push design from [GBN-PROTO-007 Phase 3](GBN-PROTO-007-Execution-Phase3-CloudWatch-Metrics-Emission.md).
**Source Plan:** [GBN-PROTO-008 Execution Plan](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md)
**AWS Sibling Plan:** [GBN-PROTO-007 Phase 3](GBN-PROTO-007-Execution-Phase3-CloudWatch-Metrics-Emission.md)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Existing authority counters | [`AuthorityMetricsSnapshot`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs) | reused; new export mechanism wraps them |
| Existing trace propagation | GBN-PROTO-006 Phase 7 — `chain_id` flows through protocol, runtime, publisher, persistence | Phase 3 attaches `chain_id` to OTLP spans |
| Existing logger / tracing layer | `tracing` + `tracing-subscriber` added in this phase | Phase 3 initializes an OTLP subscriber when the local k8s OTLP endpoint env var is present |
| Tempo OTLP endpoints | `tempo.observability.svc.cluster.local:4317` (gRPC) and `:4318` (HTTP) per Phase 2 values | service environment must point at one of these |
| Prometheus scrape annotations | already on Phase 1 manifests | `/metrics` endpoint must respond at the annotated path/port |

**Note:** GBN-PROTO-007 Phase 3 (CloudWatch) landed before this local-k8s variant.
This phase reuses the shared snapshot structs and keeps CloudWatch disabled in local
manifests via `GBN_BRIDGE_CLOUDWATCH_ENABLED=false`.

---

## 2. Review Summary

| Gap | Why It Matters | Resolution For Phase 3 |
|---|---|---|
| No metrics export from V2 binaries | Phase 2 observability stack has nothing to scrape | add `/metrics` HTTP handler driven by `prometheus` crate |
| Receiver and bridge have no metric structs | already resolved by GBN-PROTO-007 Phase 3 | reuse `ReceiverMetricsSnapshot` and `BridgeMetricsSnapshot` |
| chain_id not visible as a span attribute | Tempo cannot index by chain_id without it | add OTLP exporter + `tracing-opentelemetry` layer; attach `chain_id` to relevant spans |
| Pulling AWS SDK for local-only dev is heavy | AWS deps are already present for the parity track | keep runtime AWS calls disabled in local k8s with `GBN_BRIDGE_CLOUDWATCH_ENABLED=false` |

---

## 3. Scope Lock

### In Scope

- new dependencies: `prometheus` (or `metrics` + `metrics-exporter-prometheus`), `opentelemetry`,
  `opentelemetry-otlp`, `tracing-opentelemetry`
- new module `gbn-bridge-publisher/src/metrics_prometheus.rs` with a `Registry` builder
  that converts each snapshot type into a Prometheus exposition
- `/metrics` HTTP route added to each binary's existing public listener (or admin listener)
- new structs `ReceiverMetricsSnapshot` and `BridgeMetricsSnapshot` (shared with the
  AWS variant; if AWS Phase 3 hasn't landed yet, this phase introduces them)
- OTLP span exporter wired into the existing tracing subscriber in each binary
- a small helper `attach_chain_id(span, chain_id)` so call sites stay terse
- Cargo feature flags: default `prometheus-metrics`, opt-in `aws-cloudwatch`
- per-binary integration test that spins up the metrics handler and asserts Prometheus
  exposition format

### Out Of Scope

- AWS CloudWatch emission (covered by GBN-PROTO-007 Phase 3)
- Custom Grafana dashboards beyond the overview from Phase 2
- Sampled tracing (always on for local dev; sample rate adjustable later)
- Removing the existing `AuthorityMetrics` in-memory bookkeeping (kept verbatim;
  Prometheus reads it via a snapshot fetch)

---

## 4. Preflight Gates

1. GBN-PROTO-007 Phase 1 (admin endpoints) has landed — `/v1/admin/metrics` JSON exists
   already and is reused in tests.
2. GBN-PROTO-008 Phase 1 + Phase 2 are landed; observability stack is live in the cluster.
3. `cargo fmt --all --check` and `cargo test --workspace` pass on V2.
4. V1 protected-path diff is clean.

---

## 5. File-by-File Specification

### 5.1 Modify: workspace `Cargo.toml`

Add to `[workspace.dependencies]`:

```toml
prometheus = { version = "0.13", default-features = false }
opentelemetry = { version = "0.21", features = ["trace"] }
opentelemetry-otlp = { version = "0.14", features = ["trace", "grpc-tonic"] }
tracing-opentelemetry = "0.22"
```

Pin to whatever versions are compatible with the existing `tracing` version in the
workspace.

### 5.2 Modify: `gbn-bridge-publisher/Cargo.toml`

Add to `[dependencies]`:

```toml
prometheus = { workspace = true }
opentelemetry = { workspace = true }
opentelemetry-otlp = { workspace = true }
tracing-opentelemetry = { workspace = true }
```

Add feature flags:

```toml
[features]
default = ["prometheus-metrics"]
prometheus-metrics = []
aws-cloudwatch = ["dep:aws-config", "dep:aws-sdk-cloudwatch"]

[dependencies.aws-config]
workspace = true
optional = true
[dependencies.aws-sdk-cloudwatch]
workspace = true
optional = true
```

If `aws-config` / `aws-sdk-cloudwatch` are already in `[dependencies]` from the AWS
variant, move them to optional with `optional = true`.

### 5.3 New file: `gbn-bridge-publisher/src/metrics_prometheus.rs`

```rust
//! Prometheus exposition for V2 service metrics.
//!
//! Each binary builds a `Registry` once at startup, populates it with `IntCounterVec`s
//! corresponding to the in-memory `*MetricsSnapshot`, and exposes a `/metrics` route
//! that scrapes the latest snapshot and renders Prometheus text format on each request.

use prometheus::{Encoder, IntCounterVec, Opts, Registry, TextEncoder};

use crate::metrics::AuthorityMetricsSnapshot;

pub struct AuthorityPromMetrics {
    pub registry: Registry,
    successful_registrations: IntCounterVec,
    rejected_registrations: IntCounterVec,
    heartbeats: IntCounterVec,
    revocations: IntCounterVec,
    issued_catalogs: IntCounterVec,
    bootstrap_requests: IntCounterVec,
    rejected_bootstrap_requests: IntCounterVec,
    bootstrap_progress_reports: IntCounterVec,
    issued_batches: IntCounterVec,
    batch_rollovers: IntCounterVec,
}

impl AuthorityPromMetrics {
    pub fn new(stack_env: &str) -> Self {
        let registry = Registry::new();
        let labels = vec!["service".to_string(), "stack".to_string()];
        let mk = |name: &str| -> IntCounterVec {
            let opts = Opts::new(name, name).namespace("conduit").subsystem("authority");
            let cv = IntCounterVec::new(opts, &labels).unwrap();
            registry.register(Box::new(cv.clone())).unwrap();
            cv
        };
        Self {
            registry,
            successful_registrations: mk("successful_registrations_total"),
            rejected_registrations: mk("rejected_registrations_total"),
            heartbeats: mk("heartbeats_total"),
            revocations: mk("revocations_total"),
            issued_catalogs: mk("issued_catalogs_total"),
            bootstrap_requests: mk("bootstrap_requests_total"),
            rejected_bootstrap_requests: mk("rejected_bootstrap_requests_total"),
            bootstrap_progress_reports: mk("bootstrap_progress_reports_total"),
            issued_batches: mk("issued_batches_total"),
            batch_rollovers: mk("batch_rollovers_total"),
        }
        // Note: counters above are "delta-only" — the ::set() pattern emulates absolute
        // values by computing the delta from the previous snapshot on each scrape.
    }

    /// Update Prometheus counters from the latest in-memory snapshot.
    /// Call from the /metrics handler on each request.
    pub fn refresh(&self, snapshot: &AuthorityMetricsSnapshot, service: &str, stack: &str) {
        let labels = &[service, stack];
        // Counters are monotonic; set absolute via .reset() + .inc_by(value).
        // Or use Gauge if absolute exposition is preferred.
        // For simplicity, expose as Gauges (gauge_total naming convention is acceptable).
        // ...
    }

    pub fn encode(&self) -> Result<Vec<u8>, prometheus::Error> {
        let mut buf = Vec::new();
        let encoder = TextEncoder::new();
        let metrics_families = self.registry.gather();
        encoder.encode(&metrics_families, &mut buf)?;
        Ok(buf)
    }
}

// Analogous structs for Receiver and Bridge.
pub struct ReceiverPromMetrics { /* ... */ }
pub struct BridgePromMetrics  { /* ... */ }
```

> **Implementation note:** the `prometheus` crate models counters as monotonic. The
> Conduit in-memory `AuthorityMetricsSnapshot` is also monotonic, so a straightforward
> mapping is `prometheus::IntCounter` per field, calling `inc_by(delta)` on each scrape.
> The `refresh` method tracks the previous snapshot and emits the delta. Alternatively,
> use `IntGauge` with absolute values; either works for `rate(..)` queries.

### 5.4 New file: `gbn-bridge-publisher/src/metrics_otlp.rs`

```rust
//! OpenTelemetry OTLP exporter wiring.
//!
//! Builds a tracer pipeline that pushes spans to the Tempo OTLP endpoint configured via
//! the OTLP_ENDPOINT env var (default: http://tempo.observability.svc.cluster.local:4317).
//! Adds a tracing-subscriber layer so existing tracing instrumentation flows through.

use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn init_otlp_tracing(service_name: &str, otlp_endpoint: &str) -> anyhow::Result<()> {
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(otlp_endpoint);
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(
            opentelemetry::sdk::trace::config().with_resource(
                opentelemetry::sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new("service.name", service_name.to_string()),
                ]),
            ),
        )
        .install_batch(opentelemetry::runtime::Tokio)?;
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true);
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()?;
    Ok(())
}

/// Helper used at trace call sites to attach a chain_id attribute to the current span.
pub fn record_chain_id(chain_id: &gbn_bridge_protocol::ChainId) {
    let span = tracing::Span::current();
    span.record("chain_id", &tracing::field::display(chain_id));
}
```

### 5.5 Modify: `gbn-bridge-publisher/src/admin.rs`

Phase 1 of GBN-PROTO-007 added `/v1/admin/metrics` returning JSON. Phase 3 of this plan
adds a sibling Prometheus-exposition route — but **on the public listener**, not the
admin one, because Prometheus must scrape it without ECS-exec / kubectl-exec. Add the
route in each binary's `main` (see §5.6) rather than in admin.rs.

### 5.6 Modify: each service binary `main`

`crates/gbn-bridge-cli/src/bin/publisher-authority.rs`:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Init OTLP tracing first so all subsequent logs/spans go to Tempo.
    let otlp_endpoint = std::env::var("OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://tempo.observability.svc.cluster.local:4317".to_string());
    gbn_bridge_publisher::metrics_otlp::init_otlp_tracing(
        "publisher-authority",
        &otlp_endpoint,
    )?;

    // 2. Build Prometheus metrics registry.
    let stack_env = std::env::var("GBN_BRIDGE_STACK_ENV").unwrap_or_else(|_| "dev".into());
    let prom = gbn_bridge_publisher::metrics_prometheus::AuthorityPromMetrics::new(&stack_env);

    // 3. Build the existing public router and add the /metrics route.
    let public_router = build_public_router(...).route(
        "/metrics",
        axum::routing::get({
            let prom = prom.clone();
            let metrics = metrics.clone();
            move || async move {
                let snapshot = metrics.lock().await.snapshot();
                prom.refresh(&snapshot, "authority", &stack_env);
                let body = prom.encode().unwrap_or_default();
                ([(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
            }
        }),
    );

    // 4. Start the public listener (existing logic).
    // 5. Start the admin listener (Phase 1 of GBN-PROTO-007).

    // 6. Graceful shutdown: flush pending spans.
    opentelemetry::global::shutdown_tracer_provider();
}
```

`publisher-receiver.rs` and `exit-bridge.rs`: same pattern with their respective Prom
metric structs (`ReceiverPromMetrics`, `BridgePromMetrics`) and service names.

For the bridge binary, keep the admin listener on localhost-only `9090` and expose a
separate metrics-only listener on `0.0.0.0:9100`. The bridge k8s manifest annotates
`prometheus.io/port: "9100"` so Prometheus can scrape metrics without exposing admin
commands over the pod network.

### 5.7 Modify: `gbn-bridge-publisher/src/metrics.rs`

Append the receiver and bridge snapshot structs (shared with the AWS variant from
GBN-PROTO-007 Phase 3 §5.2). If the AWS variant has not yet landed, this phase introduces
them:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ReceiverMetricsSnapshot {
    pub frames_accepted: u64,
    pub frames_rejected: u64,
    pub bytes_ingested: u64,
    pub sessions_opened: u64,
    pub sessions_closed: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct BridgeMetricsSnapshot {
    pub commands_received: u64,
    pub commands_acked: u64,
    pub commands_rejected: u64,
    pub frames_forwarded: u64,
    pub bytes_forwarded: u64,
    pub control_reconnects: u64,
}
```

Plus their `ReceiverMetrics` / `BridgeMetrics` builders following the same pattern as
`AuthorityMetrics`.

### 5.8 Modify: chain_id attach call sites

Whichever functions in the V2 codebase already emit `chain_id` to logs (per
GBN-PROTO-006 Phase 7) gain one extra line:

```rust
gbn_bridge_publisher::metrics_otlp::record_chain_id(&chain_id);
```

This attaches the chain_id to the current OTLP span. Search for existing
`chain_id = %chain_id` style log emissions in each crate and add the helper call near
each. Estimated 6–10 call sites total.

### 5.9 Modify: each binary's k8s manifest (from GBN-PROTO-008 Phase 1)

Add an OTLP endpoint env var so the binary knows where to push spans:

```yaml
env:
  - name: OTLP_ENDPOINT
    value: http://tempo.observability.svc.cluster.local:4317
```

Add to all three Deployment specs.

### 5.10 New file: `gbn-bridge-publisher/tests/metrics_prometheus_endpoint.rs`

```rust
//! Phase 3 Prometheus endpoint coverage.

#[tokio::test]
async fn authority_metrics_endpoint_returns_prometheus_exposition() { ... }

#[tokio::test]
async fn receiver_metrics_endpoint_returns_prometheus_exposition() { ... }

#[tokio::test]
async fn bridge_metrics_endpoint_returns_prometheus_exposition() { ... }

#[tokio::test]
async fn metrics_endpoint_includes_service_and_stack_labels() { ... }

#[tokio::test]
async fn counter_values_increase_with_simulated_activity() { ... }
```

Use `prometheus::TextEncoder` plus a regex on the response body to assert the
`# HELP`, `# TYPE`, and value lines are well-formed.

---

## 6. Implementation Notes

- Added `metrics_prometheus.rs` to render Prometheus text exposition from the existing
  monotonic snapshot counters for authority, receiver, and bridge.
- Added `metrics_http.rs` for the bridge's metrics-only listener on
  `GBN_BRIDGE_METRICS_BIND_ADDR` (default `0.0.0.0:9100`).
- Added `metrics_otlp.rs` and initialized OTLP tracing in all three binaries when
  `GBN_BRIDGE_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_ENDPOINT`, or `OTLP_ENDPOINT` is set.
- Wired `chain_id` span attributes/events through authority request handling, receiver
  proxy forwarding, bridge control commands, and bridge creator upload paths.
- Updated local k8s config with `GBN_BRIDGE_OTLP_ENDPOINT` and bridge scrape annotations
  for port `9100`.
- Added focused Prometheus endpoint tests for authority, receiver, and bridge.

## 7. Validation

Completed static/local validation in the current Windows-hosted shell:

1. `cargo fmt --all` and `cargo fmt --all --check` passed.
2. `cargo check -p gbn-bridge-publisher -p gbn-bridge-cli` passed.
3. `cargo check --workspace` passed.
4. `cargo test -p gbn-bridge-publisher metrics_prometheus` passed.
5. `cargo test -p gbn-bridge-publisher --test metrics_prometheus_endpoint` passed.
6. `cargo test -p gbn-bridge-cli` passed.
7. PyYAML parsed every YAML file under `prototype/gbn-bridge-proto/infra/k8s/conduit/base`.
8. `git diff --check` passed with only Windows LF/CRLF warnings.
9. V1 protected-path diff was clean.

Live WSL2 update (2026-05-07):

1. Rebuilt and loaded Phase 3 images into the local k3d cluster.
2. Authority, receiver, and bridge pods reached Ready after fixing OTLP tracer startup to
   keep the tonic batch exporter inside a live Tokio runtime.
3. `k8s-smoke.sh --send-dummy` passed from authority, receiver, and each bridge pod.
4. The full V2 workspace suite passed through
   `infra/scripts/k8s-test-publisher-postgres.sh --workspace` against the Kubernetes
   Postgres StatefulSet.
5. The observability stack rolled out and Prometheus reported Available. Direct
   Prometheus/Tempo/Loki query checks were attempted but blocked by WSL Docker daemon
   restarts that stopped the k3d node containers.

Retained live validation checklist for the next stable WSL2 Docker session:

For future direct-observability reruns:

1. `cargo fmt --all --check`, `cargo check --workspace`, the focused Phase 3 Prometheus
   endpoint tests, and the full workspace suite pass.
2. Build images with default features:
   `docker build -f Dockerfile.publisher-authority -t veritas/publisher-authority:dev .` etc.
3. Bring up cluster + observability stack via Phase 1 + Phase 2 scripts.
4. `kubectl rollout restart -n veritas deployment/publisher-authority deployment/publisher-receiver deployment/exit-bridge`
   to pick up the new images.
5. Wait 30 s, then in Grafana → Explore → Prometheus, query
   `conduit_authority_successful_registrations_total` — returns at least one series with
   labels `{service="authority", stack="dev-local"}`.
6. After running a SendDummy from any pod (Phase 4 of GBN-PROTO-007 must be available),
   in Grafana → Explore → Tempo, search for the returned chain_id — a 3-hop trace appears
   (creator-pod → bridge-pod → receiver-pod).
7. In Grafana → Explore → Loki, query `{namespace="veritas"} | json | chain_id="<id>"`
   shows log lines tagged with that chain_id.
8. Performance sanity: per-pod CPU stays under 50 m steady-state; per-pod memory stays
   under 150 Mi. Trace exporter doesn't cause crashes under load (run a 100-frame
   SendDummy loop and confirm no panics in pod logs).

---

## 8. Open Questions Carried Into Implementation

1. **`prometheus` crate vs `metrics-rs` ecosystem** — `metrics` + `metrics-exporter-prometheus`
   gives a more idiomatic Rust experience and a vendor-neutral seam (could swap to OTLP
   metrics later). Recommended: stick with the lower-level `prometheus` crate for
   simplicity. Confirm during implementation.
2. **Sampling vs always-on tracing** — for local dev, always-on is fine. If trace volume
   is overwhelming under load tests, configure `opentelemetry::sdk::trace::Sampler::TraceIdRatioBased(0.1)`.
3. **OTLP gRPC vs HTTP** — gRPC default (port 4317). HTTP fallback (port 4318) if gRPC
   has issues with WSL networking. Confirm during implementation.
4. **Metric name conventions** — using `conduit_<service>_<metric>_total` naming. Some
   teams prefer no service prefix (use the `service` label). Recommended: keep the prefix
   for Grafana panel readability; query rewriting is trivial.
5. **`record_chain_id` call site coverage** — exact list of functions to instrument is
   determined by grepping for existing `chain_id = %` patterns. Tracked separately.
