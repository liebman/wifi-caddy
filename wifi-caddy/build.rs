fn env_or(key: &str, default: &str) -> String {
    println!("cargo:rerun-if-env-changed={key}");
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn validate_usize(key: &str, value: &str) {
    if value.parse::<usize>().is_err() {
        panic!(
            "wifi-caddy build.rs: {key}={value:?} is not a valid usize. \
             Set it to a positive integer or remove it to use the default."
        );
    }
}

fn validate_u32(key: &str, value: &str) {
    if value.parse::<u32>().is_err() {
        panic!(
            "wifi-caddy build.rs: {key}={value:?} is not a valid u32. \
             Set it to a positive integer or remove it to use the default."
        );
    }
}

fn main() {
    // Defaults are chosen for small embedded targets, but with enough margin to
    // survive a phone:
    //
    // * HANDLER_TASKS = 4: every handler costs one HTTP work buffer, one
    //   connection future (~4.5 KiB) and one TCP buffer pair. With smoltcp (and
    //   therefore embassy-net) there is no listen backlog — a listening socket
    //   only exists while a task is parked in `accept()` — so a handler that is
    //   busy serving a request (or stalled on a dead client, see
    //   IO_TIMEOUT_MS) takes a listening socket out of the pool. iPhones open
    //   several connections at once (a speculative pre-connect plus captive
    //   portal probes), so 2 handlers proved too few on real hardware; 4 leaves
    //   room to keep accepting.
    // * HTTP_BUF_SIZE = 2048 (edge-http's own default): the buffer only has to
    //   hold the request head; response bodies are streamed.
    // * TCP_BUF_SIZE = 1024 (edge-nal-embassy's own default), used for both the
    //   receive and the transmit buffer of each connection.
    // * HTTP_MAX_HEADERS = 32 (edge-http defaults to 64): each slot costs 16
    //   bytes inside every connection state.
    // * IO_TIMEOUT_MS = 5000: bounds how long a connection can hold a handler
    //   without any IO progress.
    let handler_tasks = env_or("WIFI_CADDY_HANDLER_TASKS", "4");
    let tcp_buf_size = env_or("WIFI_CADDY_TCP_BUF_SIZE", "1024");
    let http_buf_size = env_or("WIFI_CADDY_HTTP_BUF_SIZE", "2048");
    let keepalive_ms = env_or("WIFI_CADDY_KEEPALIVE_TIMEOUT_MS", "3000");
    let http_max_headers = env_or("WIFI_CADDY_HTTP_MAX_HEADERS", "32");
    let io_timeout_ms = env_or("WIFI_CADDY_IO_TIMEOUT_MS", "5000");

    validate_usize("WIFI_CADDY_HANDLER_TASKS", &handler_tasks);
    validate_usize("WIFI_CADDY_TCP_BUF_SIZE", &tcp_buf_size);
    validate_usize("WIFI_CADDY_HTTP_BUF_SIZE", &http_buf_size);
    validate_u32("WIFI_CADDY_KEEPALIVE_TIMEOUT_MS", &keepalive_ms);
    validate_usize("WIFI_CADDY_HTTP_MAX_HEADERS", &http_max_headers);
    validate_u32("WIFI_CADDY_IO_TIMEOUT_MS", &io_timeout_ms);

    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("server_tuning.rs");

    std::fs::write(
        &out,
        format!(
            "/// Per-connection TCP receive *and* transmit buffer size in bytes.\n\
             ///\n\
             /// Override with env var `WIFI_CADDY_TCP_BUF_SIZE` (default 1024, which is\n\
             /// also `edge-nal-embassy`'s own default).\n\
             pub const TCP_BUF_SIZE: usize = {tcp_buf_size};\n\
             \n\
             /// Per-connection HTTP work buffer size in bytes.\n\
             ///\n\
             /// Only has to hold the request head: response status, headers and body are\n\
             /// streamed, and request bodies are drained in 64-byte chunks.\n\
             /// Override with env var `WIFI_CADDY_HTTP_BUF_SIZE` (default 2048, which is\n\
             /// also `edge-http`'s own default).\n\
             pub const HTTP_BUF_SIZE: usize = {http_buf_size};\n\
             \n\
             /// TCP keep-alive timeout in milliseconds.\n\
             /// Override with env var `WIFI_CADDY_KEEPALIVE_TIMEOUT_MS` (default 3000).\n\
             pub const KEEPALIVE_TIMEOUT_MS: u32 = {keepalive_ms};\n\
             \n\
             /// Number of concurrent HTTP handler tasks inside `Server::run`.\n\
             ///\n\
             /// Each task costs one `HTTP_BUF_SIZE` buffer, one connection future (about\n\
             /// 4.5 KiB at these settings) and one TCP buffer pair. Keep it at 4 unless\n\
             /// you have measured otherwise: smoltcp has no listen backlog, so a\n\
             /// listening socket only exists while a task is parked in `accept()`, and a\n\
             /// phone opens several connections at once (speculative pre-connect plus\n\
             /// captive-portal probes). With `debug-server`, the debug server gets its own\n\
             /// separate task.\n\
             /// Override with env var `WIFI_CADDY_HANDLER_TASKS` (default 4).\n\
             pub const HANDLER_TASKS: usize = {handler_tasks};\n\
             \n\
             /// Maximum number of request headers parsed per connection.\n\
             ///\n\
             /// Each slot is a `httparse::Header` (16 bytes on 32-bit targets) held inside\n\
             /// the connection state, so this is multiplied by `HANDLER_TASKS`.\n\
             /// Override with env var `WIFI_CADDY_HTTP_MAX_HEADERS` (default 32;\n\
             /// `edge-http` defaults to 64).\n\
             pub const HTTP_MAX_HEADERS: usize = {http_max_headers};\n\
             \n\
             /// Idle timeout for reads, writes and shutdowns on an accepted connection,\n\
             /// in milliseconds.\n\
             ///\n\
             /// A handler task holds one TCP socket (plus one HTTP buffer) for the whole\n\
             /// connection, and `edge-http` does not bound its IO by itself. Without this\n\
             /// timeout a client that connects and then stalls — a browser's speculative\n\
             /// pre-connect, a phone that drops off the AP — pins the handler forever.\n\
             /// Once every handler is pinned there is no listening socket left and all\n\
             /// further connections are reset, which is what took the portal down on an\n\
             /// iPhone when this was missing.\n\
             /// Override with env var `WIFI_CADDY_IO_TIMEOUT_MS` (default 5000).\n\
             pub const IO_TIMEOUT_MS: u32 = {io_timeout_ms};\n"
        ),
    )
    .unwrap();
}
