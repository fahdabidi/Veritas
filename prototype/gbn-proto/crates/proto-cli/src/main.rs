//! # GBN Prototype CLI
//!
//! Command-line tool that orchestrates the full Media Creation Network pipeline
//! for testing and demonstration.
//!
//! ## ECS Roles (Phase 2)
//!
//! - `relay`     — runs gossip swarm on port 4001 AND onion relay on port 9001 concurrently
//! - `creator`   — runs gossip swarm, then after GBN_CIRCUIT_DELAY_SECS builds onion circuits
//!                 and uploads a deterministic synthetic payload through them
//! - `publisher` — loads private key from GBN_PUBLISHER_KEY_HEX, registers in Cloud Map,
//!                 runs mpub-receiver on port 7001, reassembles and BLAKE3-verifies chunks

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    net::SocketAddr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mcn_chunker::{chunk_bytes, chunk_file, hash_bytes, hash_file};

use mcn_crypto::{create_upload_session, generate_publisher_keypair, PublisherSecret};
use mcn_router_sim::{
    circuit_manager::{
        build_circuits_speculative, log_path_diversity, select_exit_candidates, CircuitManager,
    },
    control::spawn_control_server,
    gossip::GbnGossipMsg,
    create_multipath_router,
    observability::{
        publish_chunks_reassembled_from_env, publish_circuit_build_result_from_env,
        publish_path_diversity_from_env,
    },
    relay_engine, swarm::{self, SwarmControlCmd},
};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use mcn_sanitizer::{is_ffmpeg_available, sanitize_video};
use mpub_receiver::{Receiver, SENTINEL_MAGIC};
use rand::RngCore;
use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};

/// Deterministic seed for synthetic payload — same constant on Creator and Publisher
/// so the Publisher can independently compute the expected BLAKE3 hash.
const GBN_PHASE2_SEED: u64 = 0x47424E5048415345; // "GBNPHASE" in LE bytes

fn trace_next_chain(parent_chain: &str) -> String {
    let hop = mcn_router_sim::trace::next_hop_id();
    if parent_chain.is_empty() {
        hop
    } else if hop.is_empty() {
        parent_chain.to_string()
    } else {
        format!("{parent_chain} -> {hop}")
    }
}

fn trace_input(parent_chain: &str, component: &str, call: &str, params: &str) -> String {
    let chain = trace_next_chain(parent_chain);
    mcn_router_sim::control::push_packet_meta_trace(
        "ComponentInput",
        0,
        &format!("{component}.{call} INPUT params={params}"),
        &chain,
        "component.input",
    );
    chain
}

fn trace_output(chain: &str, component: &str, call: &str, output: &str, size_bytes: usize) {
    mcn_router_sim::control::push_packet_meta_trace(
        "ComponentOutput",
        size_bytes,
        &format!("{component}.{call} OUTPUT result={output}"),
        chain,
        "component.output",
    );
}

fn trace_error(chain: &str, component: &str, call: &str, err: &str) {
    mcn_router_sim::control::push_packet_meta_trace(
        "ComponentError",
        0,
        &format!("{component}.{call} ERROR err={err}"),
        chain,
        "component.error",
    );
}

#[derive(Parser)]
#[command(name = "gbn-proto")]
#[command(about = "Global Broadcast Network — Phase 1/2 Prototype CLI")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new Publisher keypair (Ed25519 + X25519)
    Keygen,

    /// Upload a video through the MCN pipeline (runs end-to-end locally)
    Upload {
        /// Path to the input video file
        #[arg(short, long)]
        input: String,

        /// Number of parallel relay paths
        #[arg(long, default_value = "3")]
        paths: usize,

        /// Number of relay hops per path
        #[arg(long, default_value = "3")]
        hops: usize,

        /// Chunk size in bytes (default 1MB)
        #[arg(long, default_value = "1048576")]
        chunk_size: usize,
    },

    /// Verify that a reassembled video matches the original
    Verify {
        /// Path to the original (sanitized) video
        #[arg(long)]
        original: String,

        /// Path to the reassembled video
        #[arg(long)]
        reassembled: String,
    },

    /// Run a long-running service (relay, creator, or publisher)
    Serve {
        /// Override the role inferred from GBN_ROLE env var
        #[arg(long)]
        role: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("gbn=debug".parse()?),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Keygen => {
            tracing::info!("Generating Publisher keypair...");
            let (secret, pubkey) = generate_publisher_keypair();

            let mut key_file = File::create("publisher.key")?;
            key_file.write_all(&secret.to_seed())?;

            let mut pub_file = File::create("publisher.pub")?;
            pub_file.write_all(&pubkey)?;

            println!("✅ Keys generated successfully.");
            println!("Private seed saved to: publisher.key (KEEP SECRET)");
            println!(
                "Public key saved to:   publisher.pub ({} bytes)",
                pubkey.len()
            );
            println!("Hex public key:        {}", hex::encode(pubkey));
            Ok(())
        }

        Commands::Upload {
            input,
            paths,
            hops,
            chunk_size,
        } => {
            let mut trace_chain = String::new();
            println!("🚀 Starting GBN Phase 1 MCN Pipeline");
            let total_start = Instant::now();

            // 0. Load keys
            let mut seed = [0u8; 32];
            File::open("publisher.key")
                .expect("Run `gbn-proto keygen` first to generate publisher.key")
                .read_exact(&mut seed)?;
            let pub_secret = PublisherSecret::from_seed(seed);

            let mut pub_key = [0u8; 32];
            File::open("publisher.pub")
                .expect("Run `gbn-proto keygen` first to generate publisher.pub")
                .read_exact(&mut pub_key)?;

            // 1. Sanitize
            let sanitized_path = format!("{}.sanitized.mp4", input);
            if is_ffmpeg_available() {
                println!("🎬 Sanitizing video (removing metadata)...");
                let c = trace_input(
                    &trace_chain,
                    "mcn-sanitizer",
                    "sanitize_video",
                    &format!("input={input},output={sanitized_path}"),
                );
                let report = match sanitize_video(&input, &sanitized_path) {
                    Ok(r) => r,
                    Err(e) => {
                        trace_error(&c, "mcn-sanitizer", "sanitize_video", &e.to_string());
                        return Err(e.into());
                    }
                };
                trace_output(
                    &c,
                    "mcn-sanitizer",
                    "sanitize_video",
                    &format!("input_size={},output_size={}", report.input_size, report.output_size),
                    report.output_size as usize,
                );
                trace_chain = c;
                println!(
                    "   Done: {} bytes -> {} bytes",
                    report.input_size, report.output_size
                );
            } else {
                println!("⚠️  FFmpeg not found. Skipping sanitization step. Using original file.");
                std::fs::copy(&input, &sanitized_path)?;
            }

            // 2. Hash Original
            let c = trace_input(
                &trace_chain,
                "mcn-chunker",
                "hash_file",
                &format!("path={sanitized_path}"),
            );
            let original_hash = match hash_file(&sanitized_path) {
                Ok(h) => h,
                Err(e) => {
                    trace_error(&c, "mcn-chunker", "hash_file", &e.to_string());
                    return Err(e.into());
                }
            };
            trace_output(
                &c,
                "mcn-chunker",
                "hash_file",
                &format!("hash={}", hex::encode(original_hash)),
                32,
            );
            trace_chain = c;
            println!("📄 Sanitized file SHA-256: {}", hex::encode(original_hash));

            // 3. Chunk
            let t = Instant::now();
            println!("🧩 Chunking file ({} bytes/chunk)...", chunk_size);
            let c = trace_input(
                &trace_chain,
                "mcn-chunker",
                "chunk_file",
                &format!("path={sanitized_path},chunk_size={chunk_size}"),
            );
            let (chunks, manifest) = match chunk_file(&sanitized_path, chunk_size) {
                Ok(v) => v,
                Err(e) => {
                    trace_error(&c, "mcn-chunker", "chunk_file", &e.to_string());
                    return Err(e.into());
                }
            };
            trace_output(
                &c,
                "mcn-chunker",
                "chunk_file",
                &format!("total_chunks={}", manifest.total_chunks),
                chunks.len(),
            );
            trace_chain = c;
            println!(
                "   Generated {} chunks in {}ms",
                chunks.len(),
                t.elapsed().as_millis()
            );

            // 4. Crypto Session
            let c = trace_input(
                &trace_chain,
                "mcn-crypto",
                "create_upload_session",
                &format!("total_chunks={}", manifest.total_chunks),
            );
            let session = match create_upload_session(&pub_key, manifest.total_chunks, original_hash) {
                Ok(s) => s,
                Err(e) => {
                    trace_error(&c, "mcn-crypto", "create_upload_session", &e.to_string());
                    return Err(e.into());
                }
            };
            trace_output(
                &c,
                "mcn-crypto",
                "create_upload_session",
                &format!("session_id={}", hex::encode(session.session_init.session_id)),
                0,
            );
            trace_chain = c;

            // 5. Setup Receiver Network
            let mut listen_addrs = Vec::new();
            for _ in 0..paths {
                listen_addrs.push("127.0.0.1:0".parse().unwrap());
            }
            let c = trace_input(
                &trace_chain,
                "mpub-receiver",
                "Receiver::new",
                &format!("listen_addrs={}", listen_addrs.len()),
            );
            let receiver = Receiver::new(listen_addrs);
            trace_output(&c, "mpub-receiver", "Receiver::new", "ok", 0);
            let c2 = trace_input(&c, "mpub-receiver", "Receiver::start", "async_start");
            let mut receiver_handle = match receiver.start().await {
                Ok(h) => h,
                Err(e) => {
                    trace_error(&c2, "mpub-receiver", "Receiver::start", &e.to_string());
                    return Err(e.into());
                }
            };
            trace_output(
                &c2,
                "mpub-receiver",
                "Receiver::start",
                &format!("bound_addrs={}", receiver_handle.bound_addrs.len()),
                receiver_handle.bound_addrs.len(),
            );
            trace_chain = c2;
            println!(
                "📡 Receiver tracking {} exit nodes",
                receiver_handle.bound_addrs.len()
            );

            // 6. Setup Router Network
            let t = Instant::now();
            println!(
                "🌐 Building multipath relay network ({} paths x {} hops)...",
                paths, hops
            );
            let c = trace_input(
                &trace_chain,
                "mcn-router-sim",
                "create_multipath_router",
                &format!("paths={},hops={}", receiver_handle.bound_addrs.len(), hops),
            );
            let router = match create_multipath_router(receiver_handle.bound_addrs.clone(), hops, 50, 100).await {
                Ok(r) => r,
                Err(e) => {
                    trace_error(&c, "mcn-router-sim", "create_multipath_router", &e.to_string());
                    return Err(e.into());
                }
            };
            trace_output(&c, "mcn-router-sim", "create_multipath_router", "ok", 0);
            trace_chain = c;
            println!("   Network ready in {}ms", t.elapsed().as_millis());

            // 7. Encrypt and Route
            let t = Instant::now();
            println!("🔒 Encrypting & routing chunks...");
            for (i, data) in chunks.iter().enumerate() {
                let info = &manifest.chunks[i];
                let c = trace_input(
                    &trace_chain,
                    "mcn-crypto",
                    "encrypt_chunk",
                    &format!("chunk_index={},len={}", info.index, data.len()),
                );
                let packet = match session.encrypt_chunk(info.index, data, info.hash) {
                    Ok(p) => p,
                    Err(e) => {
                        trace_error(&c, "mcn-crypto", "encrypt_chunk", &e.to_string());
                        return Err(e.into());
                    }
                };
                trace_output(&c, "mcn-crypto", "encrypt_chunk", "ok", packet.ciphertext.len());

                let c2 = trace_input(
                    &c,
                    "mcn-router-sim",
                    "Router::send_chunk",
                    &format!("chunk_index={}", info.index),
                );
                if let Err(e) = router.send_chunk(&packet).await {
                    trace_error(&c2, "mcn-router-sim", "Router::send_chunk", &e.to_string());
                    return Err(e.into());
                }
                trace_output(&c2, "mcn-router-sim", "Router::send_chunk", "ok", packet.ciphertext.len());
                trace_chain = c2;
            }
            println!(
                "   All {} encrypted chunks dispatched in {}ms",
                chunks.len(),
                t.elapsed().as_millis()
            );

            // 8. Receiver Await
            println!("⏳ Receiver waiting for chunks...");
            let t = Instant::now();
            let c = trace_input(
                &trace_chain,
                "mpub-receiver",
                "ReceiverHandle::await_session",
                &format!("session_id={}", hex::encode(manifest.session_id)),
            );
            let completed = match receiver_handle
                .await_session(manifest.session_id, Duration::from_secs(30))
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    trace_error(&c, "mpub-receiver", "ReceiverHandle::await_session", &e.to_string());
                    return Err(e.into());
                }
            };
            trace_output(
                &c,
                "mpub-receiver",
                "ReceiverHandle::await_session",
                &format!("packets={}", completed.packets.len()),
                completed.packets.len(),
            );
            trace_chain = c;
            println!(
                "   Session {} complete in {}ms",
                hex::encode(completed.session_id),
                t.elapsed().as_millis()
            );

            // 9. Decrypt and Reassemble
            let t = Instant::now();
            println!("🔓 Decrypting and reassembling...");
            let reassembled_path = format!("{}.reassembled.mp4", input);
            let c = trace_input(
                &trace_chain,
                "mpub-receiver",
                "CompletedSession::decrypt_and_reassemble",
                &format!("out={reassembled_path}"),
            );
            if let Err(e) = completed.decrypt_and_reassemble(
                &reassembled_path,
                &pub_secret,
                &session.session_init,
                &manifest,
            ) {
                trace_error(&c, "mpub-receiver", "CompletedSession::decrypt_and_reassemble", &e.to_string());
                return Err(e.into());
            }
            trace_output(&c, "mpub-receiver", "CompletedSession::decrypt_and_reassemble", "ok", 0);
            trace_chain = c;
            println!("   Wrote reassembled file in {}ms", t.elapsed().as_millis());

            // 10. Verify
            println!("✅ Verifying end-to-end byte perfection...");
            let c = trace_input(
                &trace_chain,
                "mpub-receiver",
                "CompletedSession::verify",
                &format!("path={reassembled_path}"),
            );
            let matched = match completed.verify(original_hash, &reassembled_path) {
                Ok(v) => v,
                Err(e) => {
                    trace_error(&c, "mpub-receiver", "CompletedSession::verify", &e.to_string());
                    return Err(e.into());
                }
            };
            trace_output(&c, "mpub-receiver", "CompletedSession::verify", &format!("matched={matched}"), 0);
            if matched {
                println!("   SUCCESS! Reassembled SHA-256 matches perfectly.");
            } else {
                println!("   FAILED! Reassembled content does not match original.");
                std::process::exit(1);
            }

            println!(
                "🎉 Pipeline complete in {}ms",
                total_start.elapsed().as_millis()
            );
            router.shutdown().await;
            receiver_handle.shutdown();
            Ok(())
        }

        Commands::Verify {
            original,
            reassembled,
        } => {
            println!("🔍 Verifying files...");
            let h1 = hash_file(&original)?;
            let h2 = hash_file(&reassembled)?;
            if h1 == h2 {
                println!("✅ MATCH: {}", hex::encode(h1));
            } else {
                println!("❌ MISMATCH!");
                println!("Original:    {}", hex::encode(h1));
                println!("Reassembled: {}", hex::encode(h2));
                std::process::exit(1);
            }
            Ok(())
        }

        Commands::Serve { role } => {
            let role = role
                .or_else(|| std::env::var("GBN_ROLE").ok())
                .unwrap_or_else(|| "relay".to_string());
            tracing::info!("Starting {} service", role);

            let noise_priv_key = load_noise_privkey_from_env().unwrap_or([0u8; 32]);
            let seed_store = Arc::new(RwLock::new(HashMap::new()));
            let (ctrl_tx, mut ctrl_rx) = mpsc::channel(32);
            let control_port: u16 = std::env::var("GBN_CONTROL_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5050);

            spawn_control_server(control_port, seed_store.clone(), ctrl_tx, noise_priv_key).await?;

            match role.as_str() {
                "relay" => {
                    let mut trace_chain = String::new();
                    let local_key = libp2p::identity::Keypair::generate_ed25519();
                    let c = trace_input(&trace_chain, "mcn-router-sim", "swarm::build_swarm", "role=relay");
                    let mut swarm_handle = swarm::build_swarm(local_key).await.map_err(|e| {
                        trace_error(&c, "mcn-router-sim", "swarm::build_swarm", &e.to_string());
                        e
                    })?;
                    trace_output(&c, "mcn-router-sim", "swarm::build_swarm", "ok", 0);
                    let c2 = trace_input(&c, "mcn-router-sim", "GossipRuntime::from_env", "role=relay");
                    let mut runtime = swarm::GossipRuntime::from_env(seed_store.clone()).await;
                    trace_output(&c2, "mcn-router-sim", "GossipRuntime::from_env", "ok", 0);
                    trace_chain = c2;

                    // Spawn onion relay engine on GBN_ONION_PORT (default 9001)
                    let onion_port: u16 = std::env::var("GBN_ONION_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(9001);
                    let onion_addr: SocketAddr = format!("0.0.0.0:{}", onion_port).parse()?;
                    let c3 = trace_input(
                        &trace_chain,
                        "mcn-router-sim",
                        "relay_engine::spawn_onion_relay",
                        &format!("listen={onion_addr}"),
                    );
                    let onion_handle = relay_engine::spawn_onion_relay(
                        onion_addr,
                        noise_priv_key,
                    )
                    .await
                    .map_err(|e| {
                        trace_error(&c3, "mcn-router-sim", "relay_engine::spawn_onion_relay", &e.to_string());
                        e
                    })?;
                    trace_output(&c3, "mcn-router-sim", "relay_engine::spawn_onion_relay", "ok", 0);
                    tracing::info!("Relay: onion engine listening on :{}", onion_port);

                    swarm::run_swarm_until_ctrl_c(&mut swarm_handle, &mut runtime, ctrl_rx).await?;
                    onion_handle.shutdown().await;
                }

                "creator" => {
                    let mut trace_chain = String::new();
                    // P2: Gossip swarm + background circuit build + upload
                    let local_key = libp2p::identity::Keypair::generate_ed25519();
                    let c = trace_input(&trace_chain, "mcn-router-sim", "swarm::build_swarm", "role=creator");
                    let mut swarm_handle = swarm::build_swarm(local_key).await.map_err(|e| {
                        trace_error(&c, "mcn-router-sim", "swarm::build_swarm", &e.to_string());
                        e
                    })?;
                    trace_output(&c, "mcn-router-sim", "swarm::build_swarm", "ok", 0);
                    let c2 = trace_input(&c, "mcn-router-sim", "GossipRuntime::from_env", "role=creator");
                    let mut runtime = swarm::GossipRuntime::from_env(seed_store.clone()).await;
                    trace_output(&c2, "mcn-router-sim", "GossipRuntime::from_env", "ok", 0);
                    trace_chain = c2;

                    let circuit_delay_secs: u64 = std::env::var("GBN_CIRCUIT_DELAY_SECS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(60);
                    let target_circuits: usize = std::env::var("GBN_CIRCUIT_PATHS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(10);

                    let upload_seed_store = seed_store.clone();
                    tokio::spawn(async move {
                        tracing::info!(
                            "Creator: waiting {}s for gossip stabilization...",
                            circuit_delay_secs
                        );
                        tokio::time::sleep(Duration::from_secs(circuit_delay_secs)).await;
                        tracing::info!("Creator: starting circuit build and upload sequence");
                        if let Err(e) = run_creator_upload(target_circuits, upload_seed_store).await {
                            tracing::error!("Creator circuit/upload failed: {e:#}");
                        }
                    });

                    swarm::run_swarm_until_ctrl_c(&mut swarm_handle, &mut runtime, ctrl_rx).await?;
                }

                "publisher" => {
                    let mut trace_chain = String::new();
                    // P2: Load private key from SSM-injected env, register in Cloud Map,
                    //     bind mpub-receiver, reassemble and BLAKE3-verify

                    let key_hex = std::env::var("GBN_PUBLISHER_KEY_HEX").context(
                        "GBN_PUBLISHER_KEY_HEX not set — check SSM parameter and ECS task role",
                    )?;
                    let seed_bytes =
                        hex::decode(&key_hex).context("Invalid hex in GBN_PUBLISHER_KEY_HEX")?;
                    anyhow::ensure!(
                        seed_bytes.len() >= 32,
                        "GBN_PUBLISHER_KEY_HEX must be at least 32 bytes (64 hex chars)"
                    );
                    let mut seed = [0u8; 32];
                    seed.copy_from_slice(&seed_bytes[..32]);
                    let pub_secret = PublisherSecret::from_seed(seed);

                    // Bind mpub-receiver TCP listener
                    let port: u16 = std::env::var("GBN_MPUB_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(7001);
                    let listen_addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
                    let c = trace_input(
                        &trace_chain,
                        "mpub-receiver",
                        "Receiver::new",
                        &format!("listen_addr={listen_addr}"),
                    );
                    let receiver = Receiver::new(vec![listen_addr]);
                    trace_output(&c, "mpub-receiver", "Receiver::new", "ok", 0);
                    let c2 = trace_input(&c, "mpub-receiver", "Receiver::start", "async_start");
                    let mut handle = receiver.start().await.map_err(|e| {
                        trace_error(&c2, "mpub-receiver", "Receiver::start", &e.to_string());
                        e
                    })?;
                    trace_output(
                        &c2,
                        "mpub-receiver",
                        "Receiver::start",
                        &format!("bound_addrs={}", handle.bound_addrs.len()),
                        handle.bound_addrs.len(),
                    );
                    trace_chain = c2;
                    tracing::info!("Publisher: mpub-receiver listening on :{}", port);

                    // Also run gossip swarm for Cloud Map keepalive
                    let local_key = libp2p::identity::Keypair::generate_ed25519();
                    let c3 = trace_input(&trace_chain, "mcn-router-sim", "swarm::build_swarm", "role=publisher");
                    let mut swarm_handle = swarm::build_swarm(local_key).await.map_err(|e| {
                        trace_error(&c3, "mcn-router-sim", "swarm::build_swarm", &e.to_string());
                        e
                    })?;
                    trace_output(&c3, "mcn-router-sim", "swarm::build_swarm", "ok", 0);
                    let c4 = trace_input(&c3, "mcn-router-sim", "GossipRuntime::from_env", "role=publisher");
                    let mut gossip_runtime = swarm::GossipRuntime::from_env(seed_store.clone()).await;
                    trace_output(&c4, "mcn-router-sim", "GossipRuntime::from_env", "ok", 0);
                    trace_chain = c4;

                    // Pre-compute expected BLAKE3 hash from the same deterministic seed
                    let upload_size: usize = std::env::var("GBN_UPLOAD_SIZE_BYTES")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(10_485_760);
                    let expected_hash = compute_expected_payload_hash(upload_size);
                    tracing::info!(
                        "Publisher: expected payload hash {}",
                        hex::encode(expected_hash)
                    );

                    // Publisher reassembly loop
                    tracing::info!("Publisher: entering reassembly loop");
                    let mut seed_broadcasted = false;
                    let boot_time = Instant::now();

                    loop {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {
                                tracing::info!("Publisher: Ctrl+C received, shutting down");
                                break;
                            }
                            Some(cmd) = ctrl_rx.recv() => {
                                match cmd {
                                    SwarmControlCmd::DumpDht(reply_tx) => {
                                        let mut peers = Vec::new();
                                        for bucket in swarm_handle.behaviour_mut().kademlia.kbuckets() {
                                            for entry in bucket.iter() {
                                                peers.push(entry.node.key.preimage().to_string());
                                            }
                                        }
                                        let _ = reply_tx.send(peers).await;
                                    }
                                    SwarmControlCmd::BroadcastSeed => {
                                        tracing::info!("Publisher: Executing manual BroadcastSeed from local seed store...");
                                        let nodes: Vec<mcn_router_sim::circuit_manager::RelayNode> = gossip_runtime.seed_store.read().unwrap().values().cloned().collect();
                                        tracing::info!("Publisher: Found {} nodes for manual BroadcastSeed", nodes.len());
                                        let msg = GbnGossipMsg::DirectorySync(nodes);
                                        let payload = serde_json::to_vec(&msg).unwrap();

                                        let mut msg_id = [0u8; 32];
                                        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                                        msg_id[0..8].copy_from_slice(&ts.to_le_bytes());
                                        msg_id[8..16].copy_from_slice(&0x5EED_5EED_u64.to_le_bytes());

                                        let outbound = gossip_runtime.engine.publish_local(msg_id, payload);
                                        for out_msg in outbound {
                                            swarm_handle.behaviour_mut().gossip.send_request(&out_msg.peer, out_msg.request);
                                        }
                                    }
                                    // Publisher role doesn't have a local relay identity to announce;
                                    // silently ignore UnicastDHT requests sent to it.
                                    SwarmControlCmd::UnicastDHT { .. } => {
                                        tracing::warn!("Publisher: UnicastDHT not supported on publisher role");
                                    }
                                }
                            }
                            _ = swarm::drive_swarm_once(&mut swarm_handle, &mut gossip_runtime) => {
                                // Once wait 15s to find peers, broadcast seed store to seed DHT network
                                if !seed_broadcasted && boot_time.elapsed() >= Duration::from_secs(15) {
                                    seed_broadcasted = true;
                                    tracing::info!("Publisher: Seed Node triggering local seed store broadcast...");
                                    let nodes: Vec<mcn_router_sim::circuit_manager::RelayNode> = gossip_runtime.seed_store.read().unwrap().values().cloned().collect();
                                    tracing::info!("Publisher: Found {} nodes from local seed store for Seed broadcast", nodes.len());
                                    if !nodes.is_empty() {
                                        let msg = GbnGossipMsg::DirectorySync(nodes);
                                        let payload = serde_json::to_vec(&msg).unwrap();
                                        
                                        let mut msg_id = [0u8; 32];
                                        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                                        msg_id[0..8].copy_from_slice(&ts.to_le_bytes());
                                        msg_id[8..16].copy_from_slice(&0x5EED_5EED_u64.to_le_bytes()); 

                                        let outbound = gossip_runtime.engine.publish_local(msg_id, payload);
                                        for out_msg in outbound {
                                            swarm_handle.behaviour_mut().gossip.send_request(&out_msg.peer, out_msg.request);
                                        }
                                    } else {
                                        tracing::warn!("Publisher: Seed Node found 0 nodes to broadcast.");
                                    }
                                }
                            }
                            result = handle.await_any_session(Duration::from_secs(300)) => {
                                match result {
                                    Ok(session) => {
                                        tracing::info!(
                                            "Publisher: session {} received with {} chunks",
                                            hex::encode(session.session_id),
                                            session.packets.len()
                                        );
                                        let recv_chain = trace_input(
                                            &trace_chain,
                                            "mpub-receiver",
                                            "await_any_session",
                                            &format!(
                                                "session_id={},chunks={}",
                                                hex::encode(session.session_id),
                                                session.packets.len()
                                            ),
                                        );
                                        trace_output(
                                            &recv_chain,
                                            "mpub-receiver",
                                            "await_any_session",
                                            "session_received",
                                            session.packets.len(),
                                        );
                                        trace_chain = recv_chain.clone();

                                        // Look up the UploadSessionInit delivered as a sentinel frame
                                        let session_init_opt = {
                                            let inits = handle.session_inits.lock().await;
                                            inits.get(&session.session_id).cloned()
                                        };

                                        match session_init_opt {
                                            Some(session_init) => {
                                                let manifest = build_manifest_from_session(&session);
                                                let out_path = format!(
                                                    "/tmp/gbn-reassembled-{}.bin",
                                                    hex::encode(session.session_id)
                                                );
                                                let dec_chain = trace_input(
                                                    &trace_chain,
                                                    "mpub-receiver",
                                                    "CompletedSession::decrypt_and_reassemble",
                                                    &format!("out={out_path}"),
                                                );
                                                match session.decrypt_and_reassemble(
                                                    &out_path, &pub_secret, &session_init, &manifest,
                                                ) {
                                                    Ok(()) => {
                                                        trace_output(
                                                            &dec_chain,
                                                            "mpub-receiver",
                                                            "CompletedSession::decrypt_and_reassemble",
                                                            "ok",
                                                            session.packets.len(),
                                                        );
                                                        let verify_chain = trace_input(
                                                            &dec_chain,
                                                            "mpub-receiver",
                                                            "CompletedSession::verify",
                                                            &format!("path={out_path}"),
                                                        );
                                                        let hash_match = match session.verify(expected_hash, &out_path) {
                                                            Ok(v) => {
                                                                trace_output(
                                                                    &verify_chain,
                                                                    "mpub-receiver",
                                                                    "CompletedSession::verify",
                                                                    &format!("matched={v}"),
                                                                    0,
                                                                );
                                                                v
                                                            }
                                                            Err(e) => {
                                                                trace_error(
                                                                    &verify_chain,
                                                                    "mpub-receiver",
                                                                    "CompletedSession::verify",
                                                                    &e.to_string(),
                                                                );
                                                                false
                                                            }
                                                        };
                                                        tracing::info!(
                                                            "Publisher: session {} reassembled: hash_match={}",
                                                            hex::encode(session.session_id),
                                                            hash_match
                                                        );
                                                        publish_chunks_reassembled_from_env(
                                                            session.packets.len() as u64, hash_match,
                                                        ).await;
                                                        trace_output(
                                                            &verify_chain,
                                                            "mcn-router-sim",
                                                            "publish_chunks_reassembled_from_env",
                                                            &format!("count={},hash_match={}", session.packets.len(), hash_match),
                                                            session.packets.len(),
                                                        );
                                                        trace_chain = verify_chain;
                                                    }
                                                    Err(e) => {
                                                        tracing::error!("Publisher: reassembly failed: {e}");
                                                        trace_error(
                                                            &dec_chain,
                                                            "mpub-receiver",
                                                            "CompletedSession::decrypt_and_reassemble",
                                                            &e.to_string(),
                                                        );
                                                        publish_chunks_reassembled_from_env(0, false).await;
                                                        trace_output(
                                                            &dec_chain,
                                                            "mcn-router-sim",
                                                            "publish_chunks_reassembled_from_env",
                                                            "count=0,hash_match=false",
                                                            0,
                                                        );
                                                        trace_chain = dec_chain;
                                                    }
                                                }
                                            }
                                            None => {
                                                tracing::warn!(
                                                    "Publisher: session {} arrived without UploadSessionInit sentinel — cannot decrypt",
                                                    hex::encode(session.session_id)
                                                );
                                                publish_chunks_reassembled_from_env(0, false).await;
                                                trace_error(
                                                    &trace_chain,
                                                    "mpub-receiver",
                                                    "lookup_session_init",
                                                    "missing UploadSessionInit sentinel",
                                                );
                                                trace_output(
                                                    &trace_chain,
                                                    "mcn-router-sim",
                                                    "publish_chunks_reassembled_from_env",
                                                    "count=0,hash_match=false",
                                                    0,
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        trace_error(
                                            &trace_chain,
                                            "mpub-receiver",
                                            "await_any_session",
                                            &e.to_string(),
                                        );
                                        tracing::warn!("Publisher: await_any_session error: {e}");
                                    }
                                }
                            }
                        }
                    }

                    handle.shutdown();
                }

                _ => {
                    anyhow::bail!("Unknown GBN_ROLE: {}", role);
                }
            }
            Ok(())
        }
    }
}

// ─────────────────────────── Phase 2: Creator Upload ──────────────────────

/// Full Creator circuit-build + upload sequence.
///
/// Called from a background task after `GBN_CIRCUIT_DELAY_SECS` gossip stabilization.
async fn run_creator_upload(target_circuits: usize, seed_store: Arc<RwLock<HashMap<SocketAddr, mcn_router_sim::circuit_manager::RelayNode>>>) -> Result<()> {
    let mut trace_chain = String::new();
    let circuit_build_start = Instant::now();

    // 1. Load Noise private key
    let c = trace_input(
        &trace_chain,
        "mcn-crypto",
        "load_noise_privkey_from_env",
        "from_env",
    );
    let noise_priv_key = match load_noise_privkey_from_env() {
        Ok(k) => k,
        Err(e) => {
            trace_error(&c, "mcn-crypto", "load_noise_privkey_from_env", &e.to_string());
            return Err(e.context("Creator: failed to load Noise private key"));
        }
    };
    trace_output(&c, "mcn-crypto", "load_noise_privkey_from_env", "ok", noise_priv_key.len());
    trace_chain = c;

    // 2. Discover relay nodes from Cloud Map — retry for up to 5 minutes.
    //    Relays may still be starting (fresh deploy) or in mid-churn (chaos cycle).
    //    We need at least 3 HostileSubnet + 1 FreeSubnet relays to build even one circuit.
    const DISCOVERY_RETRY_SECS: u64 = 30;
    const DISCOVERY_TIMEOUT_SECS: u64 = 300;
    let discovery_deadline = Instant::now() + Duration::from_secs(DISCOVERY_TIMEOUT_SECS);

    let (all_relays, exit_candidates) = loop {
        let discovered_nodes: Vec<mcn_router_sim::circuit_manager::RelayNode> =
            seed_store.read().unwrap().values().cloned().collect();
        let relays: Vec<mcn_router_sim::circuit_manager::RelayNode> = discovered_nodes
            .iter()
            .filter(|n| n.subnet_tag == "HostileSubnet" || n.subnet_tag == "FreeSubnet")
            .cloned()
            .collect();
        let exits = select_exit_candidates(&relays);

        if !relays.is_empty() && !exits.is_empty() {
            tracing::info!(
                "Creator: discovered {} relay-capable nodes ({} FreeSubnet exits) from local seed store ({} total announced nodes)",
                relays.len(),
                exits.len(),
                discovered_nodes.len()
            );
            break (relays, exits);
        }

        // Log which constraint is unmet so logs are actionable
        if relays.is_empty() {
            tracing::warn!(
                "Creator: 0 relay-capable nodes found in local seed store \
                 (nodes may still be starting or converging via Gossip). \
                 Retrying in {}s ({}s remaining)...",
                DISCOVERY_RETRY_SECS,
                discovery_deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs()
            );
        } else {
            tracing::warn!(
                "Creator: {} total relays found but 0 FreeSubnet exits \
                 (check GBN_SUBNET_TAG=FreeSubnet on free-relay tasks). \
                 Retrying in {}s ({}s remaining)...",
                relays.len(),
                DISCOVERY_RETRY_SECS,
                discovery_deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs()
            );
        }

        if Instant::now() >= discovery_deadline {
            publish_circuit_build_result_from_env(false, circuit_build_start.elapsed().as_millis())
                .await;
            anyhow::bail!(
                "Creator: relay discovery timed out after {}s — \
                 {} relay-capable nodes, {} FreeSubnet exits, {} total announced nodes. \
                 Check entrypoint.sh noise key generation and Cloud Map registration.",
                DISCOVERY_TIMEOUT_SECS,
                relays.len(),
                exits.len(),
                discovered_nodes.len()
            );
        }

        tokio::time::sleep(Duration::from_secs(DISCOVERY_RETRY_SECS)).await;
    };

    // 4. Build circuits speculatively (30 concurrent dials, keep first `target_circuits`)
    tracing::info!(
        "Creator: building {} circuits (30 concurrent speculative dials)...",
        target_circuits
    );
    let c = trace_input(
        &trace_chain,
        "mcn-router-sim",
        "build_circuits_speculative",
        &format!(
            "target_circuits={},relays={},exits={}",
            target_circuits,
            all_relays.len(),
            exit_candidates.len()
        ),
    );
    let circuits = build_circuits_speculative(
        &noise_priv_key,
        &all_relays,
        &exit_candidates,
        target_circuits,
        30,
    )
    .await
    .map_err(|e| {
        trace_error(&c, "mcn-router-sim", "build_circuits_speculative", &e.to_string());
        e
    })?;
    trace_output(
        &c,
        "mcn-router-sim",
        "build_circuits_speculative",
        &format!("circuits={}", circuits.len()),
        circuits.len(),
    );
    trace_chain = c;
    let circuit_build_latency_ms = circuit_build_start.elapsed().as_millis();
    tracing::info!(
        "Creator: built {} circuits in {}ms",
        circuits.len(),
        circuit_build_latency_ms
    );
    publish_circuit_build_result_from_env(!circuits.is_empty(), circuit_build_latency_ms).await;
    if circuits.is_empty() {
        anyhow::bail!("Creator: build_circuits_speculative returned 0 circuits — all dial attempts timed out or failed");
    }

    // 5. Verify path disjointness (test spec §5.5)
    let diversity_ok = log_path_diversity(&circuits);
    publish_path_diversity_from_env(diversity_ok).await;

    // 6. Discover Publisher address + X25519 pubkey (Static)
    let c = trace_input(
        &trace_chain,
        "mcn-router-sim",
        "discover_publisher_static",
        "static-seed",
    );
    let (_, pub_key_bytes) = swarm::discover_publisher_static()
        .await
        .map_err(|e| {
            trace_error(&c, "mcn-router-sim", "discover_publisher_static", &e.to_string());
            e
        })
        .context("Creator: failed to discover Publisher from static env configuration")?;
    trace_output(
        &c,
        "mcn-router-sim",
        "discover_publisher_static",
        &format!("pubkey={}", hex::encode(pub_key_bytes)),
        pub_key_bytes.len(),
    );
    trace_chain = c;
    tracing::info!(
        "Creator: discovered Publisher, pubkey={}",
        hex::encode(pub_key_bytes)
    );

    // 7. Generate deterministic synthetic payload (same seed as Publisher uses for verification)
    let upload_size: usize = std::env::var("GBN_UPLOAD_SIZE_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_485_760);
    tracing::info!(
        "Creator: generating {}MB synthetic payload...",
        upload_size / 1_048_576
    );
    let mut rng = ChaCha8Rng::seed_from_u64(GBN_PHASE2_SEED);
    let mut payload = vec![0u8; upload_size];
    rng.fill_bytes(&mut payload);
    let c = trace_input(
        &trace_chain,
        "mcn-chunker",
        "hash_bytes",
        &format!("payload_len={}", payload.len()),
    );
    let payload_hash = hash_bytes(&payload);
    trace_output(
        &c,
        "mcn-chunker",
        "hash_bytes",
        &format!("hash={}", hex::encode(payload_hash)),
        32,
    );
    trace_chain = c;
    tracing::info!("Creator: payload BLAKE3 = {}", hex::encode(payload_hash));

    // 8. Chunk the payload (1MB chunks)
    let c = trace_input(
        &trace_chain,
        "mcn-chunker",
        "chunk_bytes",
        &format!("payload_len={},chunk_size=1048576", payload.len()),
    );
    let (chunks, manifest) = chunk_bytes(&payload, 1_048_576).map_err(|e| {
        trace_error(&c, "mcn-chunker", "chunk_bytes", &e.to_string());
        e
    })?;
    trace_output(
        &c,
        "mcn-chunker",
        "chunk_bytes",
        &format!("total_chunks={}", manifest.total_chunks),
        chunks.len(),
    );
    trace_chain = c;
    tracing::info!(
        "Creator: {} chunks, session_id={}",
        manifest.total_chunks,
        hex::encode(manifest.session_id)
    );

    // 9. Create upload session (X25519 ECDH → AES-GCM session key)
    let c = trace_input(
        &trace_chain,
        "mcn-crypto",
        "create_upload_session",
        &format!("total_chunks={}", manifest.total_chunks),
    );
    let session = create_upload_session(&pub_key_bytes, manifest.total_chunks, payload_hash).map_err(|e| {
        trace_error(&c, "mcn-crypto", "create_upload_session", &e.to_string());
        e
    })?;
    trace_output(
        &c,
        "mcn-crypto",
        "create_upload_session",
        &format!("session_id={}", hex::encode(session.session_init.session_id)),
        0,
    );
    trace_chain = c;

    // 10. Build CircuitManager and add all circuits
    let manager = CircuitManager::new();
    for circuit in circuits {
        manager.add_circuit(circuit).await;
    }

    // 11. Send UploadSessionInit sentinel through each circuit
    //     Uses SENTINEL_MAGIC prefix so Publisher distinguishes it from EncryptedChunkPackets.
    let sentinel_payload = {
        let init_json = serde_json::to_vec(&session.session_init)?;
        let mut p = SENTINEL_MAGIC.to_vec();
        p.extend_from_slice(&init_json);
        p
    };
    // Send sentinel on chunk index u32::MAX through all circuits
    let c = trace_input(
        &trace_chain,
        "mcn-router-sim",
        "CircuitManager::send_chunk",
        "chunk_index=u32::MAX(sentinel)",
    );
    manager
        .send_chunk(u32::MAX, sentinel_payload.clone())
        .await
        .map_err(|e| {
            trace_error(&c, "mcn-router-sim", "CircuitManager::send_chunk", &e.to_string());
            e
        })
        .context("Creator: failed to send UploadSessionInit sentinel")?;
    trace_output(
        &c,
        "mcn-router-sim",
        "CircuitManager::send_chunk",
        "sentinel_sent",
        sentinel_payload.len(),
    );
    trace_chain = c;
    tracing::info!(
        "Creator: sent UploadSessionInit sentinel ({} bytes)",
        sentinel_payload.len()
    );

    // 12. Encrypt and route each chunk through the circuit manager (round-robin)
    tracing::info!(
        "Creator: sending {} encrypted chunks through onion circuits...",
        manifest.total_chunks
    );
    for i in 0..manifest.total_chunks {
        let chunk_data = &chunks[i as usize];
        let info = &manifest.chunks[i as usize];
        let c = trace_input(
            &trace_chain,
            "mcn-crypto",
            "encrypt_chunk",
            &format!("chunk_index={i},len={}", chunk_data.len()),
        );
        let packet = session.encrypt_chunk(i, chunk_data, info.hash).map_err(|e| {
            trace_error(&c, "mcn-crypto", "encrypt_chunk", &e.to_string());
            e
        })?;
        trace_output(&c, "mcn-crypto", "encrypt_chunk", "ok", packet.ciphertext.len());
        let packet_bytes = serde_json::to_vec(&packet)?;
        let c2 = trace_input(
            &c,
            "mcn-router-sim",
            "CircuitManager::send_chunk",
            &format!("chunk_index={i},packet_bytes={}", packet_bytes.len()),
        );
        manager
            .send_chunk(i, packet_bytes)
            .await
            .map_err(|e| {
                trace_error(&c2, "mcn-router-sim", "CircuitManager::send_chunk", &e.to_string());
                e
            })
            .with_context(|| format!("Creator: failed to send chunk {i}"))?;
        trace_output(&c2, "mcn-router-sim", "CircuitManager::send_chunk", "ok", chunk_data.len());
        trace_chain = c2;
    }

    tracing::info!(
        "Creator: all {} chunks dispatched ✅",
        manifest.total_chunks
    );
    Ok(())
}

// ─────────────────────────── Phase 2: Publisher Helpers ───────────────────

/// Reconstruct a `ChunkManifest` from the packets in a `CompletedSession`.
///
/// The manifest is derived from the `EncryptedChunkPacket` metadata:
/// - `session_id` from the first packet
/// - `total_chunks` from the first packet's `total_chunks` field
/// - Per-chunk `plaintext_hash` and `size` from each packet
fn build_manifest_from_session(
    session: &mpub_receiver::CompletedSession,
) -> gbn_protocol::chunk::ChunkManifest {
    let first = session.packets.values().next();
    let total_chunks = first.map(|p| p.total_chunks).unwrap_or(0);
    let mut chunks: Vec<gbn_protocol::chunk::ChunkInfo> = session
        .packets
        .values()
        .map(|p| {
            gbn_protocol::chunk::ChunkInfo {
                index: p.chunk_index,
                hash: p.plaintext_hash,
                size: 0, // will be set after decryption; placeholder for reassembly ordering
            }
        })
        .collect();
    chunks.sort_by_key(|c| c.index);

    gbn_protocol::chunk::ChunkManifest {
        session_id: session.session_id,
        total_chunks,
        content_hash: [0u8; 32], // not available pre-decryption
        total_size: 0,
        chunks,
    }
}

/// Compute the expected BLAKE3 hash of the synthetic payload independently.
///
/// Uses the same `GBN_PHASE2_SEED` and `upload_size` as the Creator,
/// so the Publisher can verify byte-perfect delivery without receiving the original.
fn compute_expected_payload_hash(upload_size: usize) -> [u8; 32] {
    let mut rng = ChaCha8Rng::seed_from_u64(GBN_PHASE2_SEED);
    let mut payload = vec![0u8; upload_size];
    rng.fill_bytes(&mut payload);
    hash_bytes(&payload)
}

// ─────────────────────────── Noise Key Helpers ────────────────────────────

/// Load a 32-byte Noise_XX Curve25519 private key from the `GBN_NOISE_PRIVKEY_HEX` env var.
///
/// If the env var is not set, generates an ephemeral random key and logs a warning.
/// In ECS, `entrypoint.sh` always sets this from `openssl rand -hex 32`.
fn load_noise_privkey_from_env() -> Result<[u8; 32]> {
    match std::env::var("GBN_NOISE_PRIVKEY_HEX") {
        Ok(hex_str) if !hex_str.is_empty() => {
            let bytes = hex::decode(&hex_str).context("GBN_NOISE_PRIVKEY_HEX is not valid hex")?;
            anyhow::ensure!(
                bytes.len() == 32,
                "GBN_NOISE_PRIVKEY_HEX must be exactly 32 bytes (64 hex chars)"
            );
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        _ => {
            tracing::warn!("GBN_NOISE_PRIVKEY_HEX not set — generating ephemeral Noise key (not suitable for ECS deployment)");
            let mut key = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            Ok(key)
        }
    }
}
