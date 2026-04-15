use anyhow::Result;
use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};
use std::env;

const DEFAULT_NAMESPACE: &str = "GBN/ScaleTest";

#[derive(Debug, Clone)]
pub struct MetricsReporter {
    client: aws_sdk_cloudwatch::Client,
    namespace: String,
    dimensions: Vec<Dimension>,
}

impl MetricsReporter {
    pub async fn from_env() -> Result<Self> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = aws_sdk_cloudwatch::Client::new(&config);
        Ok(Self {
            client,
            namespace: env::var("GBN_METRICS_NAMESPACE")
                .unwrap_or_else(|_| DEFAULT_NAMESPACE.to_string()),
            dimensions: default_dimensions(),
        })
    }

    async fn publish_metric(
        &self,
        metric_name: &str,
        value: f64,
        unit: StandardUnit,
    ) -> Result<()> {
        let datum = MetricDatum::builder()
            .metric_name(metric_name)
            .value(value)
            .unit(unit)
            .set_dimensions(Some(self.dimensions.clone()))
            .build();

        self.client
            .put_metric_data()
            .namespace(&self.namespace)
            .metric_data(datum)
            .send()
            .await?;

        Ok(())
    }

    pub async fn publish_bootstrap_result(&self, active: bool) -> Result<()> {
        self.publish_metric(
            "BootstrapResult",
            if active { 1.0 } else { 0.0 },
            StandardUnit::Count,
        )
        .await
    }

    pub async fn publish_gossip_bandwidth_bytes(&self, bytes: u64) -> Result<()> {
        // Publish as an aggregate metric (Scale + Subnet only, no NodeId) so the
        // teardown SEARCH expression never hits CloudWatch's 500-series-per-request
        // limit.  After 5 × N=100 runs the per-NodeId series count reached exactly
        // 500, causing the SEARCH to return only stale series and report 0 bytes.
        // Removing NodeId caps the series count at (distinct Subnets × distinct
        // Scale values), typically 1–3 entries, regardless of how many runs have run.
        let datum = MetricDatum::builder()
            .metric_name("GossipBandwidthBytes")
            .value(bytes as f64)
            .unit(StandardUnit::Bytes)
            .set_dimensions(Some(aggregate_dimensions()))
            .build();
        self.client
            .put_metric_data()
            .namespace(&self.namespace)
            .metric_data(datum)
            .send()
            .await?;
        Ok(())
    }

    pub async fn publish_chunks_delivered(&self, count: u64) -> Result<()> {
        self.publish_metric("ChunksDelivered", count as f64, StandardUnit::Count)
            .await
    }

    pub async fn publish_circuit_build_result(
        &self,
        success: bool,
        latency_ms: u128,
    ) -> Result<()> {
        self.publish_metric(
            "CircuitBuildResult",
            if success { 1.0 } else { 0.0 },
            StandardUnit::Count,
        )
        .await?;

        self.publish_metric(
            "CircuitBuildLatencyMs",
            latency_ms as f64,
            StandardUnit::Milliseconds,
        )
        .await
    }

    /// Publish `ChunksReceived` — aggregate {Scale, Subnet} (no NodeId).
    /// Published by exit relays each time they forward a chunk to the Publisher.
    pub async fn publish_chunks_received(&self, count: u64) -> Result<()> {
        let datum = MetricDatum::builder()
            .metric_name("ChunksReceived")
            .value(count as f64)
            .unit(StandardUnit::Count)
            .set_dimensions(Some(aggregate_dimensions()))
            .build();
        self.client
            .put_metric_data()
            .namespace(&self.namespace)
            .metric_data(datum)
            .send()
            .await?;
        Ok(())
    }

    /// Publish `ChunksReassembled` and `HashMatchResult` — aggregate {Scale, Subnet}.
    /// Published by Publisher after each complete session is reassembled and verified.
    pub async fn publish_chunks_reassembled(&self, count: u64, hash_match: bool) -> Result<()> {
        let reassembled_datum = MetricDatum::builder()
            .metric_name("ChunksReassembled")
            .value(count as f64)
            .unit(StandardUnit::Count)
            .set_dimensions(Some(aggregate_dimensions()))
            .build();
        let hash_datum = MetricDatum::builder()
            .metric_name("HashMatchResult")
            .value(if hash_match { 1.0 } else { 0.0 })
            .unit(StandardUnit::Count)
            .set_dimensions(Some(aggregate_dimensions()))
            .build();
        self.client
            .put_metric_data()
            .namespace(&self.namespace)
            .metric_data(reassembled_datum)
            .metric_data(hash_datum)
            .send()
            .await?;
        Ok(())
    }

    /// Publish `PathDiversityResult` — per-node {Scale, NodeId}.
    /// Published by Creator after verifying all 10 circuit paths are disjoint.
    pub async fn publish_path_diversity(&self, all_disjoint: bool) -> Result<()> {
        self.publish_metric(
            "PathDiversityResult",
            if all_disjoint { 1.0 } else { 0.0 },
            StandardUnit::Count,
        )
        .await
    }
}

pub async fn publish_bootstrap_result_from_env(active: bool) {
    match MetricsReporter::from_env().await {
        Ok(reporter) => {
            if let Err(e) = reporter.publish_bootstrap_result(active).await {
                tracing::warn!("CloudWatch publish BootstrapResult failed: {e}");
            }
        }
        Err(e) => tracing::warn!("CloudWatch MetricsReporter init failed: {e}"),
    }
}

pub async fn publish_circuit_build_result_from_env(success: bool, latency_ms: u128) {
    match MetricsReporter::from_env().await {
        Ok(reporter) => {
            if let Err(e) = reporter
                .publish_circuit_build_result(success, latency_ms)
                .await
            {
                tracing::warn!("CloudWatch publish CircuitBuildResult failed: {e}");
            }
        }
        Err(e) => tracing::warn!("CloudWatch MetricsReporter init failed: {e}"),
    }
}

/// Fire-and-forget: publish ChunksReceived metric (exit relay → Publisher hop).
pub async fn publish_chunks_received_from_env(count: u64) {
    match MetricsReporter::from_env().await {
        Ok(reporter) => {
            if let Err(e) = reporter.publish_chunks_received(count).await {
                tracing::warn!("CloudWatch publish ChunksReceived failed: {e}");
            }
        }
        Err(e) => tracing::warn!("CloudWatch MetricsReporter init failed: {e}"),
    }
}

/// Fire-and-forget: publish ChunksReassembled + HashMatchResult (Publisher).
pub async fn publish_chunks_reassembled_from_env(count: u64, hash_match: bool) {
    match MetricsReporter::from_env().await {
        Ok(reporter) => {
            if let Err(e) = reporter.publish_chunks_reassembled(count, hash_match).await {
                tracing::warn!("CloudWatch publish ChunksReassembled failed: {e}");
            }
        }
        Err(e) => tracing::warn!("CloudWatch MetricsReporter init failed: {e}"),
    }
}

/// Fire-and-forget: publish PathDiversityResult (Creator, per-node).
pub async fn publish_path_diversity_from_env(all_disjoint: bool) {
    match MetricsReporter::from_env().await {
        Ok(reporter) => {
            if let Err(e) = reporter.publish_path_diversity(all_disjoint).await {
                tracing::warn!("CloudWatch publish PathDiversityResult failed: {e}");
            }
        }
        Err(e) => tracing::warn!("CloudWatch MetricsReporter init failed: {e}"),
    }
}

fn default_dimensions() -> Vec<Dimension> {
    let scale = env::var("GBN_SCALE").unwrap_or_else(|_| "Unknown".to_string());
    let subnet = env::var("GBN_SUBNET_TAG").unwrap_or_else(|_| "Unknown".to_string());
    let node_id = env::var("GBN_NODE_ID")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown-node".to_string());

    vec![
        Dimension::builder().name("Scale").value(scale).build(),
        Dimension::builder().name("Subnet").value(subnet).build(),
        Dimension::builder().name("NodeId").value(node_id).build(),
    ]
}

/// Dimensions without NodeId — used for aggregate metrics that must remain
/// queryable via CloudWatch SEARCH after many per-node runs accumulate.
fn aggregate_dimensions() -> Vec<Dimension> {
    let scale = env::var("GBN_SCALE").unwrap_or_else(|_| "Unknown".to_string());
    let subnet = env::var("GBN_SUBNET_TAG").unwrap_or_else(|_| "Unknown".to_string());
    vec![
        Dimension::builder().name("Scale").value(scale).build(),
        Dimension::builder().name("Subnet").value(subnet).build(),
    ]
}
