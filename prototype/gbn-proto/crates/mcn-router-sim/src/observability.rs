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
        self.publish_metric(
            "GossipBandwidthBytes",
            bytes as f64,
            StandardUnit::Bytes,
        )
        .await
    }

    pub async fn publish_circuit_build_result(&self, success: bool, latency_ms: u128) -> Result<()> {
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
            if let Err(e) = reporter.publish_circuit_build_result(success, latency_ms).await {
                tracing::warn!("CloudWatch publish CircuitBuildResult failed: {e}");
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
