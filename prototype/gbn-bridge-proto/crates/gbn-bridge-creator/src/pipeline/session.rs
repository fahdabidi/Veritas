use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use gbn_bridge_protocol::{
    encrypt_for_publisher, EncryptedFrame, LocalDiscoveryTable, PublisherDhtEntry,
    SelfOnboardingState,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::chunker::{chunk, ChunkError};
use super::envelope::{session_id_bytes, session_id_hex, upload_ephemeral_private};
use super::manifest::UploadManifest;
use super::sanitizer::{sanitize, SanitizationReport, SanitizerFormatHint};

pub const MANIFEST_CHUNK_INDEX: u32 = 0xffff_ffff;
pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum SessionBuildError {
    #[error(transparent)]
    Chunk(#[from] ChunkError),

    #[error("creator is not onboarded: {0}")]
    CreatorNotOnboarded(String),

    #[error("local DHT is missing a publisher entry")]
    MissingPublisherEntry,

    #[error("session I/O error: {0}")]
    Io(String),

    #[error("session serialization error: {0}")]
    Serialization(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("upload session `{session_id}` not found")]
    NotFound { session_id: String },
}

impl From<std::io::Error> for SessionBuildError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for SessionBuildError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

impl From<gbn_bridge_protocol::ProtocolError> for SessionBuildError {
    fn from(value: gbn_bridge_protocol::ProtocolError) -> Self {
        Self::Protocol(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadSessionStatus {
    Built,
    Dispatching,
    Completed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UploadDispatchPlan {
    pub lanes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedUploadSession {
    pub session_id: String,
    pub session_id_bytes: Vec<u8>,
    pub chain_id: String,
    pub actor_id: String,
    pub manifest: UploadManifest,
    pub manifest_ciphertext: EncryptedFrame,
    pub chunk_ciphertexts: Vec<EncryptedFrame>,
    pub local_dht_snapshot: LocalDiscoveryTable,
    pub built_at_ms: u64,
    pub plan: UploadDispatchPlan,
    pub status: UploadSessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadSessionSummary {
    pub session_id: String,
    pub chain_id: String,
    pub actor_id: String,
    pub status: UploadSessionStatus,
    pub total_chunks: u32,
    pub chunk_size: u32,
    pub total_bytes: u64,
    pub content_hash: Vec<u8>,
    pub sanitization_profile: String,
    pub sanitization_report: SanitizationReport,
    pub built_at_ms: u64,
    pub ciphertext_chunk_count: usize,
    pub local_dht_bridge_count: usize,
}

#[derive(Debug, Clone)]
pub struct BuildUploadSessionOptions {
    pub chain_id: String,
    pub actor_id: String,
    pub plaintext: Vec<u8>,
    pub format_hint: SanitizerFormatHint,
    pub chunk_size: usize,
    pub sanitization_profile: String,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildUploadSessionResult {
    pub session: EncryptedUploadSession,
    pub summary: UploadSessionSummary,
}

pub fn build_upload_session(
    options: BuildUploadSessionOptions,
    publisher_entry: &PublisherDhtEntry,
    local_dht: &LocalDiscoveryTable,
) -> Result<BuildUploadSessionResult, SessionBuildError> {
    ensure_onboarded(local_dht)?;
    let sanitized = sanitize(&options.plaintext, options.format_hint);
    let chunked = chunk(
        &sanitized.bytes,
        if options.chunk_size == 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            options.chunk_size
        },
    )?;
    let session_id = session_id_bytes(
        &options.actor_id,
        &options.chain_id,
        &chunked.content_hash,
        chunked.chunk_size,
        options.now_ms,
    );
    let session_id_text = session_id_hex(&session_id);
    let ephemeral_private =
        upload_ephemeral_private(&options.actor_id, &session_id, &chunked.content_hash);

    let total_chunks = chunked.chunks.len() as u32;
    let creator_ephemeral_pubkey = encrypt_for_publisher(
        b"manifest-key-probe",
        &publisher_entry.pub_key,
        publisher_entry.node_id.clone(),
        session_id,
        MANIFEST_CHUNK_INDEX,
        total_chunks,
        ephemeral_private,
    )?
    .creator_ephemeral_pubkey;
    let manifest = UploadManifest {
        session_id: session_id.to_vec(),
        creator_ephemeral_pubkey,
        publisher_key_id: publisher_entry.node_id.clone(),
        total_chunks,
        content_hash: chunked.content_hash.clone(),
        sanitization_profile: options.sanitization_profile,
        sanitization_report: sanitized.report.clone(),
        created_at_ms: options.now_ms,
        chunk_size: chunked.chunk_size as u32,
        total_bytes: chunked.total_bytes,
    };
    let manifest_plaintext = serde_json::to_vec(&manifest)?;
    let manifest_ciphertext = encrypt_for_publisher(
        &manifest_plaintext,
        &publisher_entry.pub_key,
        publisher_entry.node_id.clone(),
        session_id,
        MANIFEST_CHUNK_INDEX,
        total_chunks,
        ephemeral_private,
    )?;
    let chunk_ciphertexts = chunked
        .chunks
        .iter()
        .map(|chunk| {
            encrypt_for_publisher(
                &chunk.plaintext,
                &publisher_entry.pub_key,
                publisher_entry.node_id.clone(),
                session_id,
                chunk.index,
                chunk.total,
                ephemeral_private,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let session = EncryptedUploadSession {
        session_id: session_id_text,
        session_id_bytes: session_id.to_vec(),
        chain_id: options.chain_id,
        actor_id: options.actor_id,
        manifest,
        manifest_ciphertext,
        chunk_ciphertexts,
        local_dht_snapshot: local_dht.clone(),
        built_at_ms: options.now_ms,
        plan: UploadDispatchPlan::default(),
        status: UploadSessionStatus::Built,
    };
    let summary = session.summary();
    Ok(BuildUploadSessionResult { session, summary })
}

pub fn build_upload_session_to_disk(
    base_state_dir: &Path,
    options: BuildUploadSessionOptions,
    publisher_entry: &PublisherDhtEntry,
    local_dht: &LocalDiscoveryTable,
) -> Result<BuildUploadSessionResult, SessionBuildError> {
    let result = build_upload_session(options, publisher_entry, local_dht)?;
    persist_upload_session(base_state_dir, &result.session)?;
    Ok(result)
}

pub fn list_upload_sessions(
    base_state_dir: &Path,
) -> Result<Vec<UploadSessionSummary>, SessionBuildError> {
    let root = upload_sessions_root(base_state_dir);
    let mut sessions = Vec::new();
    let Ok(entries) = fs::read_dir(&root) else {
        return Ok(sessions);
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let session = load_session_file(&entry.path())?;
            sessions.push(session.summary());
        }
    }
    sessions.sort_by(|left, right| left.built_at_ms.cmp(&right.built_at_ms));
    Ok(sessions)
}

pub fn get_upload_session(
    base_state_dir: &Path,
    session_id: &str,
) -> Result<UploadSessionSummary, SessionBuildError> {
    let session_dir = upload_sessions_root(base_state_dir).join(session_id);
    if !session_dir.exists() {
        return Err(SessionBuildError::NotFound {
            session_id: session_id.to_string(),
        });
    }
    Ok(load_session_file(&session_dir)?.summary())
}

pub fn delete_upload_session(
    base_state_dir: &Path,
    session_id: &str,
) -> Result<bool, SessionBuildError> {
    let session_dir = upload_sessions_root(base_state_dir).join(session_id);
    if !session_dir.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(session_dir)?;
    Ok(true)
}

impl EncryptedUploadSession {
    pub fn summary(&self) -> UploadSessionSummary {
        UploadSessionSummary {
            session_id: self.session_id.clone(),
            chain_id: self.chain_id.clone(),
            actor_id: self.actor_id.clone(),
            status: self.status.clone(),
            total_chunks: self.manifest.total_chunks,
            chunk_size: self.manifest.chunk_size,
            total_bytes: self.manifest.total_bytes,
            content_hash: self.manifest.content_hash.clone(),
            sanitization_profile: self.manifest.sanitization_profile.clone(),
            sanitization_report: self.manifest.sanitization_report.clone(),
            built_at_ms: self.built_at_ms,
            ciphertext_chunk_count: self.chunk_ciphertexts.len(),
            local_dht_bridge_count: self.local_dht_snapshot.bridge_entries.len(),
        }
    }
}

pub fn upload_sessions_root(base_state_dir: &Path) -> PathBuf {
    base_state_dir.join("upload_sessions")
}

fn persist_upload_session(
    base_state_dir: &Path,
    session: &EncryptedUploadSession,
) -> Result<(), SessionBuildError> {
    let session_dir = upload_sessions_root(base_state_dir).join(&session.session_id);
    let chunks_dir = session_dir.join("chunks");
    fs::create_dir_all(&chunks_dir)?;
    write_json_atomic(&session_dir.join("manifest.json"), &session.manifest)?;
    write_json_atomic(
        &session_dir.join("manifest_frame.json"),
        &session.manifest_ciphertext,
    )?;
    write_json_atomic(
        &session_dir.join("local_dht.json"),
        &session.local_dht_snapshot,
    )?;
    write_json_atomic(&session_dir.join("session.json"), session)?;
    for frame in &session.chunk_ciphertexts {
        let path = chunks_dir.join(format!("{:06}.bin", frame.chunk_index));
        write_json_atomic(&path, frame)?;
    }
    Ok(())
}

fn load_session_file(session_dir: &Path) -> Result<EncryptedUploadSession, SessionBuildError> {
    let raw = fs::read(session_dir.join("session.json"))?;
    Ok(serde_json::from_slice(&raw)?)
}

fn write_json_atomic<T>(path: &Path, value: &T) -> Result<(), SessionBuildError>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let body = serde_json::to_vec_pretty(value)?;
    {
        let mut file = File::create(&tmp)?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn ensure_onboarded(table: &LocalDiscoveryTable) -> Result<(), SessionBuildError> {
    if matches!(
        table.self_onboarding_state,
        SelfOnboardingState::Onboarded | SelfOnboardingState::FanoutPartial
    ) {
        return Ok(());
    }
    let state = serde_json::to_value(table.self_onboarding_state)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", table.self_onboarding_state));
    Err(SessionBuildError::CreatorNotOnboarded(state))
}
