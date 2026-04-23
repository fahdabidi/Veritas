use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeCatalogRequest, BridgeIngressEndpoint,
    BridgeRegister, CreatorJoinRequest, PendingCreator, PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::PublisherAuthority;
use gbn_bridge_runtime::{
    ChunkSender, ChunkSenderConfig, CreatorConfig, CreatorRuntime, ExitBridgeConfig,
    ExitBridgeRuntime, FramePayloadConfig, HostCreator, InProcessPublisherClient,
    UploadSessionConfig,
};

pub fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[91_u8; 32])
}

pub fn publisher() -> PublisherAuthority {
    PublisherAuthority::new(publisher_signing_key())
}

pub fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[seed; 32]))
}

pub fn default_capabilities() -> Vec<BridgeCapability> {
    vec![
        BridgeCapability::BootstrapSeed,
        BridgeCapability::CatalogRefresh,
        BridgeCapability::SessionRelay,
        BridgeCapability::BatchAssignment,
        BridgeCapability::ProgressReporting,
    ]
}

pub fn bridge_register(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    udp_punch_port: u16,
) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: host.into(),
            port: 443,
        }],
        requested_udp_punch_port: udp_punch_port,
        capabilities: default_capabilities(),
    }
}

pub fn bridge_config(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    udp_punch_port: u16,
) -> ExitBridgeConfig {
    ExitBridgeConfig {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoint: BridgeIngressEndpoint {
            host: host.into(),
            port: 443,
        },
        requested_udp_punch_port: udp_punch_port,
        capabilities: default_capabilities(),
    }
}

pub fn startup_bridge(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    shared_client: &InProcessPublisherClient,
    now_ms: u64,
) -> ExitBridgeRuntime {
    startup_bridge_with_class(
        bridge_id,
        key_seed,
        host,
        shared_client,
        ReachabilityClass::Direct,
        now_ms,
    )
}

pub fn startup_bridge_with_class(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    shared_client: &InProcessPublisherClient,
    reachability_class: ReachabilityClass,
    now_ms: u64,
) -> ExitBridgeRuntime {
    let mut runtime = ExitBridgeRuntime::new(
        bridge_config(bridge_id, key_seed, host, 443),
        shared_client.clone(),
    );
    runtime.startup(reachability_class, now_ms).unwrap();
    runtime
}

pub fn creator_runtime(creator_id: &str, key_seed: u8, host: &str) -> CreatorRuntime {
    CreatorRuntime::new(CreatorConfig {
        creator_id: creator_id.into(),
        ip_addr: host.into(),
        pub_key: node_public_key(key_seed),
        udp_punch_port: 443,
    })
}

pub fn prime_creator(
    creator: &mut CreatorRuntime,
    bridge_for_catalog: &mut ExitBridgeRuntime,
    now_ms: u64,
) {
    creator
        .load_publisher_trust_root(bridge_for_catalog.publisher_client().publisher_public_key())
        .unwrap();
    let catalog = bridge_for_catalog
        .publisher_client_mut()
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: creator.config().creator_id.clone(),
                known_catalog_id: None,
                direct_only: false,
                refresh_hint: None,
            },
            now_ms,
        )
        .unwrap();
    creator.ingest_catalog(catalog.clone(), now_ms).unwrap();
    for bridge in &catalog.bridges {
        if matches!(bridge.reachability_class, ReachabilityClass::Direct) {
            creator.mark_bridge_active(&bridge.bridge_id, now_ms);
        }
    }
}

pub fn creator_join_request(
    request_id: &str,
    relay_bridge_id: &str,
    key_seed: u8,
) -> CreatorJoinRequest {
    CreatorJoinRequest {
        request_id: request_id.into(),
        host_creator_id: "host-creator-01".into(),
        relay_bridge_id: relay_bridge_id.into(),
        creator: PendingCreator {
            node_id: format!("creator-{request_id}"),
            ip_addr: "203.0.113.44".into(),
            pub_key: node_public_key(key_seed),
            udp_punch_port: 443,
        },
    }
}

pub fn host_creator() -> HostCreator {
    HostCreator::new("host-creator-01")
}

pub fn chunk_sender(frame_size_bytes: usize) -> ChunkSender {
    ChunkSender::with_config(ChunkSenderConfig {
        upload_session: UploadSessionConfig {
            frame_payload: FramePayloadConfig { frame_size_bytes },
        },
    })
}
