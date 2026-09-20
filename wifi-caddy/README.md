# wifi-caddy

Platform-agnostic config storage traits, HTTP config portal, and form generation
for WiFi configuration managers.

This crate provides the core abstractions used by `wifi-caddy-proc` (derive macro)
and platform-specific runtime crates like `esp-wifi-caddy`:

- **Config storage traits** (`ConfigStorage`, `ConfigValue`, `ConfigLoadStore`,
  `ConfigGet`, `ConfigApi`, `ConfigFormGen`) for key-value config persistence.
- **HTTP config portal** (feature `portal`) with DHCP, optional captive-portal DNS,
  and a config UI server built on `edge-http` and `embassy-net`.
- **Helper types** (`ConfigStorageParams`, `ConfigHandle` type alias) used by the derive
  macro and platform init macros.

## Usage

A platform crate (e.g. `esp-wifi-caddy`) provides the storage backend, the WiFi
initialization, and the derive macro, so on ESP32 you normally depend only on it:

```toml
[dependencies]
esp-wifi-caddy = "0.1.0"
enumset        = "1.1"   # required by the generated `EnumSet<ConfigChange>`
```

This crate stays platform-agnostic: for a crate without `esp-wifi-caddy`,
depend on `wifi-caddy` and `wifi-caddy-proc` directly. The derive macro picks the
matching crate paths for whichever of the two you declared.

## Features

| Feature        | Default | Description                                      |
|----------------|---------|--------------------------------------------------|
| `captive`      | no      | Captive-portal DNS redirect                      |
| `debug-server` | no      | Additional HTTP server on the STA interface       |
| `defmt`        | no      | defmt logging support                            |
| `log`          | no      | log crate logging support                        |

## Server Tuning

The HTTP server's buffer sizes, concurrency, and header limit can be overridden
at compile time via environment variables. All are optional and fall back to
defaults chosen for small targets.

| Environment Variable             | Type    | Default | Description                                    |
|----------------------------------|---------|---------|------------------------------------------------|
| `WIFI_CADDY_HANDLER_TASKS`       | `usize` | `4`     | Concurrent HTTP handler tasks                  |
| `WIFI_CADDY_TCP_BUF_SIZE`        | `usize` | `1024`  | Per-connection TCP receive *and* transmit buffer (bytes) |
| `WIFI_CADDY_HTTP_BUF_SIZE`       | `usize` | `2048`  | Per-connection HTTP work buffer (bytes)         |
| `WIFI_CADDY_HTTP_MAX_HEADERS`    | `usize` | `32`    | Request headers parsed per connection           |
| `WIFI_CADDY_IO_TIMEOUT_MS`       | `u32`   | `5000`  | Idle read/write timeout per connection (ms)     |
| `WIFI_CADDY_KEEPALIVE_TIMEOUT_MS`| `u32`   | `3000`  | TCP keep-alive timeout (ms)                    |

Where the memory goes: every handler task costs one `HTTP_BUF_SIZE` buffer, one
connection future (~4.5 KiB — it holds the HTTP request/response state machine
plus `HTTP_MAX_HEADERS` 16-byte header slots) and one TCP buffer pair
(`2 × TCP_BUF_SIZE`). Measured on ESP32-C6 with these defaults, the HTTP task pool
is 26,312 B and the TCP buffer pool 8,197 B; with `WIFI_CADDY_HANDLER_TASKS=2`
they are 13,280 B and 4,099 B.

`WIFI_CADDY_HTTP_BUF_SIZE` only has to hold the *request head* — the response
status, headers and body are streamed, and request bodies are drained in 64-byte
chunks — so 2048 (also `edge-http`'s own default) leaves ample room for a browser's
headers plus a config page's `?set=` URL.

Keep `WIFI_CADDY_HANDLER_TASKS` at 4 unless you have tested otherwise on your own
hardware and clients: smoltcp (and therefore embassy-net) has no listen backlog —
a listening socket only exists while a task is parked in `accept()` — so every
handler that is busy, or stalled on a client that vanished, takes one listening
socket away. A phone opens several connections at once (a speculative pre-connect
plus captive-portal probes), and 2 handlers were measured to be too few for
iPhone captive-portal traffic: handler 0 stalled on a reset connection and, with
the only other handler busy, all further connections were reset and the portal
never came up.

`WIFI_CADDY_IO_TIMEOUT_MS` is what makes a stalled connection recoverable at all.
`edge-http` installs no IO timeouts of its own (its documentation asks the caller
to wrap the acceptor with `edge_nal::WithTimeout`), so this crate now wraps the
acceptor: every read, write and shutdown on an accepted socket is bounded by this
idle timeout, after which the handler closes the connection and returns to
`accept()`. Without it a single stalled client pins a handler forever.

Because the wrapped connection nests the TCP, timeout and HTTP state machines,
deeply-nested layouts can exceed rustc's default type-layout recursion limit.
Applications that use the portal should add:

```rust
#![recursion_limit = "256"]
```

Set them in the shell — for example, back to the old defaults:

```sh
WIFI_CADDY_HANDLER_TASKS=4 WIFI_CADDY_HTTP_BUF_SIZE=4096 cargo build
```

Or persistently in `.cargo/config.toml` (recommended for embedded projects):

```toml
[env]
WIFI_CADDY_HANDLER_TASKS = "4"
WIFI_CADDY_TCP_BUF_SIZE = "2048"
WIFI_CADDY_HTTP_BUF_SIZE = "4096"
WIFI_CADDY_HTTP_MAX_HEADERS = "64"
```

The tunables are also exported as constants (`wifi_caddy::portal::HANDLER_TASKS`,
`TCP_BUF_SIZE`, `HTTP_BUF_SIZE`, `HTTP_MAX_HEADERS`, `KEEPALIVE_TIMEOUT_MS`), so an
application can log them next to its own memory report. For a ready-made report,
`examples/wifi-example/footprint.sh` prints the section totals, the largest RAM
statics and a per-crate split for a release ELF.
