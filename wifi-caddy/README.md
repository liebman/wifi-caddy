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
| `WIFI_CADDY_HANDLER_TASKS`       | `usize` | `2`     | Worker tasks (requests served in parallel)      |
| `WIFI_CADDY_ACCEPTOR_TASKS`      | `usize` | `4`     | Acceptor tasks (connections accepted at once; must be ≥ `HANDLER_TASKS`) |
| `WIFI_CADDY_TCP_BUF_SIZE`        | `usize` | `1024`  | Per-connection TCP receive *and* transmit buffer (bytes) |
| `WIFI_CADDY_HTTP_BUF_SIZE`       | `usize` | `2048`  | Per-connection HTTP work buffer (bytes)         |
| `WIFI_CADDY_HTTP_MAX_HEADERS`    | `usize` | `32`    | Request headers parsed per connection           |
| `WIFI_CADDY_IO_TIMEOUT_MS`       | `u32`   | `5000`  | Idle read/write timeout per connection (ms)     |
| `WIFI_CADDY_KEEPALIVE_TIMEOUT_MS`| `u32`   | `3000`  | TCP keep-alive timeout (ms)                    |
| `WIFI_CADDY_MAX_VALUE_SIZE`      | `usize` | `128`   | Longest single config value the storage path round-trips (bytes) |

Where the memory goes — capacity is cheap, parallelism is not. `serve_loop` runs
`HANDLER_TASKS` workers (the requests served in parallel) that are fed from a
socket queue built by `ACCEPTOR_TASKS` acceptor tasks. Measured on ESP32-C6:

| layout (workers + acceptors) | capacity | queue depth | HTTP task pool | TCP buffers |
|------------------------------|----------|-------------|----------------|-------------|
| 2 + 4 (default)              | 4        | 2           | 14,672 B       | 8,197 B     |
| 1 + 4                        | 4        | 3           | 8,160 B        | 8,197 B     |
| 2 + 6                        | 6        | 4           | 15,296 B       | 12,295 B    |
| 4 + 4 (self-listening, `Server::run`) | 4 | resets when busy | 26,312 B | 8,197 B |

A worker costs 6,512 B (one `HTTP_BUF_SIZE` buffer plus a ~4.5 KiB connection future
holding the HTTP state machine, `HTTP_MAX_HEADERS` 16-byte header slots and the TCP
half); an acceptor costs only ~312 B. The TCP buffer pool has one pair
(`2 × TCP_BUF_SIZE`) per acceptor, since every accepted connection holds one.

**Why capacity (acceptors) must exceed parallelism (workers):** smoltcp has no
listen backlog, so with the naive layout — every worker listening for itself,
`Server::run` — the number of listening sockets equals the number of *idle*
workers, and a connection that arrives while every worker is busy is reset. That is
what knocked the portal out on an iPhone: one connection stalled, the only other
worker was busy, and every new connection was reset. `Server::run_with_socket_queue`
fixes this by accepting up to `ACCEPTOR_TASKS` connections and letting them wait for
a worker, so bursts (a phone's speculative pre-connect plus captive-portal probes)
queue instead of failing.

`WIFI_CADDY_IO_TIMEOUT_MS` is still required: a worker holds its connection until
the client talks or this timeout fires, and a *full* queue (all acceptors holding
sockets) stops accepting until a worker frees one.

`WIFI_CADDY_HTTP_BUF_SIZE` only has to hold the *request head* — the response
status, headers and body are streamed, and request bodies are drained in 64-byte
chunks — so 2048 (also `edge-http`'s own default) leaves ample room for a browser's
headers plus a config page's `?set=` URL.

Size the acceptor count for the worst-case burst you expect on your network rather
than for CPU: a phone opens a speculative pre-connect plus several captive-portal
probes at once, so `WIFI_CADDY_ACCEPTOR_TASKS=4` is a sensible floor and 6 gives
more headroom. Raising `WIFI_CADDY_HANDLER_TASKS` is the expensive way to fix
"connections get reset" — add acceptors instead.

`WIFI_CADDY_IO_TIMEOUT_MS` is what makes a stalled connection recoverable at all.
`edge-http` installs no IO timeouts of its own (its documentation asks the caller
to wrap the acceptor with `edge_nal::WithTimeout`), so this crate wraps the
acceptor: every read, write and shutdown on an accepted socket is bounded by this
idle timeout, after which the connection is closed and the handler goes back to
serving the queue (or, for a worker, waiting for the next queued connection).
Without it a single stalled client pins a worker forever.

Because the wrapped connection nests the TCP, timeout and HTTP state machines,
deeply-nested layouts can exceed rustc's default type-layout recursion limit.
Applications that use the portal should add:

```rust
#![recursion_limit = "256"]
```

Set them in the shell — for example, more connection capacity with the old buffer
sizes:

```sh
WIFI_CADDY_ACCEPTOR_TASKS=6 WIFI_CADDY_HTTP_BUF_SIZE=4096 cargo build
```

Or persistently in `.cargo/config.toml` (recommended for embedded projects):

```toml
[env]
WIFI_CADDY_HANDLER_TASKS = "2"   # workers (requests served in parallel)
WIFI_CADDY_ACCEPTOR_TASKS = "4"  # acceptors (connection capacity), >= workers
WIFI_CADDY_TCP_BUF_SIZE = "1024"
WIFI_CADDY_HTTP_BUF_SIZE = "2048"
WIFI_CADDY_HTTP_MAX_HEADERS = "32"
WIFI_CADDY_MAX_VALUE_SIZE = "128"   # longest single config value (bytes)
```

The tunables are also exported as constants (`wifi_caddy::portal::HANDLER_TASKS`,
`ACCEPTOR_TASKS`, `TCP_BUF_SIZE`, `HTTP_BUF_SIZE`, `HTTP_MAX_HEADERS`,
`IO_TIMEOUT_MS`, `KEEPALIVE_TIMEOUT_MS`, and `config_storage::MAX_VALUE_SIZE`), so an
application can log them next to its own memory report. For a ready-made report,
`examples/wifi-example/footprint.sh` prints the section totals, the largest RAM
statics and a per-crate split for a release ELF.

### Config value size

`WIFI_CADDY_MAX_VALUE_SIZE` (default 128) is not a buffer a request moves through,
it is the longest single config value the storage path can round-trip. It bounds the
scratch buffers in `ConfigStorage::{get_value, set_value}` and, in `esp-wifi-caddy`,
the flash backend's fetch/store buffers — which are sized from the same constant, so
one setting keeps both sides in step.

Exceeding it is not a soft failure:

- **Storing** fails with `ConfigError::BufferTooSmall("need N bytes")`, and the
  `ConfigStore` derive's `store_to` writes fields in *declaration order* and stops at
  the first error. The fields declared before the long one are already in flash, the
  ones after it are silently skipped, and the portal still reports the save as failed
  (HTTP 500) — a half-applied save. Config-group requests also skip the change
  notification, so e.g. a WiFi credential change persists but is not applied until
  the next boot.
- **Loading** fails with `ConfigError::Backend` once such a value is in flash, so
  raising the limit *down* (or config from an older build that wrote long values)
  makes the config unloadable. For `esp-wifi-caddy` applications `wifi_init!` then
  returns `Err` at boot; erase the config partition to recover.

Raise it as soon as a config has a `String` field that can be long — a REST bearer
token is typically ~190 bytes, a URL plus an entity id can pass 128 between them, and
the limit applies per *value*. The cost is stack: `MAX_VALUE_SIZE` bytes per scratch
buffer, two of them live at once on either path (`set_value` plus the backend's fetch
buffer, `get_value` plus its load buffer).
