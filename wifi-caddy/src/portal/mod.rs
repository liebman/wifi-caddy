//! Embassy + edge-http portal: AP DHCP, optional DNS, HTTP server loop.
//!
//! In-tree module when `config` feature is enabled.

extern crate alloc;

#[cfg(feature = "captive")]
pub(crate) mod captive;
pub mod config_group;
pub mod config_page;
pub mod config_ui;
mod dhcp;
#[cfg(feature = "captive")]
mod dns;
pub mod responses;

use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use edge_http::io::server::{Handler, Server};
use edge_nal::{Close, Readable, TcpAccept, TcpBind, TcpShutdown, TcpSplit, WithTimeout};
use edge_nal_embassy::{Tcp, TcpBuffers};
use embassy_executor::Spawner;
use embassy_net::Stack;
use embedded_io_async::{ErrorType, Read, Write};

include!(concat!(env!("OUT_DIR"), "/server_tuning.rs"));

// ---------------------------------------------------------------------------
// AbortOnClose wrapper: graceful close with timeout, abort as fallback.
//
// When a WiFi client disconnects, the graceful `close()` hangs because
// `tx.flush()` retransmits forever to an unreachable peer.  We try a
// real graceful close first (so in-flight response data is fully delivered
// to clients that are still connected), then fall back to abort after a
// short timeout if the flush stalls (client gone).
// ---------------------------------------------------------------------------

const CLOSE_TIMEOUT_MS: u64 = 5_000;

struct AbortOnCloseAcceptor<A>(A);

impl<A: TcpAccept> TcpAccept for AbortOnCloseAcceptor<A> {
    type Error = A::Error;
    type Socket<'a>
        = AbortOnCloseSocket<A::Socket<'a>>
    where
        Self: 'a;

    async fn accept(&self) -> Result<(SocketAddr, Self::Socket<'_>), Self::Error> {
        let (addr, socket) = self.0.accept().await?;
        Ok((addr, AbortOnCloseSocket(socket)))
    }
}

struct AbortOnCloseSocket<S>(S);

impl<S: ErrorType> ErrorType for AbortOnCloseSocket<S> {
    type Error = S::Error;
}

impl<S: Read> Read for AbortOnCloseSocket<S> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl<S: Write> Write for AbortOnCloseSocket<S> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

impl<S: Readable> Readable for AbortOnCloseSocket<S> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.readable().await
    }
}

impl<S: TcpSplit> TcpSplit for AbortOnCloseSocket<S> {
    type Read<'a>
        = S::Read<'a>
    where
        Self: 'a;
    type Write<'a>
        = S::Write<'a>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        self.0.split()
    }
}

impl<S: TcpShutdown> TcpShutdown for AbortOnCloseSocket<S> {
    async fn close(&mut self, what: Close) -> Result<(), Self::Error> {
        match embassy_time::with_timeout(
            embassy_time::Duration::from_millis(CLOSE_TIMEOUT_MS),
            self.0.close(what),
        )
        .await
        {
            Ok(result) => {
                debug!("tcp: graceful close completed");
                result
            }
            Err(_timeout) => {
                warn!("tcp: graceful close timed out, aborting");
                self.0.abort().await
            }
        }
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        self.0.abort().await
    }
}

/// Run the edge-http server on the given stack with the provided handler.
///
/// Creates TCP buffers, binds to port 80, and runs the server with `HANDLER_TASKS`
/// concurrent connection handlers. Does not return under normal operation.
pub async fn serve_loop<H: Handler>(stack: Stack<'static>, handler: H) {
    debug!("serve_loop: HANDLER_TASKS = {}", HANDLER_TASKS);
    debug!("serve_loop: TCP_BUF_SIZE = {}", TCP_BUF_SIZE);
    debug!("serve_loop: HTTP_BUF_SIZE = {}", HTTP_BUF_SIZE);
    debug!("serve_loop: KEEPALIVE_TIMEOUT_MS = {}", KEEPALIVE_TIMEOUT_MS);
    debug!("serve_loop: REQUEST_TIMEOUT_MS = {}", REQUEST_TIMEOUT_MS);
    let tcp_buffers = TcpBuffers::<{ HANDLER_TASKS }, { TCP_BUF_SIZE }, { TCP_BUF_SIZE }>::new();
    let tcp = Tcp::new(stack, &tcp_buffers);

    let acceptor = match tcp
        .bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80)))
        .await
    {
        Ok(a) => a,
        Err(_e) => {
            error!("http: TCP bind error on port 80");
            return;
        }
    };

    let acceptor = AbortOnCloseAcceptor(acceptor);
    let mut server = Server::<{ HANDLER_TASKS }, { HTTP_BUF_SIZE }>::new();
    let handler = WithTimeout::new(REQUEST_TIMEOUT_MS, handler);
    if let Err(e) = server
        .run(Some(KEEPALIVE_TIMEOUT_MS), acceptor, handler)
        .await
    {
        error!("http: server error: {}", crate::fmt::DisplayFmt(&e));
    }
}

/// Debug server: same as `serve_loop` but single concurrent handler.
#[cfg(feature = "debug-server")]
pub async fn serve_loop_debug<H: Handler>(stack: Stack<'static>, handler: H) {
    let tcp_buffers = TcpBuffers::<1, { TCP_BUF_SIZE }, { TCP_BUF_SIZE }>::new();
    let tcp = Tcp::new(stack, &tcp_buffers);

    let acceptor = match tcp
        .bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80)))
        .await
    {
        Ok(a) => a,
        Err(_e) => {
            error!("http: TCP bind error on port 80 (debug)");
            return;
        }
    };

    let mut server = Server::<1, { HTTP_BUF_SIZE }>::new();
    let handler = WithTimeout::new(REQUEST_TIMEOUT_MS, handler);
    if let Err(_e) = server
        .run(Some(KEEPALIVE_TIMEOUT_MS), acceptor, handler)
        .await
    {
        error!("http: debug server error");
    }
}

/// Spawns DHCP, optional DNS (if feature `captive`), then calls
/// `spawn_workers(spawner, ap_stack)` so the application can spawn its own HTTP server task.
pub fn start<F>(spawner: Spawner, ap_stack: Stack<'static>, spawn_workers: F)
where
    F: FnOnce(Spawner, Stack<'static>),
{
    spawner.spawn(dhcp::run(ap_stack)).unwrap();

    #[cfg(feature = "captive")]
    spawner.spawn(dns::run(ap_stack)).unwrap();

    spawn_workers(spawner, ap_stack);
}
