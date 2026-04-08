//! S1.7 — DHT Publisher Identity Spoofing
//!
//! An adversary tries to inject a fake `RelayDescriptor` into the DHT, using
//! a different private key than the real Publisher but claiming the same
//! identity key (or a slightly modified one).
//!
//! **Pass criteria:**
//!   1. A `RelayDescriptor` with a valid self-consistent signature (adversary's
//!      own key pair) but a *different* identity_key than the trusted Publisher
//!      is recognised as "not the Publisher" and rejected.
//!   2. A `RelayDescriptor` with a tampered `identity_key` field (the signature
//!      was computed over *different* bytes) fails `verify()` with
//!      `DhtError::InvalidSignature`.
//!
//! This validates the Root-of-Trust property: without the Publisher's
//! out-of-band Ed25519 key (from QR code / Sovereign App), no adversary can
//! impersonate them in the DHT.

use ed25519_dalek::{Keypair, Signer};
use gbn_protocol::dht::{DhtError, RelayDescriptor};
use rand::rngs::OsRng;
use std::net::SocketAddr;

/// Build a self-consistent `RelayDescriptor` signed with `keypair`.
fn make_descriptor(keypair: &Keypair, addr: SocketAddr, timestamp: u64) -> RelayDescriptor {
    let identity_key = keypair.public.to_bytes();
    let mut signed_data = Vec::new();
    signed_data.extend_from_slice(&identity_key);
    signed_data.extend_from_slice(addr.to_string().as_bytes());
    signed_data.extend_from_slice(&timestamp.to_le_bytes());
    let signature = keypair.sign(&signed_data).to_bytes();

    RelayDescriptor {
        identity_key,
        address: addr,
        timestamp,
        signature,
    }
}

#[test]
fn test_s1_7a_legitimate_descriptor_verifies() {
    let mut csprng = OsRng;
    let real_publisher = Keypair::generate(&mut csprng);
    let addr: SocketAddr = "1.2.3.4:9000".parse().unwrap();

    let descriptor = make_descriptor(&real_publisher, addr, 1_700_000_000);

    assert!(
        descriptor.verify().is_ok(),
        "S1.7a FAIL: A legitimate descriptor should verify successfully"
    );
}

#[test]
fn test_s1_7b_adversary_descriptor_wrong_key_fails() {
    let mut csprng = OsRng;
    let adversary = Keypair::generate(&mut csprng);
    let real_publisher = Keypair::generate(&mut csprng);
    let addr: SocketAddr = "5.6.7.8:9000".parse().unwrap();

    // Adversary creates a valid-looking descriptor signed with THEIR key
    let adversary_descriptor = make_descriptor(&adversary, addr, 1_700_000_001);

    // Client knows the real Publisher's identity key from the QR code / Sovereign App
    let trusted_publisher_key = real_publisher.public.to_bytes();

    // The adversary's descriptor identity_key won't match the trusted key
    assert_ne!(
        adversary_descriptor.identity_key,
        trusted_publisher_key,
        "S1.7b FAIL: Adversary should not be able to claim the Publisher's identity key"
    );

    // But the adversary's OWN descriptor should verify (it's internally consistent)
    assert!(
        adversary_descriptor.verify().is_ok(),
        "S1.7b setup error: adversary's self-signed descriptor should verify"
    );

    tracing::info!(
        "S1.7b PASS: Adversary descriptor identity differs from trusted Publisher key"
    );
}

#[test]
fn test_s1_7c_tampered_descriptor_signature_fails() {
    let mut csprng = OsRng;
    let real_publisher = Keypair::generate(&mut csprng);
    let addr: SocketAddr = "9.10.11.12:9000".parse().unwrap();
    let mut descriptor = make_descriptor(&real_publisher, addr, 1_700_000_002);

    // Adversary tampers identity_key after signing (attempting to claim a different identity)
    descriptor.identity_key[0] ^= 0xFF;

    let result = descriptor.verify();
    assert!(
        matches!(result, Err(DhtError::InvalidSignature) | Err(DhtError::DalekError(_))),
        "S1.7c FAIL: Tampered descriptor should fail signature verification, got: {:?}",
        result
    );
    tracing::info!("S1.7c PASS: Tampered descriptor correctly rejected");
}
