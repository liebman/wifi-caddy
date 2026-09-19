# Changelog

All notable changes to **wifi-caddy-proc** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- **Breaking:** Config page (HTML + CSS + JS) is now generated as a single static string at compile time, replacing the per-group segment arrays. ([#4], closes [#3])
- **Breaking:** `#[config_server]` and `#[config_notify]` are no longer required — `ConfigServer` and the update channel are always generated with sensible defaults. ([#4])
- `ConfigChange` fires a catchall `Changed` variant for fields without explicit `notify = "..."` instead of silently ignoring them. ([#4])
- Improved error diagnostics: parse errors surface as `compile_error!`, hash collision messages include field names, `fnv1a_hash` has compile-time golden-value assertions. ([#4])
- Edition bumped to 2024. ([#4])

### Added

- Crate-path resolution for generated code: the derive reads the deriving crate's
  manifest (`proc-macro-crate`) and emits `wifi_caddy::…` paths for a direct
  `wifi-caddy` dependency, or facade-rooted paths (`::esp_wifi_caddy::wifi_caddy::…`)
  for an `esp-wifi-caddy` dependency, so a config user no longer needs
  `wifi-caddy`, `serde`, `serde-json-core`, `embassy-sync` or `static_cell` entries.
  Renamed dependencies are resolved by name; if neither crate is found the historical
  `wifi_caddy::…` paths are emitted.
- `#[config_crate(...)]` struct attribute to override the detected crate root
  (`core`, a crate name, or an arbitrary path such as `"crate"`).
- Field attributes `prim_type` and `save_as` for custom type support (type aliases, newtypes). ([#4])
- Compile-time UI tests via `trybuild`. ([#4])

### Removed

- `ConfigUiOptions` generation and dead codegen branches. ([#4])

[#3]: https://github.com/liebman/wifi-caddy/issues/3
[#4]: https://github.com/liebman/wifi-caddy/pull/4

## [0.1.0] - 2026-03-29

<!-- next-url -->
[Unreleased]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-proc-v0.1.0...HEAD
[0.1.0]: https://github.com/liebman/wifi-caddy/compare/wifi-caddy-proc-v0.1.0...wifi-caddy-proc-v0.1.0
