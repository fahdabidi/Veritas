use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeIngressEndpoint, PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{
    AuthorityServer, AuthorityServerHandle, AuthorityService, BootstrapSessionRecord,
    BridgeCommandRecord, CatalogIssuanceRecord, PostgresStorageConfig, PublisherAuthority,
    PublisherServiceConfig, ReceiverProxyConfig, ReceiverProxyHandle, ReceiverProxyServer,
    UploadSessionRecord,
};
use gbn_bridge_runtime::{
    default_chain_id, default_request_id, BridgeControlClient, CreatorConfig, CreatorRuntime,
    ExitBridgeConfig, ExitBridgeRuntime, ForwarderClient, HostCreator, HostCreatorClient,
    HttpJsonTransport, HttpTransportConfig, PublisherApiClient,
};
use postgres::{Client, NoTls};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[33_u8; 32])
}

fn actor_signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

pub fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&actor_signing_key(seed))
}

fn default_capabilities() -> Vec<BridgeCapability> {
    vec![
        BridgeCapability::BootstrapSeed,
        BridgeCapability::CatalogRefresh,
        BridgeCapability::SessionRelay,
        BridgeCapability::BatchAssignment,
        BridgeCapability::ProgressReporting,
    ]
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}

fn postgres_url() -> String {
    std::env::var("GBN_BRIDGE_TEST_POSTGRES_URL").unwrap_or_else(|_| {
        "host=127.0.0.1 port=5432 user=postgres password=postgres dbname=veritas_proto006".into()
    })
}

fn unique_schema(prefix: &str) -> String {
    format!("{prefix}_{}", now_ms())
}

fn cleanup_schema(schema: &str) {
    let mut client = Client::connect(&postgres_url(), NoTls).unwrap();
    client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE;"))
        .unwrap();
}

pub struct DistributedHarness {
    publisher_signing_key: SigningKey,
    publisher_pub: PublicKeyBytes,
    service: Arc<Mutex<AuthorityService>>,
    authority_handle: Option<AuthorityServerHandle>,
    receiver_handle: Option<ReceiverProxyHandle>,
    authority_url: String,
    receiver_url: String,
    control_url: String,
    postgres_config: Option<PostgresStorageConfig>,
}

impl DistributedHarness {
    pub fn in_memory() -> Self {
        Self::start(None, now_ms())
    }

    pub fn durable(schema_prefix: &str) -> Self {
        let schema = unique_schema(schema_prefix);
        Self::start(
            Some(PostgresStorageConfig {
                connection_string: postgres_url(),
                schema,
            }),
            now_ms(),
        )
    }

    fn start(postgres_config: Option<PostgresStorageConfig>, now_ms: u64) -> Self {
        let publisher_signing_key = publisher_signing_key();
        let authority = match &postgres_config {
            Some(config) => PublisherAuthority::with_postgres(
                publisher_signing_key.clone(),
                gbn_bridge_publisher::AuthorityConfig::default(),
                gbn_bridge_publisher::AuthorityPolicy::default(),
                config.clone(),
                now_ms,
            )
            .unwrap(),
            None => PublisherAuthority::new(publisher_signing_key.clone()),
        };
        let publisher_pub = authority.publisher_public_key().clone();
        let server = AuthorityServer::new(
            authority,
            PublisherServiceConfig {
                bind_addr: "127.0.0.1:0".into(),
                ..PublisherServiceConfig::default()
            },
        );
        let service = server.service_handle();
        let authority_handle = server.bind().unwrap().spawn().unwrap();
        let authority_url = format!("http://{}", authority_handle.local_addr());

        let receiver_handle = ReceiverProxyServer::bind(ReceiverProxyConfig {
            bind_addr: "127.0.0.1:0".into(),
            authority_url: authority_url.clone(),
            ..ReceiverProxyConfig::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let receiver_url = format!("http://{}", receiver_handle.local_addr());
        let control_url = format!("ws://{}/v1/bridge/control", authority_handle.local_addr());

        Self {
            publisher_signing_key,
            publisher_pub,
            service,
            authority_handle: Some(authority_handle),
            receiver_handle: Some(receiver_handle),
            authority_url,
            receiver_url,
            control_url,
            postgres_config,
        }
    }

    pub fn service_handle(&self) -> Arc<Mutex<AuthorityService>> {
        Arc::clone(&self.service)
    }

    pub fn publisher_public_key(&self) -> PublicKeyBytes {
        self.publisher_pub.clone()
    }

    pub fn authority_transport(&self) -> HttpJsonTransport {
        HttpJsonTransport::new(HttpTransportConfig::new(self.authority_url.clone())).unwrap()
    }

    pub fn receiver_transport(&self) -> HttpJsonTransport {
        HttpJsonTransport::new(HttpTransportConfig::new(self.receiver_url.clone())).unwrap()
    }

    pub fn publisher_client(&self, actor_id: &str, key_seed: u8) -> PublisherApiClient {
        PublisherApiClient::new(
            actor_id.to_string(),
            actor_signing_key(key_seed),
            self.publisher_public_key(),
            self.authority_transport(),
        )
    }

    pub fn creator(&self, creator_id: &str, key_seed: u8, host: &str) -> CreatorRuntime {
        let mut creator = CreatorRuntime::new(CreatorConfig {
            creator_id: creator_id.into(),
            ip_addr: host.into(),
            pub_key: node_public_key(key_seed),
            udp_punch_port: 443,
        });
        creator.attach_publisher_client(self.publisher_client(creator_id, key_seed));
        creator
            .load_publisher_trust_root(self.publisher_public_key())
            .unwrap();
        creator
    }

    pub fn host_creator(&self, host_creator_id: &str, key_seed: u8) -> HostCreator {
        let mut host_creator = HostCreator::new(host_creator_id);
        host_creator
            .attach_client(
                HostCreatorClient::new(
                    host_creator_id,
                    self.publisher_client(host_creator_id, key_seed),
                )
                .unwrap(),
            )
            .unwrap();
        host_creator
    }

    pub fn start_bridge(
        &self,
        bridge_id: &str,
        key_seed: u8,
        host: &str,
        reachability_class: ReachabilityClass,
        startup_now_ms: u64,
    ) -> ExitBridgeRuntime {
        let signing_key = actor_signing_key(key_seed);
        let publisher_client = PublisherApiClient::new(
            bridge_id.to_string(),
            signing_key.clone(),
            self.publisher_public_key(),
            self.authority_transport(),
        );
        let mut bridge = ExitBridgeRuntime::new(
            ExitBridgeConfig {
                bridge_id: bridge_id.into(),
                identity_pub: PublicKeyBytes::from_verifying_key(&signing_key.verifying_key()),
                ingress_endpoint: BridgeIngressEndpoint {
                    host: host.into(),
                    port: 443,
                },
                requested_udp_punch_port: 443,
                capabilities: default_capabilities(),
            },
            publisher_client,
        );
        bridge.attach_forwarder_client(ForwarderClient::new(
            bridge_id.to_string(),
            signing_key.clone(),
            self.publisher_public_key(),
            self.receiver_transport(),
        ));
        bridge.startup(reachability_class, startup_now_ms).unwrap();
        let lease_id = bridge.current_lease().unwrap().lease_id.clone();
        let control_client = BridgeControlClient::connect(
            &self.control_url,
            bridge_id,
            &lease_id,
            &PublicKeyBytes::from_verifying_key(&signing_key.verifying_key()),
            &signing_key,
            &self.publisher_public_key(),
            &default_chain_id("bridge-control-connect", bridge_id, &lease_id),
            &default_request_id("control-hello", bridge_id, startup_now_ms),
            startup_now_ms,
            None,
            30_000,
        )
        .unwrap();
        bridge.attach_control_client(control_client);
        bridge
    }

    pub fn reconnect_bridge(
        &self,
        bridge_id: &str,
        key_seed: u8,
        host: &str,
        reconnect_now_ms: u64,
    ) -> ExitBridgeRuntime {
        let signing_key = actor_signing_key(key_seed);
        let publisher_client = PublisherApiClient::new(
            bridge_id.to_string(),
            signing_key.clone(),
            self.publisher_public_key(),
            self.authority_transport(),
        );
        let mut bridge = ExitBridgeRuntime::new(
            ExitBridgeConfig {
                bridge_id: bridge_id.into(),
                identity_pub: PublicKeyBytes::from_verifying_key(&signing_key.verifying_key()),
                ingress_endpoint: BridgeIngressEndpoint {
                    host: host.into(),
                    port: 443,
                },
                requested_udp_punch_port: 443,
                capabilities: default_capabilities(),
            },
            publisher_client,
        );
        bridge.attach_forwarder_client(ForwarderClient::new(
            bridge_id.to_string(),
            signing_key.clone(),
            self.publisher_public_key(),
            self.receiver_transport(),
        ));
        let lease = self
            .service
            .lock()
            .unwrap()
            .publisher_authority()
            .bridge_record(bridge_id)
            .expect("bridge record should exist for reconnect")
            .current_lease
            .clone();
        bridge.apply_remote_lease(lease.clone(), reconnect_now_ms);
        let control_client = BridgeControlClient::connect(
            &self.control_url,
            bridge_id,
            &lease.lease_id,
            &PublicKeyBytes::from_verifying_key(&signing_key.verifying_key()),
            &signing_key,
            &self.publisher_public_key(),
            &default_chain_id("bridge-control-reconnect", bridge_id, &lease.lease_id),
            &default_request_id("control-hello-reconnect", bridge_id, reconnect_now_ms),
            reconnect_now_ms,
            None,
            30_000,
        )
        .unwrap();
        bridge.attach_control_client(control_client);
        bridge
    }

    pub fn issue_catalog_record(&self, catalog_id: &str) -> CatalogIssuanceRecord {
        self.service
            .lock()
            .unwrap()
            .publisher_authority()
            .catalog_issuance(catalog_id)
            .cloned()
            .unwrap()
    }

    pub fn bootstrap_session_record(&self, bootstrap_session_id: &str) -> BootstrapSessionRecord {
        self.service
            .lock()
            .unwrap()
            .publisher_authority()
            .bootstrap_session(bootstrap_session_id)
            .cloned()
            .unwrap()
    }

    pub fn pending_commands(&self, bridge_id: &str) -> Vec<BridgeCommandRecord> {
        self.service
            .lock()
            .unwrap()
            .publisher_authority()
            .pending_bridge_commands(bridge_id)
    }

    pub fn upload_session_record(&self, session_id: &str) -> UploadSessionRecord {
        self.service
            .lock()
            .unwrap()
            .publisher_authority()
            .upload_session(session_id)
            .cloned()
            .unwrap()
    }

    pub fn process_bootstrap_timeouts(&self, timeout_now_ms: u64) -> Vec<String> {
        self.service
            .lock()
            .unwrap()
            .publisher_authority_mut()
            .process_bootstrap_timeouts(timeout_now_ms)
            .unwrap()
    }

    pub fn restart_authority_and_receiver(&mut self, now_ms: u64) {
        let postgres_config = self
            .postgres_config
            .clone()
            .expect("durable harness required for restart");
        self.shutdown_servers();

        let authority = PublisherAuthority::with_postgres(
            self.publisher_signing_key.clone(),
            gbn_bridge_publisher::AuthorityConfig::default(),
            gbn_bridge_publisher::AuthorityPolicy::default(),
            postgres_config,
            now_ms,
        )
        .unwrap();
        let server = AuthorityServer::new(
            authority,
            PublisherServiceConfig {
                bind_addr: "127.0.0.1:0".into(),
                ..PublisherServiceConfig::default()
            },
        );
        self.service = server.service_handle();
        let authority_handle = server.bind().unwrap().spawn().unwrap();
        self.authority_url = format!("http://{}", authority_handle.local_addr());
        self.control_url = format!("ws://{}/v1/bridge/control", authority_handle.local_addr());
        self.authority_handle = Some(authority_handle);

        let receiver_handle = ReceiverProxyServer::bind(ReceiverProxyConfig {
            bind_addr: "127.0.0.1:0".into(),
            authority_url: self.authority_url.clone(),
            ..ReceiverProxyConfig::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        self.receiver_url = format!("http://{}", receiver_handle.local_addr());
        self.receiver_handle = Some(receiver_handle);
    }

    fn shutdown_servers(&mut self) {
        if let Some(handle) = self.receiver_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.authority_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DistributedHarness {
    fn drop(&mut self) {
        self.shutdown_servers();
        if let Some(config) = &self.postgres_config {
            cleanup_schema(&config.schema);
        }
    }
}
