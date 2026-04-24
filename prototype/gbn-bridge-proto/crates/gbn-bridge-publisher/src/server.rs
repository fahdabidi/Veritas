use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::config::PublisherServiceConfig;
use crate::http::{AuthorityHttpServer, AuthorityHttpServerHandle};
use crate::service::AuthorityService;
use crate::PublisherAuthority;

#[derive(Debug)]
pub struct AuthorityServer {
    config: PublisherServiceConfig,
    service: Arc<Mutex<AuthorityService>>,
}

pub struct BoundAuthorityServer {
    inner: AuthorityHttpServer,
    local_addr: SocketAddr,
}

pub struct AuthorityServerHandle {
    inner: AuthorityHttpServerHandle,
}

impl AuthorityServer {
    pub fn new(authority: PublisherAuthority, config: PublisherServiceConfig) -> Self {
        let service = Arc::new(Mutex::new(AuthorityService::new(authority, &config)));
        Self { config, service }
    }

    pub fn bind(self) -> io::Result<BoundAuthorityServer> {
        let bind_addr = self
            .config
            .socket_addr()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let inner = AuthorityHttpServer::bind(
            bind_addr,
            Arc::clone(&self.service),
            self.config.request_max_bytes,
        )?;
        let local_addr = inner.local_addr()?;
        Ok(BoundAuthorityServer { inner, local_addr })
    }
}

impl BoundAuthorityServer {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn spawn(self) -> io::Result<AuthorityServerHandle> {
        Ok(AuthorityServerHandle {
            inner: self.inner.spawn()?,
        })
    }

    pub fn serve_forever(self) -> io::Result<()> {
        self.inner.serve_forever()
    }
}

impl AuthorityServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr()
    }

    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    pub fn join(self) -> io::Result<()> {
        self.inner.join()
    }
}
