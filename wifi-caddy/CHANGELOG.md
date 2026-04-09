# Changelog

All notable changes to **wifi-caddy** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- **Breaking:** `ConfigHandle<C>` is now a type alias (`&'static Mutex<CriticalSectionRawMutex, C>`) instead of a newtype struct. Remove `.config()` calls — the value is already the mutex ref. ([#4], closes [#3])
- `ConfigGroupResult` now borrows (`Json(&'a str)`, `Err(u16, &'a str)`) instead of owning `String`. Zero heap allocation on the config-group GET path. ([#4])
- `ConfigServer::init_notify` returns `DynamicSender` directly instead of `Option<DynamicSender>`. ([#4])
- `handle_config_group` and `run_http_config_loop` / `run_http_debug_loop` take `DynamicSender` directly (no `Option` wrapper). ([#4])

### Removed

- `serde` and `serde-json-core` dependencies — they were unused by wifi-caddy's own code (proc-macro-generated code expands in the user's crate). ([#4])
- `send_text_string` helper (was a trivial wrapper around `send_text`). ([#4])
- Redundant `#[allow(async_fn_in_trait)]` on individual traits (crate-level allow suffices). ([#4])

[#3]: https://github.com/liebman/wifi-caddy/issues/3
[#4]: https://github.com/liebman/wifi-caddy/pull/4

## [0.1.0] - 2026-03-29

<!-- next-url -->
[Unreleased]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-v0.1.0...HEAD
[0.1.0]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-v0.1.0...wifi-caddy-v0.1.0
