//! Creator-local discovery table ownership and persistence.
//!
//! The local DHT has one writer: the background thread spawned by [`LocalDhtStore`].
//! Admin handlers and bootstrap workers never mutate the table directly. They submit
//! [`LocalDhtCommand`] values over an `mpsc` channel, and all read paths clone an
//! `Arc<RwLock<LocalDiscoveryTable>>` snapshot. This keeps route reads from holding a
//! mutable lock across network I/O while still preserving a single ordered stream of
//! persisted state transitions.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, RwLock};
use std::thread;

use gbn_bridge_protocol::{
    BootstrapSession, BridgeDhtEntry, CreatorDhtEntry, HostRoleState, LocalDiscoveryTable,
    PublicKeyBytes, PublisherDhtEntry, SelfOnboardingState, TunnelState,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LocalDhtError {
    #[error("local DHT I/O error: {0}")]
    Io(String),

    #[error("local DHT serialization error: {0}")]
    Serialization(String),

    #[error("local DHT writer is not running")]
    WriterClosed,

    #[error("local DHT writer rejected response: {0}")]
    ResponseRejected(String),
}

impl From<std::io::Error> for LocalDhtError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for LocalDhtError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetCreatorStateResponse {
    pub actor_id: String,
    pub chain_id: String,
    pub prior_self_onboarding_state: SelfOnboardingState,
    pub prior_host_role_state: HostRoleState,
    pub prior_bootstrap_session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum LocalDhtMutation {
    SetSelfOnboardingState(SelfOnboardingState),
    SetHostRoleState(HostRoleState),
    SetPublisherEntry(Option<PublisherDhtEntry>),
    SetHostCreatorEntry(Option<CreatorDhtEntry>),
    SetCreatorEntry(Option<CreatorDhtEntry>),
    UpsertBridgeEntry(BridgeDhtEntry),
    SetActiveTunnels(Vec<TunnelState>),
    SetBootstrapSession(Option<BootstrapSession>),
    SetLastError(Option<String>),
}

#[derive(Debug)]
pub enum LocalDhtCommand {
    Mutate {
        mutation: LocalDhtMutation,
        now_ms: u64,
        reply: Sender<Result<LocalDiscoveryTable, LocalDhtError>>,
    },
    Replace {
        table: LocalDiscoveryTable,
        reply: Sender<Result<LocalDiscoveryTable, LocalDhtError>>,
    },
    Reset {
        chain_id: String,
        now_ms: u64,
        reply: Sender<Result<ResetCreatorStateResponse, LocalDhtError>>,
    },
}

#[derive(Debug, Clone)]
pub struct LocalDhtStore {
    actor_id: String,
    state_path: PathBuf,
    snapshot: Arc<RwLock<LocalDiscoveryTable>>,
    tx: Sender<LocalDhtCommand>,
}

impl LocalDhtStore {
    pub fn load_or_create(
        actor_id: impl Into<String>,
        state_path: impl Into<PathBuf>,
        trusted_publisher_key: Option<&PublicKeyBytes>,
        now_ms: u64,
    ) -> Result<Self, LocalDhtError> {
        let actor_id = actor_id.into();
        let state_path = state_path.into();
        let mut table = load_table(&state_path, &actor_id, now_ms)?;
        if let Some(trust_root) = trusted_publisher_key {
            let dropped = table.validate_and_prune(trust_root, now_ms);
            if dropped > 0 {
                table.last_error = Some(format!("dropped {dropped} invalid persisted DHT entries"));
                table.last_update_ms = now_ms;
            }
        }
        persist_table(&state_path, &table)?;
        Ok(Self::start(actor_id, state_path, table))
    }

    pub fn start(
        actor_id: impl Into<String>,
        state_path: impl Into<PathBuf>,
        table: LocalDiscoveryTable,
    ) -> Self {
        let actor_id = actor_id.into();
        let state_path = state_path.into();
        let snapshot = Arc::new(RwLock::new(table.clone()));
        let (tx, rx) = mpsc::channel();
        let writer_path = state_path.clone();
        let writer_snapshot = Arc::clone(&snapshot);
        thread::spawn(move || {
            let mut table = table;
            while let Ok(command) = rx.recv() {
                match command {
                    LocalDhtCommand::Mutate {
                        mutation,
                        now_ms,
                        reply,
                    } => {
                        let mut next = table.clone();
                        apply_mutation(&mut next, mutation, now_ms);
                        let result = commit_table(&writer_path, &writer_snapshot, &mut table, next);
                        let _ = reply.send(result);
                    }
                    LocalDhtCommand::Replace { table: next, reply } => {
                        let result = commit_table(&writer_path, &writer_snapshot, &mut table, next);
                        let _ = reply.send(result);
                    }
                    LocalDhtCommand::Reset {
                        chain_id,
                        now_ms,
                        reply,
                    } => {
                        let prior = table.clone();
                        let mut next = table.clone();
                        next.reset(now_ms);
                        let result = commit_table(&writer_path, &writer_snapshot, &mut table, next)
                            .map(|_| ResetCreatorStateResponse {
                                actor_id: prior.actor_id,
                                chain_id,
                                prior_self_onboarding_state: prior.self_onboarding_state,
                                prior_host_role_state: prior.host_role_state,
                                prior_bootstrap_session_id: prior
                                    .current_bootstrap_session
                                    .map(|session| session.session_id),
                            });
                        let _ = reply.send(result);
                    }
                }
            }
        });

        Self {
            actor_id,
            state_path,
            snapshot,
            tx,
        }
    }

    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    pub fn snapshot(&self) -> LocalDiscoveryTable {
        self.snapshot
            .read()
            .expect("local DHT snapshot lock poisoned")
            .clone()
    }

    pub fn mutate(
        &self,
        mutation: LocalDhtMutation,
        now_ms: u64,
    ) -> Result<LocalDiscoveryTable, LocalDhtError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(LocalDhtCommand::Mutate {
                mutation,
                now_ms,
                reply,
            })
            .map_err(|_| LocalDhtError::WriterClosed)?;
        rx.recv().map_err(|_| LocalDhtError::WriterClosed)?
    }

    pub fn replace(
        &self,
        table: LocalDiscoveryTable,
    ) -> Result<LocalDiscoveryTable, LocalDhtError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(LocalDhtCommand::Replace { table, reply })
            .map_err(|_| LocalDhtError::WriterClosed)?;
        rx.recv().map_err(|_| LocalDhtError::WriterClosed)?
    }

    pub fn reset(
        &self,
        chain_id: impl Into<String>,
        now_ms: u64,
    ) -> Result<ResetCreatorStateResponse, LocalDhtError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(LocalDhtCommand::Reset {
                chain_id: chain_id.into(),
                now_ms,
                reply,
            })
            .map_err(|_| LocalDhtError::WriterClosed)?;
        rx.recv().map_err(|_| LocalDhtError::WriterClosed)?
    }
}

fn load_table(
    path: &Path,
    actor_id: &str,
    now_ms: u64,
) -> Result<LocalDiscoveryTable, LocalDhtError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LocalDiscoveryTable::empty(actor_id.to_string(), now_ms))
        }
        Err(error) => return Err(error.into()),
    };

    match serde_json::from_str::<LocalDiscoveryTable>(&raw) {
        Ok(table) if table.actor_id == actor_id => Ok(table),
        Ok(table) => {
            eprintln!(
                "creator local DHT actor mismatch at {}: persisted actor_id={} runtime actor_id={}; starting empty",
                path.display(),
                table.actor_id,
                actor_id
            );
            Ok(LocalDiscoveryTable::empty(actor_id.to_string(), now_ms))
        }
        Err(error) => {
            eprintln!(
                "creator local DHT at {} is not parseable: {error}; starting empty",
                path.display()
            );
            Ok(LocalDiscoveryTable::empty(actor_id.to_string(), now_ms))
        }
    }
}

fn apply_mutation(table: &mut LocalDiscoveryTable, mutation: LocalDhtMutation, now_ms: u64) {
    match mutation {
        LocalDhtMutation::SetSelfOnboardingState(state) => {
            table.self_onboarding_state = state;
        }
        LocalDhtMutation::SetHostRoleState(state) => {
            table.host_role_state = state;
        }
        LocalDhtMutation::SetPublisherEntry(entry) => {
            table.publisher_entry = entry;
        }
        LocalDhtMutation::SetHostCreatorEntry(entry) => {
            table.host_creator_entry = entry;
        }
        LocalDhtMutation::SetCreatorEntry(entry) => {
            table.creator_entry = entry;
        }
        LocalDhtMutation::UpsertBridgeEntry(entry) => {
            match table
                .bridge_entries
                .iter_mut()
                .find(|existing| existing.bridge_id == entry.bridge_id)
            {
                Some(existing) => *existing = entry,
                None => table.bridge_entries.push(entry),
            }
        }
        LocalDhtMutation::SetActiveTunnels(tunnels) => {
            table.active_tunnels = tunnels;
        }
        LocalDhtMutation::SetBootstrapSession(session) => {
            table.current_bootstrap_session = session;
        }
        LocalDhtMutation::SetLastError(error) => {
            table.last_error = error;
        }
    }
    table.last_update_ms = now_ms;
}

fn commit_table(
    path: &Path,
    snapshot: &Arc<RwLock<LocalDiscoveryTable>>,
    current: &mut LocalDiscoveryTable,
    next: LocalDiscoveryTable,
) -> Result<LocalDiscoveryTable, LocalDhtError> {
    persist_table(path, &next)?;
    *current = next.clone();
    *snapshot
        .write()
        .expect("local DHT snapshot lock poisoned while committing") = next.clone();
    Ok(next)
}

pub fn persist_table(path: &Path, table: &LocalDiscoveryTable) -> Result<(), LocalDhtError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(table)?;
    {
        let mut file = File::create(&tmp)?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }

    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }

    fs::rename(&tmp, path)?;
    Ok(())
}
