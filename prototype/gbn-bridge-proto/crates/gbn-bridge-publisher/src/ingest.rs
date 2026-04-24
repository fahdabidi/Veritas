use gbn_bridge_protocol::{BridgeAck, BridgeAckStatus, BridgeClose, BridgeData, BridgeOpen};

use crate::ack::build_ack;
use crate::storage::{InMemoryAuthorityStorage, IngestedFrameRecord, UploadSessionRecord};
use crate::{AuthorityError, AuthorityResult};

pub fn open_session(
    storage: &mut InMemoryAuthorityStorage,
    open: BridgeOpen,
) -> AuthorityResult<()> {
    open_session_with_chain_id(storage, None, open)
}

pub fn open_session_with_chain_id(
    storage: &mut InMemoryAuthorityStorage,
    chain_id: Option<&str>,
    open: BridgeOpen,
) -> AuthorityResult<()> {
    match storage.upload_sessions.get_mut(&open.session_id) {
        Some(existing) => {
            if existing.creator_id != open.creator_id {
                return Err(AuthorityError::UploadSessionCreatorMismatch {
                    session_id: open.session_id,
                    expected_creator_id: existing.creator_id.clone(),
                    actual_creator_id: open.creator_id,
                });
            }

            if existing.closed_at_ms.is_some() {
                return Err(AuthorityError::UploadSessionClosed {
                    session_id: existing.session_id.clone(),
                });
            }

            match (&existing.chain_id, chain_id) {
                (Some(expected), Some(actual)) if expected != actual => {
                    return Err(AuthorityError::ChainIdMismatch {
                        context: "upload session open",
                        expected: expected.clone(),
                        actual: actual.to_string(),
                    });
                }
                (None, Some(actual)) => {
                    existing.chain_id = Some(actual.to_string());
                }
                _ => {}
            }

            if !existing.opened_via_bridges.contains(&open.bridge_id) {
                existing.opened_via_bridges.push(open.bridge_id);
                existing.opened_via_bridges.sort();
            }

            Ok(())
        }
        None => {
            storage.upload_sessions.insert(
                open.session_id.clone(),
                UploadSessionRecord::new(&open, chain_id.map(ToOwned::to_owned)),
            );
            Ok(())
        }
    }
}

pub fn ingest_frame(
    storage: &mut InMemoryAuthorityStorage,
    via_bridge_id: &str,
    frame: BridgeData,
    received_at_ms: u64,
) -> AuthorityResult<BridgeAck> {
    ingest_frame_with_chain_id(storage, None, via_bridge_id, frame, received_at_ms)
}

pub fn ingest_frame_with_chain_id(
    storage: &mut InMemoryAuthorityStorage,
    chain_id: Option<&str>,
    via_bridge_id: &str,
    frame: BridgeData,
    received_at_ms: u64,
) -> AuthorityResult<BridgeAck> {
    let session = storage
        .upload_sessions
        .get_mut(&frame.session_id)
        .ok_or_else(|| AuthorityError::UploadSessionNotFound {
            session_id: frame.session_id.clone(),
        })?;

    if session.closed_at_ms.is_some() {
        return Err(AuthorityError::UploadSessionClosed {
            session_id: session.session_id.clone(),
        });
    }

    match (&session.chain_id, chain_id) {
        (Some(expected), Some(actual)) if expected != actual => {
            return Err(AuthorityError::ChainIdMismatch {
                context: "upload frame ingest",
                expected: expected.clone(),
                actual: actual.to_string(),
            });
        }
        (None, Some(actual)) => {
            session.chain_id = Some(actual.to_string());
        }
        _ => {}
    }

    if let Some(existing_sequence) = session.frame_id_to_sequence.get(&frame.frame_id) {
        let status = if session.completed_at_ms.is_some() {
            BridgeAckStatus::Complete
        } else {
            BridgeAckStatus::Duplicate
        };
        return Ok(build_ack(
            &session.session_id,
            *existing_sequence,
            status,
            received_at_ms,
        ));
    }

    if session.frames_by_sequence.contains_key(&frame.sequence) {
        return Ok(build_ack(
            &session.session_id,
            frame.sequence,
            BridgeAckStatus::Duplicate,
            received_at_ms,
        ));
    }

    session
        .frame_id_to_sequence
        .insert(frame.frame_id.clone(), frame.sequence);
    session.frames_by_sequence.insert(
        frame.sequence,
        IngestedFrameRecord {
            via_bridge_id: via_bridge_id.to_string(),
            chain_id: chain_id
                .map(ToOwned::to_owned)
                .or_else(|| session.chain_id.clone()),
            frame: frame.clone(),
            received_at_ms,
        },
    );

    let status = if frame.final_frame {
        session.completed_at_ms = Some(received_at_ms);
        BridgeAckStatus::Complete
    } else {
        BridgeAckStatus::Accepted
    };

    Ok(build_ack(
        &session.session_id,
        frame.sequence,
        status,
        received_at_ms,
    ))
}

pub fn close_session(
    storage: &mut InMemoryAuthorityStorage,
    close: BridgeClose,
) -> AuthorityResult<()> {
    close_session_with_chain_id(storage, None, close)
}

pub fn close_session_with_chain_id(
    storage: &mut InMemoryAuthorityStorage,
    chain_id: Option<&str>,
    close: BridgeClose,
) -> AuthorityResult<()> {
    let session = storage
        .upload_sessions
        .get_mut(&close.session_id)
        .ok_or_else(|| AuthorityError::UploadSessionNotFound {
            session_id: close.session_id.clone(),
        })?;

    match (&session.chain_id, chain_id) {
        (Some(expected), Some(actual)) if expected != actual => {
            return Err(AuthorityError::ChainIdMismatch {
                context: "upload session close",
                expected: expected.clone(),
                actual: actual.to_string(),
            });
        }
        (None, Some(actual)) => {
            session.chain_id = Some(actual.to_string());
        }
        _ => {}
    }

    session.closed_at_ms = Some(close.closed_at_ms);
    session.close_reason = Some(close.reason);
    Ok(())
}
