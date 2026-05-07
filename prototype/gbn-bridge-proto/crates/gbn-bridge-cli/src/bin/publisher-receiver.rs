use std::net::SocketAddr;

use gbn_bridge_publisher::{
    admin::{AdminHttpServer, AdminState, DEFAULT_ADMIN_BIND_ADDR},
    ReceiverProxyConfig, ReceiverProxyServer,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("publisher-receiver startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = ReceiverProxyConfig::from_env()?;
    let request_max_bytes = config.request_max_bytes;
    let server = ReceiverProxyServer::bind(config.clone()).map_err(|error| error.to_string())?;
    let admin_addr: SocketAddr = DEFAULT_ADMIN_BIND_ADDR
        .parse()
        .expect("default admin bind address should be valid");
    let admin_server = AdminHttpServer::bind(admin_addr, AdminState::stub(), request_max_bytes)
        .map_err(|error| error.to_string())?;
    let _admin_handle = admin_server.spawn().map_err(|error| error.to_string())?;
    println!(
        "publisher-receiver proxy listening on {} and forwarding to {}; admin listening on {}",
        server.local_addr().map_err(|error| error.to_string())?,
        config.authority_url,
        DEFAULT_ADMIN_BIND_ADDR
    );
    server.serve_forever().map_err(|error| error.to_string())
}
