//! Build script: makes the socket count of each network stack configurable.
//!
//! `StackResources<N>` is allocated once for the station stack and once for the
//! access-point stack, and every socket slot costs roughly 0.5 KiB, so the count
//! is a real memory lever on small targets. The AP stack has to cover the config
//! portal's concurrent connections (see `wifi-caddy`'s `WIFI_CADDY_HANDLER_TASKS`)
//! plus the DHCP server and the captive DNS server; the STA stack only needs the
//! DHCP client plus whatever the application opens.

fn env_or_usize(key: &str, default: usize) -> usize {
    println!("cargo:rerun-if-env-changed={key}");
    match std::env::var(key) {
        Ok(value) => value.parse::<usize>().unwrap_or_else(|_| {
            panic!(
                "esp-wifi-caddy build.rs: {key}={value:?} is not a valid usize. \
                 Set it to a positive integer or remove it to use the default ({default})."
            )
        }),
        Err(_) => default,
    }
}

fn main() {
    let ap_sockets = env_or_usize("ESP_WIFI_CADDY_AP_SOCKETS", 8);
    let sta_sockets = env_or_usize("ESP_WIFI_CADDY_STA_SOCKETS", 4);

    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("stack_sockets.rs");

    std::fs::write(
        &out,
        format!(
            "/// Number of sockets on the access-point stack.\n\
             ///\n\
             /// Must cover the config portal's concurrent connections\n\
             /// (`wifi-caddy`'s `WIFI_CADDY_HANDLER_TASKS`), the DHCP server and the\n\
             /// captive-portal DNS server, plus headroom for clients that have\n\
             /// connected but are not being served yet.\n\
             /// Override with env var `ESP_WIFI_CADDY_AP_SOCKETS` (default 8;\n\
             /// was 10 before this was configurable).\n\
             pub const AP_SOCKET_COUNT: usize = {ap_sockets};\n\
             \n\
             /// Number of sockets on the station stack: the DHCP client plus whatever\n\
             /// sockets the application opens.\n\
             /// Override with env var `ESP_WIFI_CADDY_STA_SOCKETS` (default 4;\n\
             /// was 10 before this was configurable).\n\
             pub const STA_SOCKET_COUNT: usize = {sta_sockets};\n"
        ),
    )
    .unwrap();
}
