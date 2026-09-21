# Changelog

All notable changes to **wifi-caddy** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- **The HTTP portal's static RAM is now 38.5 KiB, down from 80 KiB.** On an
  ESP32-C6 release build: HTTP task pool 41,600 → 14,672 B, TCP buffers
  16,389 → 8,197 B, DHCP 7,152 → 5,264 B, DNS 6,392 → 2,408 B, `StackResources`
  pair 10,480 → 7,664 B. The buffer side comes from `WIFI_CADDY_HTTP_BUF_SIZE`
  4096 → 2048 and `WIFI_CADDY_TCP_BUF_SIZE` 2048 → 1024 (each now matches the
  corresponding upstream library default) plus a new `WIFI_CADDY_HTTP_MAX_HEADERS`
  (default 32; `edge-http` defaults to 64) that is passed to `edge-http`'s `Server`
  explicitly instead of being left at its internal default. Set the variables back
  to 4096/2048/64 for the previous buffer sizes.
- **Accept-queue mode: `serve_loop` now separates accepting from serving.** smoltcp
  has no listen backlog, so in the previous layout (every worker listening for
  itself, `Server::run`) the number of listening sockets equalled the number of
  *idle* workers, and any connection arriving while all workers were busy was
  reset — a single stalled connection was enough to take the portal down on an
  iPhone. The server now runs `HANDLER_TASKS` workers fed from a socket queue
  maintained by a new `WIFI_CADDY_ACCEPTOR_TASKS` (default 4; must be
  >= `HANDLER_TASKS`) via `Server::run_with_socket_queue`, so a burst of
  connections is accepted and waits instead of being reset. Measured on ESP32-C6:
  6,512 B per worker (HTTP buffer + connection future) versus 312 B per acceptor,
  which is why the default is now `HANDLER_TASKS=2` (parallel requests) with four
  acceptors (capacity) — the same connection capacity as four self-listening
  workers for 11.6 KiB less RAM. The TCP buffer pool is sized by `ACCEPTOR_TASKS`
  (it holds every accepted connection), and an application's AP socket count must
  cover `ACCEPTOR_TASKS` plus the DHCP and DNS servers.
- **Fixed: a stalled connection could pin a handler forever and take the portal
  down.** `edge-http` installs no IO timeouts of its own — its documentation asks
  the caller to wrap the acceptor in `edge_nal::WithTimeout` — so one client that
  connected and then stopped talking (a browser's speculative pre-connect, a phone
  that left the AP) held its handler, its TCP socket and its HTTP buffer
  indefinitely. Because smoltcp has no listen backlog, once every handler was
  pinned there was no listening socket left and the portal reset every new
  connection; this was reproduced on an iPhone, where only one handler logged
  requests after the other had hit a connection reset. `serve_loop` now wraps the
  acceptor in `WithTimeout::new(IO_TIMEOUT_MS, ...)`, with a new
  `WIFI_CADDY_IO_TIMEOUT_MS` (default 5000) bounding every read, write and
  shutdown on an accepted socket, so the handler closes the connection and returns
  to `accept()`. That wrapper nests the TCP, timeout and HTTP state machines, so
  applications should add `#![recursion_limit = "256"]` (the example does).
- The DHCP server's UDP buffers are 1024 B receive / 600 B transmit with one
  packet-metadata slot (was 1500/1500 with two), and the captive DNS server uses
  512-byte buffers. A DHCP reply is a 236-byte BOOTP header plus at most
  `Options::buf()`'s 8 options (~320 B in practice); DNS queries with an EDNS0
  OPT record are ~50-80 B, since EDNS0 declares a larger *response* size rather
  than inflating the query.
- `config_storage::MAX_VALUE_SIZE` is 128 bytes (was 256) and the config-group
  JSON response buffer is 384 bytes (was 512). Values longer than
  `MAX_VALUE_SIZE` (after serialization) fail to store with
  `ConfigError::BufferTooSmall`, so raise it — and the storage backend's own
  buffer, which must be at least as large — if a config has long `String` fields.
- The tuning constants are now `pub` (`wifi_caddy::portal::{HANDLER_TASKS,
  TCP_BUF_SIZE, HTTP_BUF_SIZE, KEEPALIVE_TIMEOUT_MS, HTTP_MAX_HEADERS}`) so an
  application can log or assert them next to its own memory accounting.
- Dependencies: the `edge-*` crates now come from crates.io releases
  (`edge-http` 0.8, `edge-nal` 0.7, `edge-nal-embassy` 0.9, `edge-dhcp` 0.8,
  `edge-captive` 0.8) instead of a pinned git revision of `ivmarkov/edge-net`
  (upstream moved to `sysgrok/edge-net`; these are the workspace v0.16.0
  releases, built against the same `embassy-net` 0.9 this crate uses). No API or
  behaviour change — every type and function the portal uses is identical, and a
  clean build no longer fetches from GitHub.
- **Breaking:** `ConfigHandle<C>` is now a type alias for `&'static Mutex<…, C>` — remove `.config()` calls. ([#4], closes [#3])
- **Breaking:** `portal` feature gate removed — HTTP server and config storage are always compiled in. ([#4])
- **Breaking:** Config notification uses `DynamicSender` directly everywhere instead of `Option` wrappers and callback closures. ([#4])
- Config page served as a single compile-time static HTML string instead of streamed segments. ([#4])
- New `ConfigType` and `ConfigServer` supertraits simplify trait bounds across the crate. ([#4])
- Eliminated heap allocations, `Box::leak()`, and panicking spawns — errors are propagated via `Result`. ([#4])

### Removed

- `ConfigUiOptions`, `JsSaveKind`, and unused `serde`/`serde-json-core` dependencies. ([#4])

[#3]: https://github.com/liebman/wifi-caddy/issues/3
[#4]: https://github.com/liebman/wifi-caddy/pull/4

## [0.1.0] - 2026-03-29

<!-- next-url -->
[Unreleased]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-v0.1.0...HEAD
[0.1.0]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-v0.1.0...wifi-caddy-v0.1.0
