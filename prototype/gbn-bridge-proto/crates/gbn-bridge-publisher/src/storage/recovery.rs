use crate::storage::InMemoryAuthorityStorage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecoverySummary {
    pub expired_bridges: usize,
    pub expired_bootstrap_sessions: usize,
    pub cleared_batch_windows: usize,
}

pub fn reconcile_recovered_state(
    storage: &mut InMemoryAuthorityStorage,
    now_ms: u64,
) -> RecoverySummary {
    let mut summary = RecoverySummary::default();

    let expired_bridge_ids = storage
        .bridges
        .iter()
        .filter_map(|(bridge_id, record)| {
            if record.current_lease.lease_expiry_ms < now_ms {
                Some(bridge_id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for bridge_id in expired_bridge_ids {
        if storage.bridges.remove(&bridge_id).is_some() {
            storage.publisher_bridge_dht_entries.remove(&bridge_id);
            summary.expired_bridges += 1;
        }
    }
    storage
        .publisher_bridge_dht_entries
        .retain(|bridge_id, _| storage.bridges.contains_key(bridge_id));

    let expired_bootstrap_ids = storage
        .bootstrap_sessions
        .iter()
        .filter_map(|(session_id, session)| {
            if session.response_expiry_ms < now_ms {
                Some(session_id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for session_id in expired_bootstrap_ids {
        if storage.bootstrap_sessions.remove(&session_id).is_some() {
            summary.expired_bootstrap_sessions += 1;
        }
    }

    if storage.current_batch.is_some() {
        storage.current_batch = None;
        summary.cleared_batch_windows += 1;
    }

    summary
}
