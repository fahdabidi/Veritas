# GBN-PROTO-007 - Execution Phase 3 Detailed Plan: CloudWatch Metrics Emission

**Status:** Pending — depends on Phase 1 landing first
**Primary Goal:** make all three Conduit V2 service binaries (publisher-authority,
publisher-receiver, exit-bridge) emit per-service metrics to the CloudWatch namespace
`Veritas/Conduit` on a 60-second cadence so V1's `LiveMetrics` dashboard pattern can be
ported in Phase 5. Add receiver and bridge in-memory metric structs analogous to the
existing `AuthorityMetricsSnapshot` so Phase 1's `/v1/admin/metrics` endpoint returns
non-stub data on every binary.
**Source Plan:** [GBN-PROTO-007 Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Existing authority metrics | [`AuthorityMetricsSnapshot`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs) — 10 `u64` fields, in-memory only | template for the receiver and bridge snapshots; reused as-is |
| Receiver metrics | none discoverable in current code | must be added in Phase 3 |
| Bridge metrics | none discoverable in current code | must be added in Phase 3 |
| CloudWatch SDK in workspace | not present | new dependency; add `aws-sdk-cloudwatch` and `aws-config` to workspace |
| TaskExecutionRole IAM policies | [conduit-full-stack.yaml:106-130](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L106-L130) — currently grants ECS task execution + Secrets Manager read | needs an additional inline policy granting `cloudwatch:PutMetricData` |
| Existing service main loops | each binary has an async runtime loop; no periodic-task helper visible | Phase 3 spawns a `tokio::time::interval` task in each binary's main |
| ECS task role | ServiceTaskRole at [conduit-full-stack.yaml:132-142](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L132-L142) — currently has no inline policies | attach the `cloudwatch:PutMetricData` policy here (task role, not execution role) |

**Note on IAM role choice:** AWS distinguishes the *task execution role* (used by the ECS
agent to pull images, write logs) from the *task role* (used by the running container
itself). `cloudwatch:PutMetricData` is performed by the running service code, so it
belongs on the **task role** (`ServiceTaskRole`), not the execution role.

---

## 2. Review Summary

| Gap | Why It Matters | Resolution For Phase 3 |
|---|---|---|
| No CloudWatch metrics emitted anywhere | Phase 5's `LiveMetrics` cannot read what is not published | add a 60s emitter task in each service binary |
| Receiver and bridge have no in-memory metric struct | Phase 1's `/v1/admin/metrics` returns stubs from those binaries | add `ReceiverMetricsSnapshot` and `BridgeMetricsSnapshot` structs |
| Workspace lacks AWS SDK deps | nothing to call CloudWatch with | add `aws-config` and `aws-sdk-cloudwatch` to workspace |
| TaskRole has no IAM permission for CW | even with code, runtime calls would 403 | add inline policy in CloudFormation |
| Cost increase is real but bounded | $0.30/metric/month × ~15 metrics = ~$4.50/month | acceptable per §3.5 of GBN-PROTO-007 main plan |

---

## 3. Scope Lock

### In Scope

- new workspace deps: `aws-config`, `aws-sdk-cloudwatch`
- new file: `gbn-bridge-publisher/src/metrics_emitter.rs` — a generic emitter that takes a
  `Fn() -> Vec<MetricDatum>` snapshot fetcher and emits every 60s
- new structs: `ReceiverMetricsSnapshot`, `BridgeMetricsSnapshot` analogous to
  `AuthorityMetricsSnapshot`
- new helper: `MetricsBuilder::cloudwatch_data(&self) -> Vec<MetricDatum>` per snapshot
  type
- spawn the emitter once in each of the three service binaries
- CloudFormation policy delta on ServiceTaskRole granting `cloudwatch:PutMetricData`
- Phase 1's `/v1/admin/metrics` handler is updated on receiver + bridge binaries to
  return the new snapshots instead of stubs

### Out Of Scope

- CloudWatch Logs emission (already happens via the awslogs driver; Phase 3 only adds
  custom metrics)
- CloudWatch Alarms / dashboards (operator can build these; Phase 3 ships only the
  underlying metric stream)
- per-bridge or per-task dimensions beyond `{Service, Stack}` (operator can drill via
  CloudWatch dimension filters; finer grain is a follow-up)
- changing the existing AuthorityMetrics behavior in Phase 1's surface

---

## 4. Preflight Gates

1. Phase 1 has landed; `/v1/admin/metrics` is reachable on every binary.
2. AWS credentials default-chain works inside the ECS task (it does — the task role is
   automatically assumed via the ECS metadata endpoint).
3. CloudFormation template still validates.
4. `cargo fmt --all --check` and `cargo test --workspace` pass.

---

## 5. File-by-File Specification

### 5.1 New file: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics_emitter.rs`

```rust
//! Periodic CloudWatch metrics emitter shared by all three V2 service binaries.

use std::sync::Arc;
use std::time::Duration;

use aws_sdk_cloudwatch::primitives::DateTime;
use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};
use aws_sdk_cloudwatch::Client as CloudWatchClient;

pub struct MetricsEmitterConfig {
    pub namespace: String,           // "Veritas/Conduit"
    pub service_dimension: String,   // "authority" | "receiver" | "bridge"
    pub stack_dimension: String,     // EnvironmentName from env
    pub period: Duration,            // Duration::from_secs(60)
}

/// Spawn a tokio task that wakes on `config.period`, calls `snapshot_fn`, and PutMetricData.
pub fn spawn_emitter<F>(
    client: CloudWatchClient,
    config: MetricsEmitterConfig,
    snapshot_fn: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn() -> Vec<MetricDatum> + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let data = snapshot_fn();
            if data.is_empty() { continue; }
            let _ = client.put_metric_data()
                .namespace(&config.namespace)
                .set_metric_data(Some(data))
                .send()
                .await;
            // Errors are logged at warn level; do NOT panic the service on CW failure.
        }
    })
}

pub fn build_dimensions(service: &str, stack: &str) -> Vec<Dimension> {
    vec![
        Dimension::builder().name("Service").value(service).build(),
        Dimension::builder().name("Stack").value(stack).build(),
    ]
}
```

### 5.2 Modify: `gbn-bridge-publisher/src/metrics.rs`

Existing `AuthorityMetricsSnapshot` gains a CW conversion method:

```rust
impl AuthorityMetricsSnapshot {
    pub fn cloudwatch_data(&self, service: &str, stack: &str) -> Vec<MetricDatum> {
        let dims = build_dimensions(service, stack);
        let now = DateTime::from(std::time::SystemTime::now());
        let mut data = Vec::with_capacity(10);
        for (name, value) in [
            ("SuccessfulRegistrations", self.successful_registrations as f64),
            ("RejectedRegistrations", self.rejected_registrations as f64),
            ("Heartbeats", self.heartbeats as f64),
            ("Revocations", self.revocations as f64),
            ("IssuedCatalogs", self.issued_catalogs as f64),
            ("BootstrapRequests", self.bootstrap_requests as f64),
            ("RejectedBootstrapRequests", self.rejected_bootstrap_requests as f64),
            ("BootstrapProgressReports", self.bootstrap_progress_reports as f64),
            ("IssuedBatches", self.issued_batches as f64),
            ("BatchRollovers", self.batch_rollovers as f64),
        ] {
            data.push(
                MetricDatum::builder()
                    .metric_name(name)
                    .timestamp(now.clone())
                    .value(value)
                    .unit(StandardUnit::Count)
                    .set_dimensions(Some(dims.clone()))
                    .build(),
            );
        }
        data
    }
}
```

Append two new structs alongside the authority one:

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

Each has its own builder + `cloudwatch_data` method following the same pattern as the
authority snapshot.

### 5.3 Modify: workspace `Cargo.toml`

Add to `[workspace.dependencies]`:

```toml
aws-config = "1"
aws-sdk-cloudwatch = "1"
```

Pin to specific minor versions matching what already exists in the workspace if other
`aws-*` crates are present.

### 5.4 Modify: `gbn-bridge-publisher/Cargo.toml`

Add to `[dependencies]`:

```toml
aws-config = { workspace = true }
aws-sdk-cloudwatch = { workspace = true }
```

### 5.5 Modify: each service binary `main`

In `crates/gbn-bridge-cli/src/bin/publisher-authority.rs`:

After admin listener spawn (Phase 1 §5.7), add:

```rust
let aws_config = aws_config::load_from_env().await;
let cw_client = aws_sdk_cloudwatch::Client::new(&aws_config);
let stack = std::env::var("GBN_BRIDGE_STACK_ENV")
    .unwrap_or_else(|_| "dev".to_string());
let emitter_config = gbn_bridge_publisher::metrics_emitter::MetricsEmitterConfig {
    namespace: "Veritas/Conduit".into(),
    service_dimension: "authority".into(),
    stack_dimension: stack.clone(),
    period: std::time::Duration::from_secs(60),
};
let metrics_for_emitter = metrics.clone();
let _emitter_handle = gbn_bridge_publisher::metrics_emitter::spawn_emitter(
    cw_client,
    emitter_config,
    move || {
        let snapshot = metrics_for_emitter.lock().now_or_never()
            .and_then(|guard| Some(guard.snapshot()))
            .unwrap_or_default();
        snapshot.cloudwatch_data("authority", &stack)
    },
);
```

Receiver binary: same wiring with `service_dimension: "receiver"` and snapshot from a new
`ReceiverMetrics` struct held by the receiver service.

Bridge binary: same with `service_dimension: "bridge"` and `BridgeMetrics`.

### 5.6 Modify: `prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml`

Existing `ServiceTaskRole` at
[lines 132-142](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L132-L142)
currently has no inline policies. Add one:

```yaml
  ServiceTaskRole:
    Type: AWS::IAM::Role
    Properties:
      RoleName: !Sub "gbn-conduit-full-${EnvironmentName}-task"
      AssumeRolePolicyDocument:
        Version: "2012-10-17"
        Statement:
          - Effect: Allow
            Principal:
              Service: ecs-tasks.amazonaws.com
            Action: sts:AssumeRole
      Policies:
        - PolicyName: conduit-full-cloudwatch-metrics
          PolicyDocument:
            Version: "2012-10-17"
            Statement:
              - Effect: Allow
                Action:
                  - cloudwatch:PutMetricData
                Resource: "*"
                Condition:
                  StringEquals:
                    "cloudwatch:namespace": "Veritas/Conduit"
```

The `Condition` element scopes the permission to only the `Veritas/Conduit` namespace,
keeping IAM minimal.

Each task definition's environment block gains `GBN_BRIDGE_STACK_ENV`:

```yaml
- Name: GBN_BRIDGE_STACK_ENV
  Value: !Ref EnvironmentName
```

Add this to:
- AuthorityTaskDefinition Environment list at
  [line 341](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L341)
- ReceiverTaskDefinition Environment list at
  [line 389](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L389)
- BridgeTaskDefinition Environment list at
  [line 419](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L419)

### 5.7 Modify: receiver and bridge binaries' `/v1/admin/metrics` handler

In Phase 1, receiver and bridge served a stub all-zero snapshot. In Phase 3, they serve
their real `ReceiverMetricsSnapshot` / `BridgeMetricsSnapshot`.

The Phase 1 `MetricsResponse` struct gains two optional variants:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "service", content = "snapshot")]
pub enum MetricsResponse {
    Authority(AuthorityMetricsSnapshot),
    Receiver(ReceiverMetricsSnapshot),
    Bridge(BridgeMetricsSnapshot),
}
```

Each binary picks the variant matching its service.

### 5.8 New file: `gbn-bridge-publisher/tests/metrics_emitter.rs`

```rust
//! Phase 3 emitter test — uses a fake CloudWatch client to assert the emitter calls
//! PutMetricData with the expected namespace, dimensions, and metric names every period.

#[tokio::test]
async fn emitter_publishes_authority_metrics_every_period() { ... }

#[tokio::test]
async fn emitter_skips_empty_snapshots() { ... }

#[tokio::test]
async fn emitter_recovers_from_put_metric_data_failure() { ... }
```

Use `aws-sdk-cloudwatch`'s test interceptor / mocking pattern (or a hand-rolled trait
abstraction over the client) to verify call shape without hitting AWS.

---

## 6. Validation

1. `cargo fmt --all --check` and `cargo test --workspace` pass.
2. Phase 3 tests pass.
3. Build and push the three updated container images.
4. Deploy `gbn-conduit-full-dev` stack.
5. Within 3 minutes, confirm in the AWS CloudWatch console (or via `aws cloudwatch
   list-metrics --namespace Veritas/Conduit`) that:
   - 10 metrics exist with `Service=authority`
   - 5 metrics exist with `Service=receiver`
   - 6 metrics exist with `Service=bridge`
   - all metrics have `Stack=dev` dimension
6. `aws cloudwatch get-metric-statistics --namespace Veritas/Conduit --metric-name
   SuccessfulRegistrations --statistics Sum --start-time <-5m> --end-time <now>
   --period 60 --dimensions Name=Service,Value=authority Name=Stack,Value=dev` returns at
   least one datapoint.
7. From the receiver container: `curl -s http://127.0.0.1:9090/v1/admin/metrics` returns
   `{"service":"Receiver","snapshot":{...real fields...}}`.
8. Tear down stack; confirm CloudWatch metrics stop arriving (and the metric history
   remains queryable for the standard 15-month retention).
9. Cost note: confirm the additional metric line items in Cost Explorer next billing day
   stay under $0.20/day for one stack.

---

## 7. Open Questions Carried Into Implementation

1. **Metric resolution** — 60s standard resolution ($0.30/metric/month) vs 1s high-resolution
   ($1.50/metric/month). Recommended: 60s. Confirm.
2. **Dimension cardinality** — Phase 3 uses only `Service` + `Stack`. Should `BridgeId` be
   added as a third dimension on bridge metrics? Risk: cardinality explosion if many
   bridges. Recommend: defer per-bridge dimensions; add only if operator explicitly needs
   per-bridge graphs.
3. **Emitter shutdown** — does Phase 3 need a graceful-shutdown path that flushes pending
   metrics on SIGTERM? Recommend: skip; CloudWatch tolerates last-period gaps.
4. **Receiver / Bridge metric definitions** — confirm the listed counters are the right
   ones for each service before implementation. The receiver counters (`frames_accepted`,
   etc.) and bridge counters (`commands_received`, `frames_forwarded`, etc.) are best
   guesses from existing receiver/bridge code; align with what the team actually wants
   to track.
