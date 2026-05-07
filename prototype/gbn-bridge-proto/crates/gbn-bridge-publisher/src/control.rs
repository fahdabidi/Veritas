use std::collections::BTreeMap;
use std::io;
use std::net::TcpStream;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gbn_bridge_protocol::{BridgeControlError, BridgeControlFrame};
use serde::{Deserialize, Serialize};
use tungstenite::{accept, Error as WebSocketError, Message};

use crate::service::{AuthorityService, ServiceError};

pub const CONTROL_ROUTE_PATH: &str = "/v1/bridge/control";

#[derive(Debug, Clone)]
pub struct ControlSessionState {
    pub session_id: String,
    pub bridge_id: String,
    pub lease_id: String,
    pub last_acked_seq_no: Option<u64>,
    pub connected_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub sender: Sender<BridgeControlFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeAdminCommandReceipt {
    pub command_id: String,
    pub seq_no: u64,
    pub dispatched_at_ms: u64,
}

#[derive(Debug, Default)]
pub struct ControlSessionRegistry {
    next_session_seq: u64,
    sessions_by_bridge: BTreeMap<String, ControlSessionState>,
    bridge_by_session: BTreeMap<String, String>,
}

impl ControlSessionRegistry {
    pub fn replace_session(
        &mut self,
        bridge_id: &str,
        lease_id: &str,
        connected_at_ms: u64,
        last_acked_seq_no: Option<u64>,
        sender: Sender<BridgeControlFrame>,
    ) -> ControlSessionState {
        if let Some(previous) = self.sessions_by_bridge.remove(bridge_id) {
            self.bridge_by_session.remove(&previous.session_id);
        }
        self.next_session_seq = self.next_session_seq.saturating_add(1);
        let session = ControlSessionState {
            session_id: format!("control-{bridge_id}-{:06}", self.next_session_seq),
            bridge_id: bridge_id.to_string(),
            lease_id: lease_id.to_string(),
            last_acked_seq_no,
            connected_at_ms,
            last_seen_at_ms: connected_at_ms,
            sender,
        };
        self.bridge_by_session
            .insert(session.session_id.clone(), bridge_id.to_string());
        self.sessions_by_bridge
            .insert(bridge_id.to_string(), session.clone());
        session
    }

    pub fn bridge_session(&self, bridge_id: &str) -> Option<&ControlSessionState> {
        self.sessions_by_bridge.get(bridge_id)
    }

    pub fn session(&self, session_id: &str) -> Option<&ControlSessionState> {
        let bridge_id = self.bridge_by_session.get(session_id)?;
        self.sessions_by_bridge.get(bridge_id)
    }

    pub fn touch_session(&mut self, session_id: &str, seen_at_ms: u64) -> Option<()> {
        let bridge_id = self.bridge_by_session.get(session_id)?.clone();
        let session = self.sessions_by_bridge.get_mut(&bridge_id)?;
        session.last_seen_at_ms = seen_at_ms;
        Some(())
    }

    pub fn update_acked_seq_no(&mut self, session_id: &str, seq_no: u64) -> Option<()> {
        let bridge_id = self.bridge_by_session.get(session_id)?.clone();
        let session = self.sessions_by_bridge.get_mut(&bridge_id)?;
        session.last_acked_seq_no = Some(
            session
                .last_acked_seq_no
                .map(|current| current.max(seq_no))
                .unwrap_or(seq_no),
        );
        Some(())
    }

    pub fn remove_session(&mut self, session_id: &str) {
        if let Some(bridge_id) = self.bridge_by_session.remove(session_id) {
            self.sessions_by_bridge.remove(&bridge_id);
        }
    }
}

pub fn looks_like_control_upgrade(stream: &TcpStream) -> io::Result<bool> {
    let mut peek = [0_u8; 1024];
    let read = stream.peek(&mut peek)?;
    if read == 0 {
        return Ok(false);
    }
    let preview = String::from_utf8_lossy(&peek[..read]).to_ascii_lowercase();
    Ok(preview.starts_with("get /v1/bridge/control "))
}

pub fn handle_control_connection(
    stream: TcpStream,
    service: &Arc<Mutex<AuthorityService>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut websocket =
        accept(stream).map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
    let (idle_timeout_ms, _heartbeat_interval_ms) = {
        let service = service.lock().expect("authority service mutex poisoned");
        service.control_timeouts()
    };

    let hello = match read_control_frame(&mut websocket)? {
        Some(BridgeControlFrame::Hello(hello)) => hello,
        Some(frame) => {
            send_error_frame(
                &mut websocket,
                BridgeControlError {
                    code: "bad_request".into(),
                    message: format!("expected hello frame, got {}", frame_type(&frame)),
                },
            )?;
            return Ok(());
        }
        None => return Ok(()),
    };

    let (sender, receiver) = mpsc::channel();
    let accepted = {
        let mut service = service.lock().expect("authority service mutex poisoned");
        service.accept_control_hello(hello, sender)
    };
    let (welcome, initial_commands) = match accepted {
        Ok(accepted) => accepted,
        Err(error) => {
            send_error_frame(
                &mut websocket,
                BridgeControlError {
                    code: error.code().into(),
                    message: error.message().into(),
                },
            )?;
            return Ok(());
        }
    };
    let session_id = welcome.session_id.clone();
    websocket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))?;
    websocket
        .get_mut()
        .set_write_timeout(Some(Duration::from_secs(5)))?;
    send_control_frame(&mut websocket, &BridgeControlFrame::Welcome(welcome))?;
    for command in initial_commands {
        send_control_frame(&mut websocket, &BridgeControlFrame::Command(command))?;
    }

    let mut idle_since_ms = now_ms();
    loop {
        loop {
            match receiver.try_recv() {
                Ok(frame) => {
                    send_control_frame(&mut websocket, &frame)?;
                    idle_since_ms = now_ms();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        match read_control_frame(&mut websocket) {
            Ok(Some(BridgeControlFrame::Ack(ack))) => {
                idle_since_ms = now_ms();
                let commands = {
                    let mut service = service.lock().expect("authority service mutex poisoned");
                    service
                        .handle_control_ack(ack)
                        .map_err(service_error_to_io)?
                };
                for command in commands {
                    send_control_frame(&mut websocket, &BridgeControlFrame::Command(command))?;
                }
            }
            Ok(Some(BridgeControlFrame::Progress(progress))) => {
                idle_since_ms = now_ms();
                let commands = {
                    let mut service = service.lock().expect("authority service mutex poisoned");
                    service
                        .handle_control_progress(progress)
                        .map_err(service_error_to_io)?
                };
                for command in commands {
                    send_control_frame(&mut websocket, &BridgeControlFrame::Command(command))?;
                }
            }
            Ok(Some(BridgeControlFrame::Keepalive(keepalive))) => {
                idle_since_ms = now_ms();
                let commands = {
                    let mut service = service.lock().expect("authority service mutex poisoned");
                    service
                        .handle_control_keepalive(keepalive)
                        .map_err(service_error_to_io)?
                };
                for command in commands {
                    send_control_frame(&mut websocket, &BridgeControlFrame::Command(command))?;
                }
            }
            Ok(Some(BridgeControlFrame::Error(_))) => {
                idle_since_ms = now_ms();
            }
            Ok(Some(BridgeControlFrame::Hello(_)))
            | Ok(Some(BridgeControlFrame::Welcome(_)))
            | Ok(Some(BridgeControlFrame::Command(_))) => {
                send_error_frame(
                    &mut websocket,
                    BridgeControlError {
                        code: "bad_request".into(),
                        message: "unexpected control frame after handshake".into(),
                    },
                )?;
                break;
            }
            Ok(None) => {
                if now_ms().saturating_sub(idle_since_ms) > idle_timeout_ms {
                    break;
                }
            }
            Err(error) => {
                let _ = send_error_frame(
                    &mut websocket,
                    BridgeControlError {
                        code: "internal".into(),
                        message: error.to_string(),
                    },
                );
                break;
            }
        }
    }

    let mut service = service.lock().expect("authority service mutex poisoned");
    service.remove_control_session(&session_id);
    Ok(())
}

fn read_control_frame(
    websocket: &mut tungstenite::WebSocket<TcpStream>,
) -> io::Result<Option<BridgeControlFrame>> {
    match websocket.read() {
        Ok(Message::Text(text)) => deserialize_frame(text.as_bytes()).map(Some),
        Ok(Message::Binary(bytes)) => deserialize_frame(&bytes).map(Some),
        Ok(Message::Ping(payload)) => {
            websocket
                .send(Message::Pong(payload))
                .map_err(websocket_error_to_io)?;
            Ok(None)
        }
        Ok(Message::Pong(_)) => Ok(None),
        Ok(Message::Close(_)) => Ok(None),
        Ok(Message::Frame(_)) => Ok(None),
        Err(WebSocketError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(websocket_error_to_io(error)),
    }
}

fn deserialize_frame(bytes: &[u8]) -> io::Result<BridgeControlFrame> {
    serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

fn send_control_frame(
    websocket: &mut tungstenite::WebSocket<TcpStream>,
    frame: &BridgeControlFrame,
) -> io::Result<()> {
    let payload = serde_json::to_vec(frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    websocket
        .send(Message::Binary(payload))
        .map_err(websocket_error_to_io)
}

fn send_error_frame(
    websocket: &mut tungstenite::WebSocket<TcpStream>,
    error: BridgeControlError,
) -> io::Result<()> {
    send_control_frame(websocket, &BridgeControlFrame::Error(error))
}

fn websocket_error_to_io(error: WebSocketError) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error.to_string())
}

fn service_error_to_io(error: ServiceError) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error.message().to_string())
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}
