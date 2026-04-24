use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{BatchAssignment, CreatorJoinRequest};

use crate::bootstrap::creator_bootstrap_entry;
use crate::policy;
use crate::punch;
use crate::storage::{BatchWindowState, InMemoryAuthorityStorage, PendingBatchAssignment};
use crate::{AuthorityConfig, AuthorityError, AuthorityPolicy, AuthorityResult};

#[derive(Debug, Clone)]
pub struct FinalizedBatch {
    pub batch_id: String,
    pub assignments: Vec<BatchAssignment>,
    pub bridge_assignments: Vec<gbn_bridge_protocol::BridgeBatchAssign>,
}

pub fn enqueue_join_request(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    policy: &AuthorityPolicy,
    chain_id: Option<&str>,
    request: CreatorJoinRequest,
    now_ms: u64,
) -> AuthorityResult<Option<FinalizedBatch>> {
    let finalized = if let Some(batch) = &storage.current_batch {
        let batch_expired =
            now_ms.saturating_sub(batch.window_started_at_ms) >= config.batch_window_ms;
        let batch_full = batch.assignments.len() >= config.batch_capacity;
        if batch_expired || batch_full {
            Some(finalize_current_batch(
                storage,
                signing_key,
                config,
                policy,
                batch.window_started_at_ms,
            )?)
        } else {
            None
        }
    } else {
        None
    };

    let creator_entry = creator_bootstrap_entry(&request, signing_key, config, now_ms)?;
    let pending = PendingBatchAssignment {
        bootstrap_session_id: storage.next_bootstrap_id(),
        chain_id: chain_id.map(ToOwned::to_owned),
        join_request: request,
        creator_entry,
    };

    let next_batch_id = if storage.current_batch.is_none() {
        Some(storage.next_batch_id())
    } else {
        None
    };

    let batch = storage
        .current_batch
        .get_or_insert_with(|| BatchWindowState {
            batch_id: next_batch_id.expect("batch id should be reserved before insertion"),
            window_started_at_ms: now_ms,
            assignments: Vec::new(),
        });
    batch.assignments.push(pending);

    Ok(finalized)
}

pub fn flush_ready_batch(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    policy: &AuthorityPolicy,
    now_ms: u64,
) -> AuthorityResult<Option<FinalizedBatch>> {
    let Some(batch) = &storage.current_batch else {
        return Ok(None);
    };

    if now_ms.saturating_sub(batch.window_started_at_ms) < config.batch_window_ms {
        return Ok(None);
    }

    finalize_current_batch(
        storage,
        signing_key,
        config,
        policy,
        batch.window_started_at_ms,
    )
    .map(Some)
}

fn finalize_current_batch(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    policy: &AuthorityPolicy,
    window_started_at_ms: u64,
) -> AuthorityResult<FinalizedBatch> {
    let batch = storage
        .current_batch
        .take()
        .expect("current batch should exist when finalized");

    let bridge_ids = policy::bootstrap_candidates(storage, window_started_at_ms, policy)
        .into_iter()
        .take(config.bootstrap_bridge_count)
        .map(|record| record.bridge_id)
        .collect::<Vec<_>>();
    if bridge_ids.is_empty() {
        return Err(AuthorityError::NoEligibleBatchBridge);
    }

    let assignments = batch
        .assignments
        .into_iter()
        .map(|pending| BatchAssignment {
            chain_id: pending
                .chain_id
                .clone()
                .unwrap_or_else(|| pending.join_request.chain_id.clone()),
            bootstrap_session_id: pending.bootstrap_session_id,
            creator: pending.creator_entry,
            requested_bridge_count: config.bootstrap_bridge_count as u16,
        })
        .collect::<Vec<_>>();

    let bridge_assignments = punch::sign_batch_assignments(
        signing_key,
        &format!("batch-{}", batch.batch_id),
        &batch.batch_id,
        &bridge_ids,
        &assignments,
        config,
        window_started_at_ms,
    )?;

    Ok(FinalizedBatch {
        batch_id: batch.batch_id,
        assignments,
        bridge_assignments,
    })
}
