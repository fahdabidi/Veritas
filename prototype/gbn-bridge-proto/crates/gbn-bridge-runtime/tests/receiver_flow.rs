use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeAckStatus, BridgeCapability, BridgeCatalogRequest,
    BridgeIngressEndpoint, PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{AuthorityServer, PublisherAuthority, PublisherServiceConfig};
use gbn_bridge_runtime::{
    AckTracker, BridgePool, ChunkSender, ChunkSenderConfig, CreatorConfig, CreatorRuntime,
    ExitBridgeConfig, ExitBridgeRuntime, FanoutScheduler, FanoutSchedulerConfig,
    FramePayloadConfig, HttpJsonTransport, HttpTransportConfig, PublisherApiClient,
    UploadSessionConfig,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[91_u8; 32])
}

fn actor_signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&actor_signing_key(seed))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn bridge_config(bridge_id: &str, key_seed: u8, host: &str) -> ExitBridgeConfig {
    ExitBridgeConfig {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoint: BridgeIngressEndpoint {
            host: host.into(),
            port: 443,
        },
        requested_udp_punch_port: 443,
        capabilities: vec![
            BridgeCapability::BootstrapSeed,
            BridgeCapability::CatalogRefresh,
            BridgeCapability::SessionRelay,
            BridgeCapability::BatchAssignment,
            BridgeCapability::ProgressReporting,
        ],
    }
}

fn transport(handle: &gbn_bridge_publisher::AuthorityServerHandle) -> HttpJsonTransport {
    HttpJsonTransport::new(HttpTransportConfig::new(format!(
        "http://{}",
        handle.local_addr()
    )))
    .unwrap()
}

fn creator_runtime(creator_id: &str, key_seed: u8, host: &str) -> CreatorRuntime {
    CreatorRuntime::new(CreatorConfig {
        creator_id: creator_id.into(),
        ip_addr: host.into(),
        pub_key: node_public_key(key_seed),
        udp_punch_port: 443,
    })
}

#[test]
fn upload_frames_flow_over_receiver_routes_and_persist_chain_id() {
    let authority = PublisherAuthority::new(publisher_signing_key());
    let publisher_pub = authority.publisher_public_key().clone();
    let server = AuthorityServer::new(
        authority,
        PublisherServiceConfig {
            bind_addr: "127.0.0.1:0".into(),
            ..PublisherServiceConfig::default()
        },
    );
    let service: Arc<Mutex<_>> = server.service_handle();
    let handle = server.bind().unwrap().spawn().unwrap();

    let mut bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-receiver-01", 101, "198.51.100.101"),
        PublisherApiClient::new(
            "bridge-receiver-01",
            actor_signing_key(101),
            publisher_pub.clone(),
            transport(&handle),
        ),
    );
    let bridge_startup_now_ms = now_ms();
    bridge
        .startup(ReachabilityClass::Direct, bridge_startup_now_ms)
        .unwrap();

    let mut creator = creator_runtime("creator-receiver-01", 111, "203.0.113.101");
    creator
        .load_publisher_trust_root(publisher_pub.clone())
        .unwrap();
    let mut catalog_client = PublisherApiClient::new(
        "creator-receiver-01",
        actor_signing_key(111),
        publisher_pub,
        transport(&handle),
    );
    let catalog_now_ms = now_ms();
    let catalog = catalog_client
        .issue_catalog(
            "catalog-chain-receiver-01",
            &BridgeCatalogRequest {
                creator_id: creator.config().creator_id.clone(),
                known_catalog_id: None,
                direct_only: true,
                refresh_hint: None,
            },
            catalog_now_ms,
        )
        .unwrap();
    creator.ingest_catalog(catalog, catalog_now_ms).unwrap();
    creator.mark_bridge_active("bridge-receiver-01", catalog_now_ms);

    let pool = BridgePool::from_creator(&creator, 10).unwrap();
    assert_eq!(
        pool.selected_bridge_ids(),
        vec!["bridge-receiver-01".to_string()]
    );

    let mut sender = ChunkSender::with_config(ChunkSenderConfig {
        upload_session: UploadSessionConfig {
            frame_payload: FramePayloadConfig {
                frame_size_bytes: 6,
            },
        },
    });
    let payload = b"phase-six-network-receiver-path".to_vec();
    let session = sender.begin_session(&creator, &payload, now_ms()).unwrap();
    let mut tracker = AckTracker::new(&session);
    let scheduler = FanoutScheduler::new(
        &pool,
        FanoutSchedulerConfig {
            target_bridge_count: 10,
            reuse_timeout_ms: 25,
        },
        now_ms(),
    );

    let open_now_ms = now_ms();
    sender
        .open_selected_bridges(
            &session,
            &pool.selected_bridge_ids(),
            std::slice::from_mut(&mut bridge),
            open_now_ms,
        )
        .unwrap();

    let plan = scheduler.initial_plan(session.frames()).unwrap();
    let dispatch_now_ms = now_ms();
    let mut acks = sender
        .send_dispatches(
            &plan.initial,
            std::slice::from_mut(&mut bridge),
            &mut tracker,
            dispatch_now_ms,
        )
        .unwrap();
    let reuse_now_ms = dispatch_now_ms + 30;
    let reused = scheduler
        .reuse_pending(&plan.pending, reuse_now_ms)
        .unwrap();
    acks.extend(
        sender
            .send_dispatches(
                &reused,
                std::slice::from_mut(&mut bridge),
                &mut tracker,
                reuse_now_ms,
            )
            .unwrap(),
    );
    sender
        .close_selected_bridges(
            &session,
            &pool.selected_bridge_ids(),
            std::slice::from_mut(&mut bridge),
            reuse_now_ms + 20,
        )
        .unwrap();

    assert!(tracker.all_acked());
    assert!(acks
        .iter()
        .any(|ack| ack.status == BridgeAckStatus::Complete));

    let stored_session = {
        let service = service.lock().unwrap();
        service
            .publisher_authority()
            .upload_session(session.session_id())
            .cloned()
            .unwrap()
    };
    assert_eq!(stored_session.chain_id.as_deref(), Some(session.chain_id()));
    assert_eq!(
        stored_session.frames_by_sequence.len(),
        session.frame_count()
    );
    assert!(stored_session
        .frames_by_sequence
        .values()
        .all(|record| record.chain_id.as_deref() == Some(session.chain_id())));
    assert_eq!(bridge.local_forwarded_frames().len(), session.frame_count());

    handle.join().unwrap();
}
