use std::time::{SystemTime, UNIX_EPOCH};

use gbn_bridge_publisher::{
    AuthorityConfig, AuthorityPolicy, AuthorityServer, PostgresStorageConfig, PublisherAuthority,
    PublisherServiceConfig, PublisherSigningSource,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("bridge-publisher startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = PublisherServiceConfig::from_env()?;
    let authority_config = AuthorityConfig::from_env()?;
    let signing_key = PublisherSigningSource::from_env()
        .and_then(|source| source.load_signing_key())
        .map_err(|error| error.to_string())?;
    let authority = match PostgresStorageConfig::from_env().map_err(|error| error.to_string())? {
        Some(postgres_config) => PublisherAuthority::with_postgres(
            signing_key,
            authority_config,
            AuthorityPolicy::default(),
            postgres_config,
            now_ms(),
        )
        .map_err(|error| error.to_string())?,
        None => PublisherAuthority::with_config(
            signing_key,
            authority_config,
            AuthorityPolicy::default(),
        ),
    };
    let server = AuthorityServer::new(authority, config);
    let bound = server.bind().map_err(|error| error.to_string())?;
    println!(
        "bridge-publisher authority API listening on {}",
        bound.local_addr()
    );
    bound.serve_forever().map_err(|error| error.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}
