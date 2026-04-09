# Changelog

All notable changes to **wifi-caddy-proc** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- `ConfigChange` enum: fields without `notify = "..."` now fire a catchall `Changed` variant instead of being silently ignored. Removes the `__None` sentinel. ([#4], closes [#3])
- `ConfigServer::init_notify` returns `DynamicSender` directly (not `Option`). ([#4])
- Dead `config_server_present` / `notify_channel` booleans and their unreachable branches removed from `StructAttrs` and codegen. ([#4])
- Inherent `pub fn get()` now delegates to `ConfigGet::get()` instead of duplicating match arms. ([#4])

[#3]: https://github.com/liebman/wifi-caddy/issues/3
[#4]: https://github.com/liebman/wifi-caddy/pull/4

## [0.1.0] - 2026-03-29

<!-- next-url -->
[Unreleased]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-proc-v0.1.0...HEAD
[0.1.0]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-proc-v0.1.0...wifi-caddy-proc-v0.1.0
