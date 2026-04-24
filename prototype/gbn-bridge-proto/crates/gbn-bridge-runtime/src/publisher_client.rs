use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use gbn_bridge_protocol::{
    BootstrapJoinReply, BootstrapProgress, BridgeAck, BridgeCatalogRequest, BridgeCatalogResponse,
    BridgeClose, BridgeCommandAck, BridgeControlCommand, BridgeData, BridgeHeartbeat, BridgeLease,
    BridgeOpen, BridgeRegister, CreatorJoinRequest, PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{AuthorityResult, PublisherAuthority};

use crate::publisher_api_client::PublisherApiClient;
use crate::RuntimeResult;

#[derive(Debug, Clone)]
pub enum PublisherClient {
    Network(PublisherApiClient),
    Simulation(InProcessPublisherClient),
}

impl PublisherClient {
    pub fn from_network(client: PublisherApiClient) -> Self {
        Self::Network(client)
    }

    pub fn from_simulation(client: InProcessPublisherClient) -> Self {
        Self::Simulation(client)
    }

    pub fn publisher_public_key(&self) -> PublicKeyBytes {
        match self {
            Self::Network(client) => client.publisher_public_key().clone(),
            Self::Simulation(client) => client.publisher_public_key(),
        }
    }

    pub fn register_bridge(
        &mut self,
        request: BridgeRegister,
        reachability_class: ReachabilityClass,
        now_ms: u64,
    ) -> RuntimeResult<BridgeLease> {
        match self {
            Self::Network(client) => client.register_bridge(request, reachability_class, now_ms),
            Self::Simulation(client) => client
                .register_bridge(request, reachability_class, now_ms)
                .map_err(Into::into),
        }
    }

    pub fn renew_lease(&mut self, heartbeat: BridgeHeartbeat) -> RuntimeResult<BridgeLease> {
        match self {
            Self::Network(client) => client.renew_lease(heartbeat),
            Self::Simulation(client) => client.renew_lease(heartbeat).map_err(Into::into),
        }
    }

    pub fn issue_catalog(
        &mut self,
        chain_id: &str,
        request: &BridgeCatalogRequest,
        now_ms: u64,
    ) -> RuntimeResult<BridgeCatalogResponse> {
        match self {
            Self::Network(client) => client.issue_catalog(chain_id, request, now_ms),
            Self::Simulation(client) => client.issue_catalog(request, now_ms).map_err(Into::into),
        }
    }

    pub fn begin_bootstrap(
        &mut self,
        chain_id: &str,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> RuntimeResult<BootstrapJoinReply> {
        match self {
            Self::Network(client) => client.begin_bootstrap(chain_id, request, now_ms),
            Self::Simulation(client) => client.begin_bootstrap(request, now_ms).map_err(Into::into),
        }
    }

    pub fn report_progress(
        &mut self,
        chain_id: &str,
        progress: BootstrapProgress,
    ) -> RuntimeResult<()> {
        match self {
            Self::Network(client) => {
                let _ = client.report_progress(chain_id, progress)?;
                Ok(())
            }
            Self::Simulation(client) => {
                let _ = client.report_progress(chain_id, progress)?;
                Ok(())
            }
        }
    }

    pub fn simulation(&self) -> Option<&InProcessPublisherClient> {
        match self {
            Self::Simulation(client) => Some(client),
            Self::Network(_) => None,
        }
    }

    pub fn simulation_mut(&mut self) -> Option<&mut InProcessPublisherClient> {
        match self {
            Self::Simulation(client) => Some(client),
            Self::Network(_) => None,
        }
    }
}

impl From<InProcessPublisherClient> for PublisherClient {
    fn from(value: InProcessPublisherClient) -> Self {
        Self::Simulation(value)
    }
}

impl From<PublisherApiClient> for PublisherClient {
    fn from(value: PublisherApiClient) -> Self {
        Self::Network(value)
    }
}

#[derive(Debug, Clone)]
pub struct InProcessPublisherClient {
    authority: Rc<RefCell<PublisherAuthority>>,
    reported_progress: Vec<BootstrapProgress>,
    forwarded_frames: Vec<BridgeData>,
}

impl InProcessPublisherClient {
    pub fn new(authority: PublisherAuthority) -> Self {
        Self {
            authority: Rc::new(RefCell::new(authority)),
            reported_progress: Vec::new(),
            forwarded_frames: Vec::new(),
        }
    }

    pub fn publisher_public_key(&self) -> PublicKeyBytes {
        self.authority.borrow().publisher_public_key().clone()
    }

    pub fn authority(&self) -> Ref<'_, PublisherAuthority> {
        self.authority.borrow()
    }

    pub fn authority_mut(&self) -> RefMut<'_, PublisherAuthority> {
        self.authority.borrow_mut()
    }

    pub fn replace_authority(&mut self, authority: PublisherAuthority) {
        *self.authority.borrow_mut() = authority;
    }

    pub fn register_bridge(
        &mut self,
        request: BridgeRegister,
        reachability_class: ReachabilityClass,
        now_ms: u64,
    ) -> AuthorityResult<BridgeLease> {
        self.authority
            .borrow_mut()
            .register_bridge(request, reachability_class, now_ms)
    }

    pub fn renew_lease(&mut self, heartbeat: BridgeHeartbeat) -> AuthorityResult<BridgeLease> {
        self.authority.borrow_mut().handle_heartbeat(heartbeat)
    }

    pub fn reclassify_bridge(
        &mut self,
        bridge_id: &str,
        reachability_class: ReachabilityClass,
        udp_punch_port: Option<u16>,
        now_ms: u64,
    ) -> AuthorityResult<BridgeLease> {
        self.authority.borrow_mut().reclassify_bridge(
            bridge_id,
            reachability_class,
            udp_punch_port,
            now_ms,
        )
    }

    pub fn issue_catalog(
        &mut self,
        request: &BridgeCatalogRequest,
        now_ms: u64,
    ) -> AuthorityResult<BridgeCatalogResponse> {
        self.authority.borrow_mut().issue_catalog(request, now_ms)
    }

    pub fn begin_bootstrap(
        &mut self,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<BootstrapJoinReply> {
        self.authority
            .borrow_mut()
            .begin_bootstrap_reply(request, now_ms)
    }

    pub fn open_bridge_session(&mut self, open: BridgeOpen) -> AuthorityResult<()> {
        self.authority.borrow_mut().open_bridge_session(open)
    }

    pub fn ingest_bridge_frame(
        &mut self,
        via_bridge_id: &str,
        frame: BridgeData,
        received_at_ms: u64,
    ) -> AuthorityResult<BridgeAck> {
        self.authority
            .borrow_mut()
            .ingest_bridge_frame(via_bridge_id, frame, received_at_ms)
    }

    pub fn close_bridge_session(&mut self, close: BridgeClose) -> AuthorityResult<()> {
        self.authority.borrow_mut().close_bridge_session(close)
    }

    pub fn report_progress(
        &mut self,
        chain_id: &str,
        progress: BootstrapProgress,
    ) -> AuthorityResult<gbn_bridge_publisher::BootstrapProgressReceipt> {
        self.reported_progress.push(progress);
        let update = self
            .authority
            .borrow_mut()
            .report_bootstrap_progress_with_chain_id(
                chain_id,
                self.reported_progress
                    .last()
                    .cloned()
                    .expect("progress was just pushed"),
            )?;
        Ok(gbn_bridge_publisher::BootstrapProgressReceipt {
            bootstrap_session_id: self
                .reported_progress
                .last()
                .expect("progress was just pushed")
                .bootstrap_session_id
                .clone(),
            reporter_id: self
                .reported_progress
                .last()
                .expect("progress was just pushed")
                .reporter_id
                .clone(),
            stored_event_count: update.stored_event_count,
            latest_stage: "recorded".into(),
        })
    }

    pub fn reported_progress(&self) -> &[BootstrapProgress] {
        &self.reported_progress
    }

    pub fn forward_frame(&mut self, frame: BridgeData) {
        self.forwarded_frames.push(frame);
    }

    pub fn take_pending_control_commands(
        &mut self,
        bridge_id: &str,
        sent_at_ms: u64,
    ) -> AuthorityResult<Vec<BridgeControlCommand>> {
        let pending = self.authority.borrow().pending_bridge_commands(bridge_id);
        let mut authority = self.authority.borrow_mut();
        let mut commands = Vec::with_capacity(pending.len());
        for record in pending {
            authority.mark_bridge_command_dispatched(bridge_id, &record.command_id, sent_at_ms)?;
            commands.push(gbn_bridge_publisher::assignment::wire_command(
                &format!("simulation-{bridge_id}"),
                &record,
            ));
        }
        Ok(commands)
    }

    pub fn acknowledge_control_command(&mut self, ack: &BridgeCommandAck) -> AuthorityResult<()> {
        self.authority.borrow_mut().acknowledge_bridge_command(ack)
    }

    pub fn forwarded_frames(&self) -> &[BridgeData] {
        &self.forwarded_frames
    }
}
