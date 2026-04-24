use std::net::TcpStream;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    BootstrapProgress, BridgeCommandAck, BridgeCommandAckStatus, BridgeControlCommand,
    BridgeControlFrame, BridgeControlHello, BridgeControlHelloUnsigned, BridgeControlKeepalive,
    BridgeControlProgress, ProtocolError, PublicKeyBytes,
};
use tungstenite::{connect, Message, WebSocket};

use crate::{RuntimeError, RuntimeResult};

#[derive(Debug)]
pub struct BridgeControlClient {
    bridge_id: String,
    publisher_pub: PublicKeyBytes,
    session_id: String,
    last_acked_seq_no: Option<u64>,
    socket: WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
}

impl BridgeControlClient {
    #[allow(clippy::too_many_arguments)]
    pub fn connect(
        url: &str,
        bridge_id: &str,
        lease_id: &str,
        bridge_pub: &PublicKeyBytes,
        signing_key: &SigningKey,
        publisher_pub: &PublicKeyBytes,
        chain_id: &str,
        request_id: &str,
        now_ms: u64,
        resume_acked_seq_no: Option<u64>,
        max_skew_ms: u64,
    ) -> RuntimeResult<Self> {
        let (mut socket, _) = connect(url).map_err(|error| RuntimeError::ControlTransport {
            operation: "connect",
            detail: error.to_string(),
        })?;
        let hello = BridgeControlHello::sign(
            BridgeControlHelloUnsigned {
                bridge_id: bridge_id.to_string(),
                lease_id: lease_id.to_string(),
                bridge_pub: bridge_pub.clone(),
                sent_at_ms: now_ms,
                request_id: request_id.to_string(),
                resume_acked_seq_no,
                chain_id: chain_id.to_string(),
            },
            signing_key,
        )?;
        send_frame(&mut socket, &BridgeControlFrame::Hello(hello))?;

        let welcome = match read_frame(&mut socket)? {
            Some(BridgeControlFrame::Welcome(welcome)) => welcome,
            Some(BridgeControlFrame::Error(error)) => {
                return Err(RuntimeError::ControlProtocol {
                    detail: format!("publisher rejected control hello: {}", error.message),
                });
            }
            Some(other) => {
                return Err(RuntimeError::ControlProtocol {
                    detail: format!("expected welcome frame, got {}", frame_type(&other)),
                });
            }
            None => {
                return Err(RuntimeError::ControlProtocol {
                    detail: "publisher closed control session before welcome".into(),
                });
            }
        };
        welcome.verify_authority(publisher_pub, now_ms, max_skew_ms)?;
        set_socket_timeouts(&mut socket)?;

        Ok(Self {
            bridge_id: bridge_id.to_string(),
            publisher_pub: publisher_pub.clone(),
            session_id: welcome.session_id.clone(),
            last_acked_seq_no: resume_acked_seq_no,
            socket,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn last_acked_seq_no(&self) -> Option<u64> {
        self.last_acked_seq_no
    }

    pub fn receive_command(&mut self, now_ms: u64) -> RuntimeResult<Option<BridgeControlCommand>> {
        loop {
            let Some(frame) = read_frame(&mut self.socket)? else {
                return Ok(None);
            };
            match frame {
                BridgeControlFrame::Command(command) => {
                    if command.bridge_id != self.bridge_id {
                        return Err(RuntimeError::ControlProtocol {
                            detail: format!(
                                "received command for unexpected bridge `{}`",
                                command.bridge_id
                            ),
                        });
                    }
                    verify_command_payload(&command, &self.publisher_pub, now_ms)?;
                    return Ok(Some(command));
                }
                BridgeControlFrame::Keepalive(_) => continue,
                BridgeControlFrame::Error(error) => {
                    return Err(RuntimeError::ControlProtocol {
                        detail: format!("publisher control error: {}", error.message),
                    });
                }
                other => {
                    return Err(RuntimeError::ControlProtocol {
                        detail: format!("unexpected control frame {}", frame_type(&other)),
                    });
                }
            }
        }
    }

    pub fn acknowledge_command(
        &mut self,
        command: &BridgeControlCommand,
        status: BridgeCommandAckStatus,
        acked_at_ms: u64,
    ) -> RuntimeResult<BridgeCommandAck> {
        let ack = BridgeCommandAck {
            session_id: self.session_id.clone(),
            bridge_id: self.bridge_id.clone(),
            command_id: command.command_id.clone(),
            seq_no: command.seq_no,
            acked_at_ms,
            chain_id: command.chain_id.clone(),
            status,
        };
        send_frame(&mut self.socket, &BridgeControlFrame::Ack(ack.clone()))?;
        self.last_acked_seq_no = Some(
            self.last_acked_seq_no
                .map(|current| current.max(command.seq_no))
                .unwrap_or(command.seq_no),
        );
        Ok(ack)
    }

    pub fn send_progress(
        &mut self,
        chain_id: &str,
        progress: BootstrapProgress,
    ) -> RuntimeResult<()> {
        send_frame(
            &mut self.socket,
            &BridgeControlFrame::Progress(BridgeControlProgress {
                session_id: self.session_id.clone(),
                chain_id: chain_id.to_string(),
                progress,
            }),
        )
    }

    pub fn send_keepalive(&mut self, sent_at_ms: u64) -> RuntimeResult<()> {
        send_frame(
            &mut self.socket,
            &BridgeControlFrame::Keepalive(BridgeControlKeepalive {
                session_id: self.session_id.clone(),
                bridge_id: self.bridge_id.clone(),
                sent_at_ms,
                chain_id: format!("control-session-{}", self.session_id),
                last_acked_seq_no: self.last_acked_seq_no,
            }),
        )
    }
}

fn send_frame(
    socket: &mut WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    frame: &BridgeControlFrame,
) -> RuntimeResult<()> {
    let payload = serde_json::to_vec(frame).map_err(|error| RuntimeError::ControlProtocol {
        detail: error.to_string(),
    })?;
    socket
        .send(Message::Binary(payload))
        .map_err(|error| RuntimeError::ControlTransport {
            operation: "send",
            detail: error.to_string(),
        })
}

fn read_frame(
    socket: &mut WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
) -> RuntimeResult<Option<BridgeControlFrame>> {
    match socket.read() {
        Ok(Message::Text(text)) => deserialize_frame(text.as_bytes()).map(Some),
        Ok(Message::Binary(bytes)) => deserialize_frame(&bytes).map(Some),
        Ok(Message::Ping(payload)) => {
            socket.send(Message::Pong(payload)).map_err(|error| {
                RuntimeError::ControlTransport {
                    operation: "pong",
                    detail: error.to_string(),
                }
            })?;
            Ok(None)
        }
        Ok(Message::Pong(_)) => Ok(None),
        Ok(Message::Close(_)) => Ok(None),
        Ok(Message::Frame(_)) => Ok(None),
        Err(tungstenite::Error::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(RuntimeError::ControlTransport {
            operation: "read",
            detail: error.to_string(),
        }),
    }
}

fn deserialize_frame(bytes: &[u8]) -> RuntimeResult<BridgeControlFrame> {
    serde_json::from_slice(bytes).map_err(|error| RuntimeError::ControlProtocol {
        detail: error.to_string(),
    })
}

fn verify_command_payload(
    command: &BridgeControlCommand,
    publisher_pub: &PublicKeyBytes,
    now_ms: u64,
) -> Result<(), ProtocolError> {
    match &command.payload {
        gbn_bridge_protocol::BridgeCommandPayload::SeedAssign(payload) => {
            payload.verify_authority(publisher_pub, now_ms)
        }
        gbn_bridge_protocol::BridgeCommandPayload::PunchStart(payload) => {
            payload.verify_authority(publisher_pub, now_ms)
        }
        gbn_bridge_protocol::BridgeCommandPayload::BatchAssign(payload) => {
            payload.verify_authority(publisher_pub, now_ms)
        }
        gbn_bridge_protocol::BridgeCommandPayload::Revoke(payload) => {
            let _ = now_ms;
            payload.verify_signature(publisher_pub)
        }
        gbn_bridge_protocol::BridgeCommandPayload::CatalogRefresh(payload) => {
            payload.verify_authority(publisher_pub, now_ms)
        }
    }
}

fn set_socket_timeouts(
    socket: &mut WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
) -> RuntimeResult<()> {
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => {
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .map_err(|error| RuntimeError::ControlTransport {
                    operation: "set-read-timeout",
                    detail: error.to_string(),
                })?;
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .map_err(|error| RuntimeError::ControlTransport {
                    operation: "set-write-timeout",
                    detail: error.to_string(),
                })?;
            Ok(())
        }
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

fn frame_type(frame: &BridgeControlFrame) -> &'static str {
    match frame {
        BridgeControlFrame::Hello(_) => "hello",
        BridgeControlFrame::Welcome(_) => "welcome",
        BridgeControlFrame::Command(_) => "command",
        BridgeControlFrame::Ack(_) => "ack",
        BridgeControlFrame::Progress(_) => "progress",
        BridgeControlFrame::Keepalive(_) => "keepalive",
        BridgeControlFrame::Error(_) => "error",
    }
}
