//! CloudWatch custom metrics emission shared by Conduit V2 service binaries.

use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

pub const DEFAULT_CLOUDWATCH_NAMESPACE: &str = "Veritas/Conduit";
pub const DEFAULT_METRICS_PERIOD_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsEmitterConfig {
    pub namespace: String,
    pub service: String,
    pub stack: String,
    pub period: Duration,
}

impl MetricsEmitterConfig {
    pub fn from_env(service: impl Into<String>) -> Self {
        Self {
            namespace: std::env::var("GBN_BRIDGE_CLOUDWATCH_NAMESPACE")
                .unwrap_or_else(|_| DEFAULT_CLOUDWATCH_NAMESPACE.to_string()),
            service: service.into(),
            stack: std::env::var("GBN_BRIDGE_STACK_ENV").unwrap_or_else(|_| "dev".to_string()),
            period: Duration::from_secs(
                std::env::var("GBN_BRIDGE_METRICS_PERIOD_SECS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|value| *value > 0)
                    .unwrap_or(DEFAULT_METRICS_PERIOD_SECS),
            ),
        }
    }
}

pub fn cloudwatch_metrics_enabled() -> bool {
    match std::env::var("GBN_BRIDGE_CLOUDWATCH_ENABLED") {
        Ok(value) => parse_enabled_flag(&value),
        Err(_) => true,
    }
}

fn parse_enabled_flag(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

pub fn spawn_cloudwatch_emitter<F>(config: MetricsEmitterConfig, snapshot_fn: F) -> JoinHandle<()>
where
    F: Fn(&str, &str) -> Vec<MetricDatum> + Send + 'static,
{
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("cloudwatch metrics runtime init failed: {error}");
                return;
            }
        };

        runtime.block_on(async move {
            let aws_config =
                aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let client = aws_sdk_cloudwatch::Client::new(&aws_config);
            let mut interval = tokio::time::interval(config.period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;
                let data = snapshot_fn(&config.service, &config.stack);
                if data.is_empty() {
                    continue;
                }
                if let Err(error) = client
                    .put_metric_data()
                    .namespace(&config.namespace)
                    .set_metric_data(Some(data))
                    .send()
                    .await
                {
                    eprintln!(
                        "cloudwatch metrics put failed namespace={} service={} stack={} error={error}",
                        config.namespace, config.service, config.stack
                    );
                }
            }
        });
    })
}

pub fn metric_data(name: &str, value: f64, service: &str, stack: &str) -> MetricDatum {
    let dimensions = vec![
        Dimension::builder().name("Service").value(service).build(),
        Dimension::builder().name("Stack").value(stack).build(),
    ];

    MetricDatum::builder()
        .metric_name(name)
        .timestamp(aws_sdk_cloudwatch::primitives::DateTime::from(
            SystemTime::now(),
        ))
        .value(value)
        .unit(StandardUnit::Count)
        .set_dimensions(Some(dimensions))
        .build()
}

#[cfg(test)]
mod tests {
    use super::{metric_data, parse_enabled_flag};

    #[test]
    fn metric_data_sets_name_value_unit_and_dimensions() {
        let datum = metric_data("FramesAccepted", 7.0, "receiver", "dev");

        assert_eq!(datum.metric_name(), Some("FramesAccepted"));
        assert_eq!(datum.value(), Some(7.0));
        assert_eq!(datum.dimensions().len(), 2);
        assert!(datum
            .dimensions()
            .iter()
            .any(|dimension| dimension.name() == Some("Service")
                && dimension.value() == Some("receiver")));
        assert!(
            datum
                .dimensions()
                .iter()
                .any(|dimension| dimension.name() == Some("Stack")
                    && dimension.value() == Some("dev"))
        );
    }

    #[test]
    fn parse_enabled_flag_accepts_common_false_values() {
        for value in ["0", "false", "FALSE", " off ", "no"] {
            assert!(!parse_enabled_flag(value));
        }
        for value in ["1", "true", "yes", ""] {
            assert!(parse_enabled_flag(value));
        }
    }
}
