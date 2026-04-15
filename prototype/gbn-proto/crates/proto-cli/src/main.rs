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
    path::PathBuf,
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
    create_multipath_router,
    observability::{
        publish_chunks_reassembled_from_env, publish_circuit_build_result_from_env,
        publish_path_diversity_from_env,
    },
    relay_engine, swarm,
};
use mcn_sanitizer::{is_ffmpeg_available, sanitize_video};
use mpub_receiver::{Receiver, SENTINEL_MAGIC};
use rand::RngCore;
use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};

/// Deterministic seed for synthetic payload — same constant on Creator and Publisher
/// so the Publisher can independently compute the expected BLAKE3 hash.
const GBN_PHASE2_SEED: u64 = 0x47424E5048415345; // "GBNPHASE" in LE bytes

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
                let report = sanitize_video(&input, &sanitized_path)?;
                println!(
                    "   Done: {} bytes -> {} bytes",
                    report.input_size, report.output_size
                );
            } else {
                println!("⚠️  FFmpeg not found. Skipping sanitization step. Using original file.");
                std::fs::copy(&input, &sanitized_path)?;
            }

            // 2. Hash Original
            let original_hash = hash_file(&sanitized_path)?;
            println!("📄 Sanitized file SHA-256: {}", hex::encode(original_hash));

            // 3. Chunk
            let t = Instant::now();
            println!("🧩 Chunking file ({} bytes/chunk)...", chunk_size);
            let (chunks, manifest) = chunk_file(&sanitized_path, chunk_size)?;
            println!(
                "   Generated {} chunks in {}ms",
                chunks.len(),
                t.elapsed().as_millis()
            );

            // 4. Crypto Session
            let session = create_upload_session(&pub_key, manifest.total_chunks, original_hash)?;

            // 5. Setup Receiver Network
            let mut listen_addrs = Vec::new();
            for _ in 0..paths {
                listen_addrs.push("127.0.0.1:0".parse().unwrap());
            }
            let receiver = Receiver::new(listen_addrs);
            let mut receiver_handle = receiver.start().await?;
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
            let router =
                create_multipath_router(receiver_handle.bound_addrs.clone(), hops, 50, 100).await?;
            println!("   Network ready in {}ms", t.elapsed().as_millis());

            // 7. Encrypt and Route
            let t = Instant::now();
            println!("🔒 Encrypting & routing chunks...");
            for (i, data) in chunks.iter().enumerate() {
                let info = &manifest.chunks[i];
                let packet = session.encrypt_chunk(info.index, data, info.hash)?;
                router.send_chunk(&packet).await?;
            }
            println!(
                "   All {} encrypted chunks dispatched in {}ms",
                chunks.len(),
                t.elapsed().as_millis()
            );

            // 8. Receiver Await
            println!("⏳ Receiver waiting for chunks...");
            let t = Instant::now();
            let completed = receiver_handle
                .await_session(manifest.session_id, Duration::from_secs(30))
                .await?;
            println!(
                "   Session {} complete in {}ms",
                hex::encode(completed.session_id),
                t.elapsed().as_millis()
            );

            // 9. Decrypt and Reassemble
            let t = Instant::now();
            println!("🔓 Decrypting and reassembling...");
            let reassembled_path = format!("{}.reassembled.mp4", input);
            completed.decrypt_and_reassemble(
                &reassembled_path,
                &pub_secret,
                &session.session_init,
                &manifest,
            )?;
            println!("   Wrote reassembled file in {}ms", t.elapsed().as_millis());

            // 10. Verify
            println!("✅ Verifying end-to-end byte perfection...");
            let matched = completed.verify(original_hash, &reassembled_path)?;
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

            match role.as_str() {
                "relay" => {
                    // P2: Run gossip swarm AND onion relay engine concurrently
                    let noise_priv_key = load_noise_privkey_from_env()
                        .context("Relay: failed to load Noise private key")?;

                    let local_key = libp2p::identity::Keypair::generate_ed25519();
                    let mut swarm_handle = swarm::build_swarm(local_key).await?;
                    let mut runtime = swarm::GossipRuntime::from_env().await;

                    // Spawn onion relay engine on GBN_ONION_PORT (default 9001)
                    let onion_port: u16 = std::env::var("GBN_ONION_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(9001);
                    let onion_addr: SocketAddr = format!("0.0.0.0:{}", onion_port).parse()?;
                    let onion_handle = relay_engine::spawn_onion_relay(
                        onion_addr,
                        noise_priv_key,
                        0,
                        0, // no artificial jitter in ECS
                    )
                    .await?;
                    tracing::info!("Relay: onion engine listening on :{}", onion_port);

                    swarm::run_swarm_until_ctrl_c(&mut swarm_handle, &mut runtime).await?;
                    onion_handle.shutdown().await;
                }

                "creator" => {
                    // P2: Gossip swarm + background circuit build + upload
                    let local_key = libp2p::identity::Keypair::generate_ed25519();
                    let mut swarm_handle = swarm::build_swarm(local_key).await?;
                    let mut runtime = swarm::GossipRuntime::from_env().await;

                    let circuit_delay_secs: u64 = std::env::var("GBN_CIRCUIT_DELAY_SECS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(60);
                    let target_circuits: usize = std::env::var("GBN_CIRCUIT_PATHS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(10);

                    tokio::spawn(async move {
                        tracing::info!(
                            "Creator: waiting {}s for gossip stabilization...",
                            circuit_delay_secs
                        );
                        tokio::time::sleep(Duration::from_secs(circuit_delay_secs)).await;
                        tracing::info!("Creator: starting circuit build and upload sequence");
                        if let Err(e) = run_creator_upload(target_circuits).await {
                            tracing::error!("Creator circuit/upload failed: {e:#}");
                        }
                    });

                    swarm::run_swarm_until_ctrl_c(&mut swarm_handle, &mut runtime).await?;
                }

                "publisher" => {
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

                    // Derive X25519 pubkey and register with Cloud Map
                    let pub_key_hex = hex::encode(pub_secret.x25519_public_key());
                    swarm::register_publisher_pubkey_in_cloudmap(&pub_key_hex).await?;

                    // Bind mpub-receiver TCP listener
                    let port: u16 = std::env::var("GBN_MPUB_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(7001);
                    let listen_addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
                    let receiver = Receiver::new(vec![listen_addr]);
                    let mut handle = receiver.start().await?;
                    tracing::info!("Publisher: mpub-receiver listening on :{}", port);

                    // Also run gossip swarm for Cloud Map keepalive
                    let local_key = libp2p::identity::Keypair::generate_ed25519();
                    let mut swarm_handle = swarm::build_swarm(local_key).await?;
                    let mut gossip_runtime = swarm::GossipRuntime::from_env().await;

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
                    loop {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {
                                tracing::info!("Publisher: Ctrl+C received, shutting down");
                                break;
                            }
                            _ = swarm::drive_swarm_once(&mut swarm_handle, &mut gossip_runtime) => {}
                            result = handle.await_any_session(Duration::from_secs(300)) => {
                                match result {
                                    Ok(session) => {
                                        tracing::info!(
                                            "Publisher: session {} received with {} chunks",
                                            hex::encode(session.session_id),
                                            session.packets.len()
                                        );

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
                                                match session.decrypt_and_reassemble(
                                                    &out_path, &pub_secret, &session_init, &manifest,
                                                ) {
                                                    Ok(()) => {
                                                        let hash_match = session.verify(expected_hash, &out_path)
                                                            .unwrap_or(false);
                                                        tracing::info!(
                                                            "Publisher: session {} reassembled: hash_match={}",
                                                            hex::encode(session.session_id),
                                                            hash_match
                                                        );
                                                        publish_chunks_reassembled_from_env(
                                                            session.packets.len() as u64, hash_match,
                                                        ).await;
                                                    }
                                                    Err(e) => {
                                                        tracing::error!("Publisher: reassembly failed: {e}");
                                                        publish_chunks_reassembled_from_env(0, false).await;
                                                    }
                                                }
                                            }
                                            None => {
                                                tracing::warn!(
                                                    "Publisher: session {} arrived without UploadSessionInit sentinel — cannot decrypt",
                                                    hex::encode(session.session_id)
                                                );
                                                publish_chunks_reassembled_from_env(0, false).await;
                                            }
                                        }
                                    }
                                    Err(e) => tracing::warn!("Publisher: await_any_session error: {e}"),
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
async fn run_creator_upload(target_circuits: usize) -> Result<()> {
    let circuit_build_start = Instant::now();

    // 1. Load Noise private key
    let noise_priv_key =
        load_noise_privkey_from_env().context("Creator: failed to load Noise private key")?;

    // 2. Discover relay nodes from Cloud Map — retry for up to 5 minutes.
    //    Relays may still be starting (fresh deploy) or in mid-churn (chaos cycle).
    //    We need at least 3 HostileSubnet + 1 FreeSubnet relays to build even one circuit.
    const DISCOVERY_RETRY_SECS: u64 = 30;
    const DISCOVERY_TIMEOUT_SECS: u64 = 300;
    let discovery_deadline = Instant::now() + Duration::from_secs(DISCOVERY_TIMEOUT_SECS);

    let (all_relays, exit_candidates) = loop {
        let relays = swarm::discover_relay_nodes_from_cloudmap()
            .await
            .context("Creator: failed to discover relay nodes from Cloud Map")?;
        let exits = select_exit_candidates(&relays);

        if !relays.is_empty() && !exits.is_empty() {
            tracing::info!(
                "Creator: discovered {} relay nodes ({} FreeSubnet exits)",
                relays.len(),
                exits.len()
            );
            break (relays, exits);
        }

        // Log which constraint is unmet so logs are actionable
        if relays.is_empty() {
            tracing::warn!(
                "Creator: 0 relay nodes with GBN_NOISE_PUBKEY_HEX found in Cloud Map \
                 (relays may still be starting or registering). \
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
                 {} total relays, {} FreeSubnet exits. \
                 Check entrypoint.sh noise key generation and Cloud Map registration.",
                DISCOVERY_TIMEOUT_SECS,
                relays.len(),
                exits.len()
            );
        }

        tokio::time::sleep(Duration::from_secs(DISCOVERY_RETRY_SECS)).await;
    };

    // 4. Build circuits speculatively (30 concurrent dials, keep first `target_circuits`)
    tracing::info!(
        "Creator: building {} circuits (30 concurrent speculative dials)...",
        target_circuits
    );
    let circuits = build_circuits_speculative(
        &noise_priv_key,
        &all_relays,
        &exit_candidates,
        target_circuits,
        30,
    )
    .await?;
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

    // 6. Discover Publisher address + X25519 pubkey
    let (_, pub_key_bytes) = swarm::discover_publisher_from_cloudmap()
        .await
        .context("Creator: failed to discover Publisher from Cloud Map")?;
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
    let payload_hash = hash_bytes(&payload);
    tracing::info!("Creator: payload BLAKE3 = {}", hex::encode(payload_hash));

    // 8. Chunk the payload (1MB chunks)
    let (chunks, manifest) = chunk_bytes(&payload, 1_048_576)?;
    tracing::info!(
        "Creator: {} chunks, session_id={}",
        manifest.total_chunks,
        hex::encode(manifest.session_id)
    );

    // 9. Create upload session (X25519 ECDH → AES-GCM session key)
    let session = create_upload_session(&pub_key_bytes, manifest.total_chunks, payload_hash)?;

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
    manager
        .send_chunk(u32::MAX, sentinel_payload.clone())
        .await
        .context("Creator: failed to send UploadSessionInit sentinel")?;
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
        let packet = session.encrypt_chunk(i, chunk_data, info.hash)?;
        let packet_bytes = serde_json::to_vec(&packet)?;
        manager
            .send_chunk(i, packet_bytes)
            .await
            .with_context(|| format!("Creator: failed to send chunk {i}"))?;
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
