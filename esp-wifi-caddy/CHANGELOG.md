# Changelog

All notable changes to **esp-wifi-caddy** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- **Breaking:** `WifiCaddyCommand` uses `heapless::String` types (`WifiSsid`, `WifiPass`, `WifiApSsidPrefix`) instead of `alloc::string::String`. ([#4], closes [#3])
- **Breaking:** Removed `config` and `partition-table` feature gates — flash storage and partition lookup are always compiled in. ([#4])
- `FlashConfigStorage` uses type-state (`Unmounted` → `Mounted`) and no longer heap-allocates. ([#4])
- Eliminated all panicking spawns and `Box::leak()` — `wifi_init!` propagates errors via `Result`. ([#4])
- Notification uses `DynamicSender` directly instead of `Option` wrappers. ([#4])

[#3]: https://github.com/liebman/wifi-caddy/issues/3
[#4]: https://github.com/liebman/wifi-caddy/pull/4

## [0.1.0] - 2026-03-29

<!-- next-url -->
[Unreleased]: https://github.com/liebman/wifi-caddy/compare/esp-wifi-caddy-v0.1.0...HEAD
[0.1.0]: https://github.com/liebman/wifi-caddy/compare/esp-wifi-caddy-v0.1.0...esp-wifi-caddy-v0.1.0
