//! Embassy + edge-http portal: AP DHCP, optional DNS, HTTP server loop.

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
use edge_nal::TcpBind;
use edge_nal::WithTimeout;
use edge_nal_embassy::{Tcp, TcpBuffers};
use embassy_executor::Spawner;
use embassy_net::Stack;
use static_cell::StaticCell;

include!(concat!(env!("OUT_DIR"), "/server_tuning.rs"));

/// Milliseconds to wait before retrying TCP bind on port 80 after a failure.
const TCP_BIND_RETRY_DELAY_MS: u64 = 2000;

/// Run the edge-http server on the given stack with the provided handler.
///
/// Creates TCP buffers, binds to port 80 (retrying with a delay if bind fails),
/// and runs the server with `HANDLER_TASKS` concurrent connection handlers.
/// Does not return.
///
/// Connection keepalive is enforced by edge-http's `Server::run`. Per-request
/// timeouts (if desired) should be handled inside the `Handler` implementation.
pub async fn serve_loop<H: Handler>(stack: Stack<'static>, handler: H) -> ! {
    debug!("serve_loop: HANDLER_TASKS = {}", HANDLER_TASKS);
    debug!("serve_loop: TCP_BUF_SIZE = {}", TCP_BUF_SIZE);
    debug!("serve_loop: HTTP_BUF_SIZE = {}", HTTP_BUF_SIZE);
    debug!("serve_loop: HTTP_MAX_HEADERS = {}", HTTP_MAX_HEADERS);
    debug!(
        "serve_loop: KEEPALIVE_TIMEOUT_MS = {}",
        KEEPALIVE_TIMEOUT_MS
    );
    debug!("serve_loop: IO_TIMEOUT_MS = {}", IO_TIMEOUT_MS);
    static TCP_BUF: StaticCell<TcpBuffers<{ HANDLER_TASKS }, { TCP_BUF_SIZE }, { TCP_BUF_SIZE }>> =
        StaticCell::new();
    let tcp_buffers = TCP_BUF.uninit().write(TcpBuffers::new());
    let tcp = Tcp::new(stack, tcp_buffers);

    let acceptor = loop {
        match tcp
            .bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80)))
            .await
        {
            Ok(a) => break a,
            Err(e) => {
                error!(
                    "http: TCP bind error on port 80: {}",
                    crate::fmt::DisplayFmt(&e)
                );
                embassy_time::Timer::after(embassy_time::Duration::from_millis(
                    TCP_BIND_RETRY_DELAY_MS,
                ))
                .await;
            }
        }
    };

    // Bound the IO of every accepted connection. `edge-http` deliberately installs
    // no IO timeouts of its own, and a handler task holds its TCP socket (one slot
    // of the buffer pool above) and its HTTP buffer for the whole connection: a
    // client that connects and then stalls — a browser's speculative pre-connect,
    // a phone that dropped off the AP — would otherwise pin that handler
    // indefinitely. Since smoltcp has no listen backlog, a pinned handler also
    // means one fewer listening socket, and once all handlers are pinned the
    // portal resets every new connection. With this timeout a stalled connection
    // is dropped and the handler returns to `accept()`.
    let acceptor = WithTimeout::new(IO_TIMEOUT_MS, acceptor);

    let mut server = Server::<{ HANDLER_TASKS }, { HTTP_BUF_SIZE }, { HTTP_MAX_HEADERS }>::new();
    match server
        .run(Some(KEEPALIVE_TIMEOUT_MS), acceptor, handler)
        .await
    {
        Ok(()) => error!("http: server exited unexpectedly"),
        Err(e) => error!("http: server error: {}", crate::fmt::DisplayFmt(&e)),
    }
    loop {
        core::future::pending::<()>().await;
    }
}

/// Debug server: same as `serve_loop` but single concurrent handler.
#[cfg(feature = "debug-server")]
pub async fn serve_loop_debug<H: Handler>(stack: Stack<'static>, handler: H) -> ! {
    static TCP_BUF_DBG: StaticCell<TcpBuffers<1, { TCP_BUF_SIZE }, { TCP_BUF_SIZE }>> =
        StaticCell::new();
    let tcp_buffers = TCP_BUF_DBG.uninit().write(TcpBuffers::new());
    let tcp = Tcp::new(stack, tcp_buffers);

    let acceptor = loop {
        match tcp
            .bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80)))
            .await
        {
            Ok(a) => break a,
            Err(e) => {
                error!(
                    "http: TCP bind error on port 80 (debug): {}",
                    crate::fmt::DisplayFmt(&e)
                );
                embassy_time::Timer::after(embassy_time::Duration::from_millis(
                    TCP_BIND_RETRY_DELAY_MS,
                ))
                .await;
            }
        }
    };

    // Same IO timeout as `serve_loop` (see the note there): the debug server runs
    // a single handler, so a stalled connection would take the server down.
    let acceptor = WithTimeout::new(IO_TIMEOUT_MS, acceptor);

    let mut server = Server::<1, { HTTP_BUF_SIZE }, { HTTP_MAX_HEADERS }>::new();
    match server
        .run(Some(KEEPALIVE_TIMEOUT_MS), acceptor, handler)
        .await
    {
        Ok(()) => error!("http: debug server exited unexpectedly"),
        Err(e) => error!("http: debug server error: {}", crate::fmt::DisplayFmt(&e)),
    }
    loop {
        core::future::pending::<()>().await;
    }
}

/// Spawns DHCP, optional DNS (if feature `captive`), then calls
/// `spawn_workers(spawner, ap_stack)` so the application can spawn its own HTTP server task.
pub fn start<F>(
    spawner: Spawner,
    ap_stack: Stack<'static>,
    spawn_workers: F,
) -> Result<(), crate::Error>
where
    F: FnOnce(Spawner, Stack<'static>) -> Result<(), crate::Error>,
{
    spawner.spawn(dhcp::run(ap_stack).map_err(|_| crate::Error::SpawnDhcp)?);

    #[cfg(feature = "captive")]
    spawner.spawn(dns::run(ap_stack).map_err(|_| crate::Error::SpawnDns)?);

    spawn_workers(spawner, ap_stack)?;
    Ok(())
}
