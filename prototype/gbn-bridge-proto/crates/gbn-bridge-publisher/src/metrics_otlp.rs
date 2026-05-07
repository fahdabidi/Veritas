//! Optional OTLP tracing setup for local Kubernetes observability.

use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace, Resource};
use tokio::runtime::{Builder, Runtime};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const DEFAULT_OTLP_ENDPOINT: &str = "http://tempo.observability.svc.cluster.local:4317";

pub struct OtlpTracingGuard {
    _runtime: Runtime,
}

impl Drop for OtlpTracingGuard {
    fn drop(&mut self) {
        opentelemetry::global::shutdown_tracer_provider();
    }
}

pub fn init_otlp_tracing_from_env(service_name: &str) -> Result<Option<OtlpTracingGuard>, String> {
    if matches_env_false("GBN_BRIDGE_OTLP_ENABLED") {
        return Ok(None);
    }

    let endpoint = std::env::var("GBN_BRIDGE_OTLP_ENDPOINT")
        .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
        .or_else(|_| std::env::var("OTLP_ENDPOINT"))
        .ok();
    let Some(endpoint) = endpoint else {
        return Ok(None);
    };
    let endpoint = if endpoint.trim().is_empty() {
        DEFAULT_OTLP_ENDPOINT.to_string()
    } else {
        endpoint
    };

    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(endpoint);
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .map_err(|error| format!("failed to create OTLP runtime: {error}"))?;
    let tracer =
        {
            let _enter = runtime.enter();
            opentelemetry_otlp::new_pipeline()
                .tracing()
                .with_exporter(exporter)
                .with_trace_config(trace::config().with_resource(Resource::new(vec![
                    KeyValue::new("service.name", service_name.to_string()),
                ])))
                .install_batch(opentelemetry_sdk::runtime::Tokio)
                .map_err(|error| format!("failed to install OTLP tracer: {error}"))?
        };
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(otel_layer)
        .try_init()
        .map_err(|error| format!("failed to initialize tracing subscriber: {error}"))?;

    Ok(Some(OtlpTracingGuard { _runtime: runtime }))
}

pub fn chain_span(operation: &'static str, chain_id: &str) -> tracing::Span {
    tracing::info_span!("conduit_chain", operation = operation, chain_id = %chain_id)
}

pub fn record_chain_id(chain_id: &str) {
    tracing::Span::current().record("chain_id", tracing::field::display(chain_id));
    tracing::info!(chain_id = %chain_id, "conduit chain event");
}

fn matches_env_false(key: &str) -> bool {
    std::env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off"
            )
        })
        .unwrap_or(false)
}
