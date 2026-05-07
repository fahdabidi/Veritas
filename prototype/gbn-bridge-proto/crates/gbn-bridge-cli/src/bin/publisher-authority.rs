use std::env;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gbn_bridge_protocol::{publisher_identity, DEFAULT_UDP_PUNCH_PORT};
use gbn_bridge_publisher::{
    admin::{AdminCreatorConfig, AdminHttpServer, AdminState, DEFAULT_ADMIN_BIND_ADDR},
    metrics_emitter::{spawn_cloudwatch_emitter, MetricsEmitterConfig},
    AuthorityConfig, AuthorityPolicy, AuthorityServer, PostgresStorageConfig, PublisherAuthority,
    PublisherServiceConfig, PublisherSigningSource,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("publisher-authority startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = PublisherServiceConfig::from_env()?;
    let request_max_bytes = config.request_max_bytes;
    let authority_url = local_http_url_from_bind_addr(&config.bind_addr, 8080);
    let signing_key = PublisherSigningSource::from_env()
        .and_then(|source| source.load_signing_key())
        .map_err(|error| error.to_string())?;
    let admin_creator = AdminCreatorConfig {
        actor_id: env::var("GBN_BRIDGE_ADMIN_CREATOR_ID")
            .unwrap_or_else(|_| "publisher-authority".to_string()),
        signing_key: signing_key.clone(),
        publisher_pub: publisher_identity(&signing_key),
        authority_url,
        creator_ip_addr: "127.0.0.1".to_string(),
        udp_punch_port: DEFAULT_UDP_PUNCH_PORT,
        timeout: Duration::from_secs(5),
    };
    let authority = match PostgresStorageConfig::from_env().map_err(|error| error.to_string())? {
        Some(postgres_config) => PublisherAuthority::with_postgres(
            signing_key,
            AuthorityConfig::default(),
            AuthorityPolicy::default(),
            postgres_config,
            now_ms(),
        )
        .map_err(|error| error.to_string())?,
        None => PublisherAuthority::new(signing_key),
    };
    let server = AuthorityServer::new(authority, config);
    let service_handle = server.service_handle();
    let bound = server.bind().map_err(|error| error.to_string())?;
    let admin_addr: SocketAddr = DEFAULT_ADMIN_BIND_ADDR
        .parse()
        .expect("default admin bind address should be valid");
    let admin_server = AdminHttpServer::bind(
        admin_addr,
        AdminState::authority_with_creator(service_handle.clone(), admin_creator),
        request_max_bytes,
    )
    .map_err(|error| error.to_string())?;
    let _admin_handle = admin_server.spawn().map_err(|error| error.to_string())?;
    let metrics_service = service_handle.clone();
    let _metrics_handle = spawn_cloudwatch_emitter(
        MetricsEmitterConfig::from_env("authority"),
        move |service, stack| {
            metrics_service
                .lock()
                .expect("authority service mutex poisoned while emitting metrics")
                .publisher_authority()
                .metrics_snapshot()
                .cloudwatch_data(service, stack)
        },
    );
    println!(
        "publisher-authority service listening on {}; admin listening on {}",
        bound.local_addr(),
        DEFAULT_ADMIN_BIND_ADDR
    );
    bound.serve_forever().map_err(|error| error.to_string())
}

fn local_http_url_from_bind_addr(bind_addr: &str, default_port: u16) -> String {
    let port = bind_addr
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .unwrap_or(default_port);
    format!("http://127.0.0.1:{port}")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}
