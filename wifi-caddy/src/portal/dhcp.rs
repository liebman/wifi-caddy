//! AP DHCP server and captive-portal constants.

use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use edge_dhcp::io::DEFAULT_SERVER_PORT;
use edge_dhcp::server::{Server, ServerOptions};
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_net::Stack;

/// AP IP address (gateway and DNS for clients).
pub const AP_IP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 2, 1);

#[cfg(feature = "captive")]
/// Host name used for captive redirect (IP as string).
pub const AP_HOST_NAME: &str = "192.168.2.1";

#[cfg(feature = "captive")]
/// URL to which non-captive requests are redirected.
pub const AP_URL: &str = "http://192.168.2.1/";

const AP_DNS_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 2, 1);
const AP_POOL_START: Ipv4Addr = Ipv4Addr::new(192, 168, 2, 100);
const AP_POOL_END: Ipv4Addr = Ipv4Addr::new(192, 168, 2, 200);

/// Embassy task: run the DHCP server on the AP stack, leasing addresses to connected clients.
#[embassy_executor::task]
pub async fn run(stack: Stack<'static>) {
    info!("dhcp: start DHCP task");

    // Sizing: the receive side has to hold a client request (RFC 2131's minimum
    // client message size is 576 bytes; 1024 leaves room for large option sets),
    // while the transmit side only carries our reply — a 236-byte BOOTP header,
    // the 4-byte magic cookie and at most `Options::buf()`'s 8 options, so about
    // 320 bytes in practice — and 600 gives a 2x margin.
    let buffers = UdpBuffers::<1, 600, 1024, 1>::new();
    let udp = Udp::new(stack, &buffers);

    let mut socket = match udp
        .bind(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
    {
        Ok(s) => s,
        Err(_e) => {
            error!("dhcp: bind error");
            return;
        }
    };

    let mut server: Server<_, 64> = Server::new_with_et(AP_IP_ADDRESS);
    server.range_start = AP_POOL_START;
    server.range_end = AP_POOL_END;

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
    let dns = [AP_DNS_ADDRESS];
    let mut server_options = ServerOptions::new(AP_IP_ADDRESS, Some(&mut gw_buf));
    server_options.dns = &dns;

    // Scratch buffer for one datagram: receives the request and holds the
    // encoded reply (see the buffer-sizing note above).
    let mut buf = [0; 1024];
    if let Err(_e) =
        edge_dhcp::io::server::run(&mut server, &server_options, &mut socket, &mut buf).await
    {
        error!("dhcp: server error");
    }
}
