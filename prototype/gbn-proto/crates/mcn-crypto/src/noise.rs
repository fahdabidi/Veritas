use anyhow::{Context, Result};
use snow::{Builder, HandshakeState, TransportState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
pub const NOISE_ONE_SHOT_PATTERN: &str = "Noise_N_25519_ChaChaPoly_BLAKE2s";

/// Initialize an Initiator for a Noise_XX handshake.
/// The Initiator drives the connection, passing their local private key and the
/// exact remote public key they expect to dial.
pub fn build_initiator(local_priv: &[u8], remote_pub: &[u8]) -> Result<HandshakeState> {
    let builder = Builder::new(NOISE_PATTERN.parse()?);
    let state = builder
        .local_private_key(local_priv)
        .remote_public_key(remote_pub)
        .build_initiator()
        .context("Failed to build Noise_XX initiator")?;
    Ok(state)
}

/// Initialize a Responder for a Noise_XX handshake.
/// The Responder expects incoming traffic and uses their local private key.
pub fn build_responder(local_priv: &[u8]) -> Result<HandshakeState> {
    let builder = Builder::new(NOISE_PATTERN.parse()?);
    let state = builder
        .local_private_key(local_priv)
        .build_responder()
        .context("Failed to build Noise_XX responder")?;
    Ok(state)
}

/// Encrypts an outgoing payload via established TransportState.
pub fn encrypt_frame(transport: &mut TransportState, payload: &[u8]) -> Result<Vec<u8>> {
    let mut ciphertext = vec![0u8; payload.len() + 65535]; // snow max message size buffer
    let len = transport.write_message(payload, &mut ciphertext)?;
    ciphertext.truncate(len);
    Ok(ciphertext)
}

/// Decrypts an incoming frame via established TransportState.
pub fn decrypt_frame(transport: &mut TransportState, ciphertext: &[u8]) -> Result<Vec<u8>> {
    let mut plaintext = vec![0u8; ciphertext.len()];
    let len = transport.read_message(ciphertext, &mut plaintext)?;
    plaintext.truncate(len);
    Ok(plaintext)
}

/// Complete a Noise_XX handshake over a length-prefixed TCP stream.
pub async fn complete_handshake(
    stream: &mut TcpStream,
    mut hs: HandshakeState,
    _initiator: bool,
) -> Result<TransportState> {
    let mut buf = vec![0u8; 65535];
    let mut msg = vec![0u8; 65535];

    loop {
        if hs.is_handshake_finished() {
            break;
        }

        // Let the Noise state machine determine turn order.
        // If write is attempted out of turn, Snow returns NotTurnToWrite.
        match hs.write_message(&[], &mut buf) {
            Ok(len) => {
                let payload = &buf[..len];
                stream
                    .write_all(&(payload.len() as u32).to_le_bytes())
                    .await?;
                stream.write_all(payload).await?;
                stream.flush().await?;
            }
            Err(e) if e.to_string().contains("NotTurnToWrite") => {
                let mut len_buf = [0u8; 4];
                stream.read_exact(&mut len_buf).await?;
                let len = u32::from_le_bytes(len_buf) as usize;
                let raw = &mut msg[..len];
                stream.read_exact(raw).await?;
                hs.read_message(raw, &mut buf)?;
            }
            Err(e) => return Err(e.into()),
        }
    }

    Ok(hs.into_transport_mode()?)
}

/// One-shot seal for onion payloads using Noise_N.
///
/// The sender only needs the recipient's static public key and sends a single
/// encrypted message without an interactive handshake round-trip.
pub fn seal(recipient_pub: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let builder = Builder::new(NOISE_ONE_SHOT_PATTERN.parse()?);
    let mut hs = builder
        .remote_public_key(recipient_pub)
        .build_initiator()
        .context("Failed to build Noise_N initiator")?;
    let mut buf = vec![0u8; plaintext.len() + 128];
    let len = hs.write_message(plaintext, &mut buf)?;
    buf.truncate(len);
    Ok(buf)
}

/// One-shot open for onion payloads using Noise_N.
///
/// The recipient uses its static private key to decrypt a single inbound
/// ciphertext frame.
pub fn open(local_priv: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
    let builder = Builder::new(NOISE_ONE_SHOT_PATTERN.parse()?);
    let mut hs = builder
        .local_private_key(local_priv)
        .build_responder()
        .context("Failed to build Noise_N responder")?;
    let mut buf = vec![0u8; ciphertext.len()];
    let len = hs.read_message(ciphertext, &mut buf)?;
    buf.truncate(len);
    Ok(buf)
}
