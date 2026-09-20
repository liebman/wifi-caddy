//! Captive-portal DNS server (all queries resolve to AP).

use embassy_time::Duration;
use embassy_time::Timer;

use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_net::Stack;

use super::dhcp::AP_IP_ADDRESS;

/// Embassy task: run a captive-portal DNS server that resolves all queries to [`AP_IP_ADDRESS`].
#[embassy_executor::task]
pub async fn run(stack: Stack<'static>) {
    info!("dns: start DNS task");
    // Sizing: DNS queries are small — a stub resolver's query with an EDNS0 OPT
    // record is ~50-80 bytes, because EDNS0 declares a larger *response* size
    // rather than inflating the query — and our captive replies (question echoed
    // plus one A record) are smaller still, so 512 bytes each way, the classic
    // DNS-over-UDP limit, is ample.
    let buffers = UdpBuffers::<1, 512, 512, 1>::new();
    let mut tx_buf = [0; 512];
    let mut rx_buf = [0; 512];

    let udp = Udp::new(stack, &buffers);

    loop {
        if let Err(_e) = edge_captive::io::run(
            &udp,
            edge_captive::io::DEFAULT_SOCKET,
            &mut tx_buf,
            &mut rx_buf,
            AP_IP_ADDRESS,
            Duration::from_secs(60).into(),
        )
        .await
        {
            error!("dns: error");
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}
