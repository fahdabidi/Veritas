use gbn_bridge_publisher::{ReceiverProxyConfig, ReceiverProxyServer};

fn main() {
    if let Err(error) = run() {
        eprintln!("publisher-receiver startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = ReceiverProxyConfig::from_env()?;
    let server = ReceiverProxyServer::bind(config.clone()).map_err(|error| error.to_string())?;
    println!(
        "publisher-receiver proxy listening on {} and forwarding to {}",
        server.local_addr().map_err(|error| error.to_string())?,
        config.authority_url
    );
    server.serve_forever().map_err(|error| error.to_string())
}
