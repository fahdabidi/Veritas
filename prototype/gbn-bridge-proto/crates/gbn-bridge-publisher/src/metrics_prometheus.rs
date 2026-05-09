//! Prometheus exposition helpers for Conduit V2 service metrics.

use crate::metrics::{AuthorityMetricsSnapshot, BridgeMetricsSnapshot, ReceiverMetricsSnapshot};

pub const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

pub fn stack_from_env() -> String {
    std::env::var("GBN_BRIDGE_STACK_ENV").unwrap_or_else(|_| "dev".to_string())
}

pub fn authority_metrics_text(
    snapshot: &AuthorityMetricsSnapshot,
    service: &str,
    stack: &str,
) -> String {
    let mut out = String::new();
    write_metric(
        &mut out,
        "conduit_authority_successful_registrations_total",
        "Successful bridge registrations accepted by the authority.",
        snapshot.successful_registrations,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_rejected_registrations_total",
        "Bridge registrations rejected by the authority.",
        snapshot.rejected_registrations,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_heartbeats_total",
        "Bridge heartbeat requests accepted by the authority.",
        snapshot.heartbeats,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_revocations_total",
        "Bridge revocations recorded by the authority.",
        snapshot.revocations,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_issued_catalogs_total",
        "Creator catalogs issued by the authority.",
        snapshot.issued_catalogs,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_bootstrap_requests_total",
        "Bootstrap join requests handled by the authority.",
        snapshot.bootstrap_requests,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_rejected_bootstrap_requests_total",
        "Bootstrap join requests rejected by the authority.",
        snapshot.rejected_bootstrap_requests,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_bootstrap_progress_reports_total",
        "Bootstrap progress reports accepted by the authority.",
        snapshot.bootstrap_progress_reports,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_issued_batches_total",
        "Batch assignments issued by the authority.",
        snapshot.issued_batches,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_authority_batch_rollovers_total",
        "Batch assignment rollovers performed by the authority.",
        snapshot.batch_rollovers,
        service,
        stack,
    );
    out
}

pub fn receiver_metrics_text(
    snapshot: &ReceiverMetricsSnapshot,
    service: &str,
    stack: &str,
) -> String {
    let mut out = String::new();
    write_metric(
        &mut out,
        "conduit_receiver_frames_accepted_total",
        "Receiver frame requests accepted by the publisher.",
        snapshot.frames_accepted,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_receiver_frames_rejected_total",
        "Receiver frame requests rejected by the publisher.",
        snapshot.frames_rejected,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_receiver_bytes_ingested_total",
        "Receiver payload bytes accepted by the publisher.",
        snapshot.bytes_ingested,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_receiver_sessions_opened_total",
        "Receiver sessions opened by the publisher.",
        snapshot.sessions_opened,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_receiver_sessions_closed_total",
        "Receiver sessions closed by the publisher.",
        snapshot.sessions_closed,
        service,
        stack,
    );
    out
}

pub fn bridge_metrics_text(snapshot: &BridgeMetricsSnapshot, service: &str, stack: &str) -> String {
    let mut out = String::new();
    write_metric(
        &mut out,
        "conduit_bridge_commands_received_total",
        "Admin control commands observed by the bridge.",
        snapshot.commands_received,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_bridge_commands_acked_total",
        "Admin control commands acknowledged by the bridge.",
        snapshot.commands_acked,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_bridge_commands_rejected_total",
        "Admin control commands rejected by the bridge.",
        snapshot.commands_rejected,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_bridge_frames_forwarded_total",
        "Creator frames forwarded by the bridge.",
        snapshot.frames_forwarded,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_bridge_bytes_forwarded_total",
        "Creator payload bytes forwarded by the bridge.",
        snapshot.bytes_forwarded,
        service,
        stack,
    );
    write_metric(
        &mut out,
        "conduit_bridge_control_reconnects_total",
        "Bridge control-plane reconnects.",
        snapshot.control_reconnects,
        service,
        stack,
    );
    out
}

pub fn creator_metrics_text(actor_id: &str, service: &str, stack: &str) -> String {
    let actor_id = escape_label_value(actor_id);
    let service = escape_label_value(service);
    let stack = escape_label_value(stack);
    format!(
        "# HELP conduit_creator_info Creator admin runner readiness marker.\n\
         # TYPE conduit_creator_info gauge\n\
         conduit_creator_info{{service=\"{service}\",stack=\"{stack}\",actor_id=\"{actor_id}\"}} 1\n"
    )
}

fn write_metric(out: &mut String, name: &str, help: &str, value: u64, service: &str, stack: &str) {
    let service = escape_label_value(service);
    let stack = escape_label_value(stack);
    out.push_str("# HELP ");
    out.push_str(name);
    out.push(' ');
    out.push_str(help);
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(name);
    out.push_str(" counter\n");
    out.push_str(name);
    out.push_str("{service=\"");
    out.push_str(&service);
    out.push_str("\",stack=\"");
    out.push_str(&stack);
    out.push_str("\"} ");
    out.push_str(&value.to_string());
    out.push('\n');
}

fn escape_label_value(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_exposition_includes_counter_type_labels_and_value() {
        let snapshot = AuthorityMetricsSnapshot {
            successful_registrations: 2,
            rejected_registrations: 1,
            ..AuthorityMetricsSnapshot::default()
        };

        let body = authority_metrics_text(&snapshot, "authority", "dev-local");

        assert!(body.contains("# TYPE conduit_authority_successful_registrations_total counter"));
        assert!(body.contains(
            "conduit_authority_successful_registrations_total{service=\"authority\",stack=\"dev-local\"} 2"
        ));
        assert!(body.contains(
            "conduit_authority_rejected_registrations_total{service=\"authority\",stack=\"dev-local\"} 1"
        ));
    }

    #[test]
    fn receiver_and_bridge_exposition_use_phase_two_dashboard_names() {
        let receiver = receiver_metrics_text(
            &ReceiverMetricsSnapshot {
                frames_accepted: 3,
                bytes_ingested: 128,
                ..ReceiverMetricsSnapshot::default()
            },
            "receiver",
            "dev-local",
        );
        let bridge = bridge_metrics_text(
            &BridgeMetricsSnapshot {
                frames_forwarded: 4,
                bytes_forwarded: 256,
                ..BridgeMetricsSnapshot::default()
            },
            "bridge",
            "dev-local",
        );

        assert!(receiver.contains(
            "conduit_receiver_frames_accepted_total{service=\"receiver\",stack=\"dev-local\"} 3"
        ));
        assert!(receiver.contains(
            "conduit_receiver_bytes_ingested_total{service=\"receiver\",stack=\"dev-local\"} 128"
        ));
        assert!(bridge.contains(
            "conduit_bridge_frames_forwarded_total{service=\"bridge\",stack=\"dev-local\"} 4"
        ));
        assert!(bridge.contains(
            "conduit_bridge_bytes_forwarded_total{service=\"bridge\",stack=\"dev-local\"} 256"
        ));
    }

    #[test]
    fn label_values_are_escaped() {
        let body = bridge_metrics_text(&BridgeMetricsSnapshot::default(), "br\"idge", "dev\\local");

        assert!(body.contains("{service=\"br\\\"idge\",stack=\"dev\\\\local\"}"));
    }

    #[test]
    fn creator_exposition_includes_actor_info_gauge() {
        let body = creator_metrics_text("host-creator", "creator-host", "dev-local");

        assert!(body.contains("# TYPE conduit_creator_info gauge"));
        assert!(body.contains(
            "conduit_creator_info{service=\"creator-host\",stack=\"dev-local\",actor_id=\"host-creator\"} 1"
        ));
    }
}
