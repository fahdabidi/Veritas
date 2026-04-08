//! # GBN Prototype CLI
//!
//! Command-line tool that orchestrates the full Media Creation Network pipeline
//! for testing and demonstration.

use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    time::Instant,
};

use clap::{Parser, Subcommand};
use gbn_protocol::chunk::EncryptedChunkPacket;
use mcn_chunker::{chunk_file, hash_file};
use mcn_crypto::{create_upload_session, generate_publisher_keypair, PublisherSecret};
use mcn_router_sim::create_multipath_router;
use mcn_sanitizer::{is_ffmpeg_available, sanitize_video};
use mpub_receiver::Receiver;

#[derive(Parser)]
#[command(name = "gbn-proto")]
#[command(about = "Global Broadcast Network — Phase 1 Prototype CLI")]
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("gbn=debug".parse()?),
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
            println!("Public key saved to:   publisher.pub ({} bytes)", pubkey.len());
            println!("Hex public key:        {}", hex::encode(pubkey));
        }
        
        Commands::Upload { input, paths, hops, chunk_size } => {
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
            let t = Instant::now();
            let sanitized_path = format!("{}.sanitized.mp4", input);
            if is_ffmpeg_available() {
                println!("🎬 Sanitizing video (removing metadata)...");
                let report = sanitize_video(&input, &sanitized_path)?;
                println!("   Done in {}ms: {} bytes -> {} bytes", report.duration_ms, report.input_size, report.output_size);
            } else {
                println!("⚠️  FFmpeg not found. Skipping sanitization step. Using original file.");
                std::fs::copy(&input, &sanitized_path)?;
            }

            // 2. Hash Original (to verify reassembly later)
            let original_hash = hash_file(&sanitized_path)?;
            println!("📄 Sanitized file SHA-256: {}", hex::encode(original_hash));

            // 3. Chunk
            let t = Instant::now();
            println!("🧩 Chunking file ({} bytes/chunk)...", chunk_size);
            let (chunks, manifest) = chunk_file(&sanitized_path, chunk_size)?;
            println!("   Generated {} chunks in {}ms", chunks.len(), t.elapsed().as_millis());
            println!("   Session ID: {}", hex::encode(manifest.session_id));

            // 4. Crypto Session
            let session = create_upload_session(&pub_key, manifest.total_chunks, original_hash)?;

            // 5. Setup Receiver Network
            let mut listen_addrs = Vec::new();
            for _ in 0..paths {
                listen_addrs.push("127.0.0.1:0".parse().unwrap());
            }
            let receiver = Receiver::new(listen_addrs);
            let mut receiver_handle = receiver.start().await?;
            println!("📡 Receiver tracking {} exit nodes", receiver_handle.bound_addrs.len());

            // 6. Setup Router Network
            let t = Instant::now();
            println!("🌐 Building multipath relay network ({} paths x {} hops)...", paths, hops);
            // 50-100ms jitter per hop
            let router = create_multipath_router(receiver_handle.bound_addrs.clone(), hops, 50, 100).await?;
            println!("   Network ready in {}ms", t.elapsed().as_millis());

            // 7. Encrypt and Route
            let t = Instant::now();
            println!("🔒 Encrypting & routing chunks...");
            for (i, data) in chunks.iter().enumerate() {
                let info = &manifest.chunks[i];
                let packet = session.encrypt_chunk(info.index, data, info.hash)?;
                router.send_chunk(&packet).await?;
            }
            println!("   All {} encrypted chunks dispatched to Tor-like network in {}ms", chunks.len(), t.elapsed().as_millis());

            // 8. Receiver Await
            println!("⏳ Receiver waiting for chunks (with simulated network jitter)...");
            let t = Instant::now();
            let completed = receiver_handle.await_session(manifest.session_id, std::time::Duration::from_secs(30)).await?;
            println!("   Session {} complete! All chunks arrived in {}ms", hex::encode(completed.session_id), t.elapsed().as_millis());

            // 9. Decrypt and Reassemble
            let t = Instant::now();
            println!("🔓 Decrypting and reassembling...");
            let reassembled_path = format!("{}.reassembled.mp4", input);
            completed.decrypt_and_reassemble(&reassembled_path, &pub_secret, &session.session_init, &manifest)?;
            println!("   Wrote reassembled file to {} in {}ms", reassembled_path, t.elapsed().as_millis());

            // 10. Verify
            println!("✅ Verifying end-to-end byte perfection...");
            let matched = completed.verify(original_hash, &reassembled_path)?;
            if matched {
                println!("   SUCCESS! Reassembled SHA-256 matches perfectly.");
            } else {
                println!("   FAILED! Reassembled content does not match original.");
                std::process::exit(1);
            }

            println!("🎉 Pipeline complete in {}ms", total_start.elapsed().as_millis());

            // Cleanup
            router.shutdown().await;
            receiver_handle.shutdown();
        }

        Commands::Verify { original, reassembled } => {
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
        }
    }

    Ok(())
}
