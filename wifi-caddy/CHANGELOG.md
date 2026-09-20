# Changelog

All notable changes to **wifi-caddy** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

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
