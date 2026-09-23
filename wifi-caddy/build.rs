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
    // * HANDLER_TASKS = 2: the number of *workers*, i.e. requests served in
    //   parallel. Each worker costs one HTTP work buffer plus a ~4.5 KiB
    //   connection future (6,512 B measured on ESP32-C6). Parallelism is
    //   deliberately kept low: the acceptor queue below absorbs connection bursts,
    //   so extra workers buy throughput for simultaneous requests, not capacity.
    // * ACCEPTOR_TASKS = 4: the number of *listeners*, i.e. how many connections
    //   can be accepted (and queued for a worker) at once. smoltcp has no listen
    //   backlog, so without these extra listeners any connection arriving while
    //   every worker is busy would be reset — which is what broke the portal on an
    //   iPhone. Each acceptor costs only ~312 B, so capacity is cheap and
    //   parallelism is expensive: measured portal RAM on ESP32-C6 is 38.5 KiB at
    //   2+4 versus 50.1 KiB for four self-listening workers with the same capacity.
    // * HTTP_BUF_SIZE = 2048 (edge-http's own default): the buffer only has to
    //   hold the request head; response bodies are streamed.
    // * TCP_BUF_SIZE = 1024 (edge-nal-embassy's own default), used for both the
    //   receive and the transmit buffer of each connection.
    // * HTTP_MAX_HEADERS = 32 (edge-http defaults to 64): each slot costs 16
    //   bytes inside every connection state.
    // * IO_TIMEOUT_MS = 5000: bounds how long a connection can hold a worker
    //   without any IO progress.
    // * MAX_VALUE_SIZE = 128: the longest single config value (in serialized
    //   bytes) that the storage path can round-trip. It bounds the per-call
    //   scratch buffers in `ConfigStorage::{get_value, set_value}`, which live on
    //   the stack of whatever task stores the config — for `esp-wifi-caddy` that
    //   is the config HTTP worker (plus the flash backend's own fetch/store
    //   buffers, which are sized from the same constant) — so the default stays
    //   small. A config with longer `String` fields must raise it: a longer value
    //   fails to store with `ConfigError::BufferTooSmall` and, once in flash,
    //   fails to load with `ConfigError::Backend`.
    let handler_tasks = env_or("WIFI_CADDY_HANDLER_TASKS", "2");
    let tcp_buf_size = env_or("WIFI_CADDY_TCP_BUF_SIZE", "1024");
    let http_buf_size = env_or("WIFI_CADDY_HTTP_BUF_SIZE", "2048");
    let keepalive_ms = env_or("WIFI_CADDY_KEEPALIVE_TIMEOUT_MS", "3000");
    let http_max_headers = env_or("WIFI_CADDY_HTTP_MAX_HEADERS", "32");
    let io_timeout_ms = env_or("WIFI_CADDY_IO_TIMEOUT_MS", "5000");
    let acceptor_tasks = env_or("WIFI_CADDY_ACCEPTOR_TASKS", "4");
    let max_value_size = env_or("WIFI_CADDY_MAX_VALUE_SIZE", "128");

    validate_usize("WIFI_CADDY_MAX_VALUE_SIZE", &max_value_size);
    validate_usize("WIFI_CADDY_HANDLER_TASKS", &handler_tasks);
    validate_usize("WIFI_CADDY_TCP_BUF_SIZE", &tcp_buf_size);
    validate_usize("WIFI_CADDY_HTTP_BUF_SIZE", &http_buf_size);
    validate_u32("WIFI_CADDY_KEEPALIVE_TIMEOUT_MS", &keepalive_ms);
    validate_usize("WIFI_CADDY_HTTP_MAX_HEADERS", &http_max_headers);
    validate_u32("WIFI_CADDY_IO_TIMEOUT_MS", &io_timeout_ms);
    validate_usize("WIFI_CADDY_ACCEPTOR_TASKS", &acceptor_tasks);

    // The socket-queue layout keeps exactly `ACCEPTOR_TASKS` sockets alive at any
    // time (in the stack, waiting in the queue, or being served), so a queue with
    // fewer acceptors than workers could never keep every worker fed, and the TCP
    // buffer pool (sized with `ACCEPTOR_TASKS`) must hold them all.
    let handler_count: usize = handler_tasks.parse().unwrap();
    let acceptor_count: usize = acceptor_tasks.parse().unwrap();
    assert!(
        acceptor_count >= handler_count,
        "wifi-caddy build.rs: WIFI_CADDY_ACCEPTOR_TASKS ({acceptor_count}) must be >= \
         WIFI_CADDY_HANDLER_TASKS ({handler_count}) — the acceptor queue is what keeps \
         connections from being reset while every worker is busy."
    );

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
             pub const IO_TIMEOUT_MS: u32 = {io_timeout_ms};\n\
             \n\
             /// Number of acceptor tasks in the HTTP server's socket queue.\n\
             ///\n\
             /// `serve_loop` runs `HANDLER_TASKS` workers fed from a queue of accepted\n\
             /// connections maintained by this many acceptor tasks. Because smoltcp has no\n\
             /// listen backlog, the alternative (each worker listening itself, i.e.\n\
             /// `Server::run`) resets any connection that arrives while every worker is\n\
             /// busy; with the queue such a connection is accepted and waits instead. The\n\
             /// number of sockets alive at any time is exactly this value (in the stack,\n\
             /// queued for a worker, or being served), so it also sizes the TCP buffer\n\
             /// pool and must be >= `HANDLER_TASKS`.\n\
             /// Override with env var `WIFI_CADDY_ACCEPTOR_TASKS` (default 4).\n\
             pub const ACCEPTOR_TASKS: usize = {acceptor_tasks};\n"
        ),
    )
    .unwrap();

    // `MAX_VALUE_SIZE` belongs to `config_storage` rather than `portal`, so it gets
    // its own generated file (included by `src/config_storage.rs`).
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("config_limits.rs");

    let body = concat!(
        "/// Maximum serialized size of a single config value, in bytes.\n",
        "///\n",
        "/// Override with env var `WIFI_CADDY_MAX_VALUE_SIZE` (default 128); see the\n",
        "/// `config_storage` module documentation for what it bounds.\n",
        "pub const MAX_VALUE_SIZE: usize = ",
    );
    std::fs::write(&out, format!("{body}{max_value_size};\n")).unwrap();
}
