use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use gbn_bridge_publisher::{
    admin::{AdminHttpServer, AdminState, DEFAULT_ADMIN_BIND_ADDR},
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
    let signing_key = PublisherSigningSource::from_env()
        .and_then(|source| source.load_signing_key())
        .map_err(|error| error.to_string())?;
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
        AdminState::authority(service_handle),
        request_max_bytes,
    )
    .map_err(|error| error.to_string())?;
    let _admin_handle = admin_server.spawn().map_err(|error| error.to_string())?;
    println!(
        "publisher-authority service listening on {}; admin listening on {}",
        bound.local_addr(),
        DEFAULT_ADMIN_BIND_ADDR
    );
    bound.serve_forever().map_err(|error| error.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}
