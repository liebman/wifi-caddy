# Changelog

All notable changes to **esp-wifi-caddy** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- All public functions (`run_inner`, `run_inner_by_partition`, `__spawn_config_http_workers`, init macros) accept `DynamicSender` directly instead of `Option<DynamicSender>`. ([#4], closes [#3])
- `ConfigHandle` is now a type alias; functions return the mutex ref directly instead of wrapping it. ([#4])

[#3]: https://github.com/liebman/wifi-caddy/issues/3
[#4]: https://github.com/liebman/wifi-caddy/pull/4

## [0.1.0] - 2026-03-29

<!-- next-url -->
[Unreleased]: https://github.com/liebman/wifi-caddy/compare/esp-wifi-caddy-v0.1.0...HEAD
[0.1.0]: https://github.com/liebman/wifi-caddy/compare/esp-wifi-caddy-v0.1.0...esp-wifi-caddy-v0.1.0
