//! Minimal blocking HTTP listener for metrics-only endpoints.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::metrics_prometheus::PROMETHEUS_CONTENT_TYPE;

pub struct MetricsHttpServer {
    listener: TcpListener,
    renderer: Arc<dyn Fn() -> String + Send + Sync>,
}

pub struct MetricsHttpServerHandle {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl MetricsHttpServer {
    pub fn bind<F>(bind_addr: SocketAddr, renderer: F) -> io::Result<Self>
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        Ok(Self {
            listener: TcpListener::bind(bind_addr)?,
            renderer: Arc::new(renderer),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn spawn(self) -> io::Result<MetricsHttpServerHandle> {
        let stop = Arc::new(AtomicBool::new(false));
        let local_addr = self.local_addr()?;
        let stop_for_thread = Arc::clone(&stop);
        let join = thread::spawn(move || self.run_loop(stop_for_thread));
        Ok(MetricsHttpServerHandle {
            local_addr,
            stop,
            join: Some(join),
        })
    }

    fn run_loop(self, stop: Arc<AtomicBool>) -> io::Result<()> {
        self.listener.set_nonblocking(true)?;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }

            match self.listener.accept() {
                Ok((stream, _)) => {
                    let renderer = Arc::clone(&self.renderer);
                    thread::spawn(move || {
                        if let Err(error) = handle_connection(stream, renderer) {
                            eprintln!("metrics connection error: {error}");
                        }
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl MetricsHttpServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn join(mut self) -> io::Result<()> {
        self.shutdown();
        match self.join.take() {
            Some(join) => join
                .join()
                .map_err(|_| io::Error::other("metrics server thread panicked"))?,
            None => Ok(()),
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    renderer: Arc<dyn Fn() -> String + Send + Sync>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request = read_request_line(&mut stream)?;
    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/metrics") => text_response(200, "OK", PROMETHEUS_CONTENT_TYPE, renderer()),
        ("GET", "/healthz") => text_response(200, "OK", "text/plain; charset=utf-8", "ok\n"),
        _ => text_response(404, "Not Found", "text/plain; charset=utf-8", "not found\n"),
    };
    stream.write_all(&response)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpRequestLine {
    method: String,
    path: String,
}

fn read_request_line(stream: &mut TcpStream) -> io::Result<HttpRequestLine> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 256];

    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before request line completed",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "metrics request headers exceed 8 KiB",
            ));
        }
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let request = std::str::from_utf8(&buffer)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request must be utf-8"))?;
    let line = request
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request method"))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request path"))?
        .to_string();

    Ok(HttpRequestLine { method, path })
}

fn text_response(
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: impl AsRef<str>,
) -> Vec<u8> {
    let body = body.as_ref().as_bytes();
    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(body);
    response
}
