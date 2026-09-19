# Changelog

All notable changes to **esp-wifi-caddy** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Added

- **Facade re-exports:** the crate now re-exports the `WifiCaddyConfig` derive, the
  `wifi-caddy` core crate (as `wifi_caddy`), `serde`, `serde_json_core`,
  `embassy_sync`, `static_cell`, plus `embassy_executor` / `embassy_net` /
  `embassy_time` / `embassy_futures` / `heapless` / `defmt` / `log` conveniences.
  Macro-generated code is routed through this crate, so an ESP32 app no longer needs
  `wifi-caddy`, `wifi-caddy-proc`, `serde`, `serde-json-core`, `embassy-sync` or
  `static_cell` entries — `enumset` stays required because `enumset_derive` names
  `::enumset` directly.
- `wifi_init!` / `mk_static!` expansions now reach `embassy_sync`, `embassy_net`,
  `embassy_executor` and `static_cell` through `$crate::` paths instead of
  injecting those bare crate names into application code.
- **Chip features for every Wi-Fi capable chip `esp-radio` supports:** `esp32`,
  `esp32c2`, `esp32c3`, `esp32c5`, `esp32c6`, `esp32c61`, `esp32s2`, `esp32s3`
  and `esp32s31`. Each one forwards the chip feature to `esp-radio` — and to
  `esp-hal`, `esp-rtos`, `esp-storage`, `esp-alloc` and
  `esp-bootloader-esp-idf` — so an application does not need an `esp-radio`
  dependency of its own to select a chip.
- `log` now forwards to `esp-radio/log-04`, matching the existing
  `esp-radio/defmt` forwarding of the `defmt` feature.
- The connection task logs the generated AP SSID when the AP is enabled
  (`APUp`) and the SSID it connects to (`debug`), so the AP name — which is the
  configured prefix plus the AP MAC — is visible in the log.

### Changed

- **The target chip is selected with an `esp-wifi-caddy` chip feature** instead of
  enabling the chip feature on `esp-radio` (and the rest of the `esp-*` stack) in
  the application's own `Cargo.toml`. Nothing changed for existing config users
  beyond the ability to drop their `esp-radio` dependency.
- **MSRV is now Rust 1.97** (was 1.93), matching Espressif's `esp` toolchain
  1.97.0.0 that this crate — and `esp-radio` 1.0.0-beta.1 — are built with.
  `rust-toolchain.toml` selects that toolchain and its `rust-src` component.
- **Breaking:** `WifiCaddyCommand` uses `heapless::String` types (`WifiSsid`, `WifiPass`, `WifiApSsidPrefix`) instead of `alloc::string::String`. ([#4], closes [#3])
- **Breaking:** Removed `config` and `partition-table` feature gates — flash storage and partition lookup are always compiled in. ([#4])
- `FlashConfigStorage` uses type-state (`Unmounted` → `Mounted`) and no longer heap-allocates. ([#4])
- Eliminated all panicking spawns and `Box::leak()` — `wifi_init!` propagates errors via `Result`. ([#4])
- Notification uses `DynamicSender` directly instead of `Option` wrappers. ([#4])

### Fixed

- **The Wi-Fi connection task no longer busy-loops while the station is not
  connected.** `WifiController::wait_for_disconnect_async` resolves immediately
  with `WifiError::NotConnected` in that state, so the task's `select3` completed
  on every poll: it flooded the log with
  `wifi: connection task: StaDisconnected - reconnect in 5s`, never reached the
  reconnect timer that is supposed to wait 5 s, and never yielded to the executor
  — starving the config portal, DNS, DHCP and application tasks. Disconnects are
  now awaited only while a STA connection is expected, using
  `WifiController::subscribe()` with the subscription created *before* the first
  station-state read, so a disconnect can be neither missed nor busy-polled.
- `esp-wifi-caddy` now enables `esp-radio`'s `unstable` feature (as it already
  does `esp-hal`'s) for `WifiController::is_connected()` and
  `WifiController::subscribe()`, which the connection task uses. Applications
  need no change.

[#3]: https://github.com/liebman/wifi-caddy/issues/3
[#4]: https://github.com/liebman/wifi-caddy/pull/4

## [0.1.0] - 2026-03-29

<!-- next-url -->
[Unreleased]: https://github.com/liebman/wifi-caddy/compare/esp-wifi-caddy-v0.1.0...HEAD
[0.1.0]: https://github.com/liebman/wifi-caddy/compare/esp-wifi-caddy-v0.1.0...esp-wifi-caddy-v0.1.0
