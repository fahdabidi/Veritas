use anyhow::{Context, Result};
use snow::{Builder, HandshakeState, TransportState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

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
    initiator: bool,
) -> Result<TransportState> {
    let mut buf = vec![0u8; 65535];
    let mut msg = vec![0u8; 65535];

    let send_first = initiator;

    loop {
        if hs.is_handshake_finished() {
            break;
        }

        if (send_first && hs.get_handshake_hash().len() % 2 == 0)
            || (!send_first && hs.get_handshake_hash().len() % 2 != 0)
        {
            let len = hs.write_message(&[], &mut buf)?;
            let payload = &buf[..len];
            stream
                .write_all(&(payload.len() as u32).to_le_bytes())
                .await?;
            stream.write_all(payload).await?;
            stream.flush().await?;
        } else {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len = u32::from_le_bytes(len_buf) as usize;
            let raw = &mut msg[..len];
            stream.read_exact(raw).await?;
            hs.read_message(raw, &mut buf)?;
        }
    }

    Ok(hs.into_transport_mode()?)
}
