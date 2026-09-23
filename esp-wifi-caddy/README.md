# esp-wifi-caddy

A `no_std` crate for ESP32 WiFi STA+AP management using Embassy and esp-radio.
Runs a station connection plus an optional access point, with a command channel
to toggle AP on/off. Optional flash-backed config persistence and a captive
HTTP portal for runtime configuration.

## How it works

```text
┌─────────────────────────────────────────────────────────────────┐
│  Your Application                                               │
│                                                                 │
│  #[derive(WifiCaddyConfig)]    ← wifi-caddy-proc                │
│  struct AppConfig { ... }           generates:                  │
│                                     • ConfigStorage impl        │
│                                     • HTML config form          │
│                                     • HTTP config API           │
│                                     • init_wifi                 │
│                                     • config change channel     │
├─────────────────────────────────────────────────────────────────┤
│  esp-wifi-caddy                                                 │
│                                                                 │
│  init(spawner, wifi)  ──→  WifiStacks + WifiCommandSender       │
│        │                        │                               │
│        ├─ STA stack             ├─ send StaUp(ssid, pass)       │
│        ├─ AP stack              ├─ send APUp(prefix)            │
│        └─ WifiRunner loop       └─ send APDown                  │
│                                                                 │
│  Config storage & HTTP portal:                                  │
│  ┌──────────────────────────────────────────────┐               │
│  │ Flash config storage (sequential-storage)    │               │
│  │ HTTP config UI (edge-http on AP stack)        │               │
│  │ DHCP server (edge-dhcp)                      │               │
│  │ DNS captive redirect (feature "captive")     │               │
│  └──────────────────────────────────────────────┘               │
└─────────────────────────────────────────────────────────────────┘
```

## Overview

- **Platform:** ESP32 with `esp-hal`, `esp-radio`, and `embassy-net`.
- **Configuration:** All configuration is via `WifiCaddyCommand`: send `StaUp(ssid, pass)` to
  enable STA with credentials, `APUp(prefix)` to enable the AP (full SSID = prefix + MAC), and
  `APDown` to disable the AP. The caddy starts with empty state until you send commands.
- **Config storage:** Flash-backed config storage traits and captive HTTP portal
  (AP DHCP, HTTP server, config UI). Config structs are derived with
  `#[derive(WifiCaddyConfig)]`, re-exported from this crate.

## Single-dependency setup

This crate is a facade: `#[derive(WifiCaddyConfig)]`, the config traits, the config
page and `wifi_init!` are all reachable through it, and it re-exports the crates
that macro-generated code references. An ESP32 app therefore needs just:

```toml
[dependencies]
esp-wifi-caddy = "0.1.0"
enumset        = "1.1"   # the generated `EnumSet<ConfigChange>` derives `EnumSetType`
```

`enumset` cannot be re-exported because `enumset_derive` names `::enumset`
directly. Everything else — `wifi-caddy`, `wifi-caddy-proc`, `serde`,
`serde-json-core`, `embassy-sync`, `static_cell` — is re-exported here and named
through it by the generated code:

```rust,ignore
use esp_wifi_caddy::{ConfigHandle, WifiCaddyConfig, WifiSsid, WifiPass, WifiApSsidPrefix};
use esp_wifi_caddy::WifiCaddyCommand; // StaUp(ssid, pass) / APUp(prefix) / APDown
```

Rust code that depends on `wifi-caddy` directly (no `esp-wifi-caddy`) keeps the
historical `wifi_caddy::…` paths in generated code; see the
[wifi-caddy-proc README](../wifi-caddy-proc/README.md) for the resolution rules
and the `#[config_crate(...)]` override.

## Boot flow (with config + proc macro)

1. `AppConfig::init_wifi(spawner, wifi, flash, "config")` initializes WiFi, mounts flash config storage from the named partition, loads saved config, and starts the HTTP config server on the AP stack.
2. Your app receives `(WifiStacks, WifiCommandSender, ConfigHandle<AppConfig>, config_rx)` where `ConfigHandle` is a `&'static Mutex` alias.
3. Send `StaUp(ssid, pass)` to connect, `APUp(prefix)` to enable the AP with the config portal, `APDown` to disable it.
4. Use the returned channel receiver (`config_rx`) to react when settings are updated via the portal.

## Quick integration (without config)

1. Call `esp_wifi_caddy::init(spawner, wifi)` to get `(wifi_stacks, wifi_sender)`.
2. Store the `WifiCommandSender` where your tasks can use it.
3. Spawn a task that sends `StaUp(ssid, pass)` when you have credentials (e.g. from
   config or env). Optionally send `APUp(prefix)` to enable the AP.
4. Use `wifi_stacks.sta` and `wifi_stacks.ap` as the `embassy_net::Stack` for your
   network tasks.

## Quick integration (with config + WifiCaddyConfig)

This is the recommended path for most applications.

Your `Cargo.toml` needs these dependencies plus one chip feature (beyond the
`esp-hal` / `esp-rtos` / `embassy-*` crates your own application code uses). The
derive routes the code it generates through this crate, so the rest of the config
stack comes along with it:

```toml
[dependencies]
esp-wifi-caddy    = { version = "0.1.0", features = ["esp32s3"] }
enumset           = "1.1"
esp-storage       = "0.10.0"
```

The chip feature (`esp32s3` above — see [Chip selection](#chip-selection))
forwards the selection to `esp-radio` and the rest of the `esp-*` stack, so
`esp-radio` does not need an entry of its own.

See [wifi-example/Cargo.toml](../examples/wifi-example/Cargo.toml) for a
complete working example.

```rust,ignore
use esp_wifi_caddy::WifiCaddyConfig;

#[derive(Clone, Debug, WifiCaddyConfig)]
#[config_server]
#[config_notify]
pub struct AppConfig {
    #[config_store(env_default = "WIFI_SSID", notify = "Wifi")]
    #[config_form(fieldset = "WiFi", help = "Network name")]
    wifi_ssid: String,
    #[config_store(env_default = "WIFI_PASS", notify = "Wifi")]
    #[config_form(fieldset = "WiFi", input_type = "password", help = "Password")]
    wifi_pass: String,
}
```

Then in your `main`:

```rust,ignore
let (wifi_stacks, wifi_sender, config, config_rx) =
    esp_wifi_caddy::wifi_init!(AppConfig, spawner, wifi, flash, "config")
        .expect("wifi_init");
```

This single call:
1. Initializes WiFi (STA + AP stacks).
2. Mounts flash config storage from the named partition.
3. Loads saved config values.
4. Starts the HTTP config server on the AP stack (with DHCP and optional captive DNS).
5. Initializes the config-update channel (capacity defaults to the number of config pages, or `#[config_notify(cap = N)]`) and returns the receiver as the 4th tuple element.

`config` is a `&'static Mutex<AppConfig>` — pass it directly to your tasks.
Use `config_rx.receive().await` in a task loop to react to config changes.

The config UI supports multiple tabs when you use `page = "Name"` on `#[config_form]`. Each tab
loads its data on first visit (lazy load) and shows a loading overlay until ready. Use
`#[config_ui(default_group = "Network")]` to choose which tab is active on load.

Each tab is an independent `<form>` backed by its own config group, so **Save and Reload act only
on the active tab**: Save persists just that tab's fields (one `GET /config-group/<Page>` request)
and reports "`<Page>` saved". Edits made on a tab you have not saved are kept in the page — they
survive switching tabs — and the tab shows a dot marker as a reminder that it still needs to be
saved on that tab.

### Config page layout and CSS customization

The config page is a single HTML document with one `<style>` block: built-in CSS first, then any
`extra_css` from `#[config_ui(extra_css = "...")]`. Your CSS overrides the built-in rules when
selectors match.

**Page structure:**

```
<body>
  <div class="container">
    <header>
      <h1>   (page_heading)
      <p>    (subtitle)
    </header>
    <div class="nav">
      (nav_left)  ...  (nav_right)
    </div>
    <div class="content">
      <div id="message" class="message"></div>   -- flash messages (save/load)
      <div class="config-tabs">       -- tab bar (only if >1 page)
        <button class="config-tab active">
      </div>
      <div class="config-tab-panel">
        <div class="config-loading-overlay"> …
        <form id="configForm-…">   -- one form per tab; Save posts only this group
          <div class="config-form">
            <fieldset class="config-form-fieldset">
              <legend class="config-form-legend"> …
              <div class="config-form-group config-form-field-…">
                <label class="config-form-label"> …
                <input class="config-form-input …">
                <div class="config-form-help"> …
              </div>
            </fieldset>
          </div>
          <div class="button-group">  -- Reload + Save
        </form>
      </div>
    </div>
  </div>
</body>
```

**CSS classes and elements you can override:**

| Selector | Purpose |
|----------|---------|
| `body` | Page background (default: gradient) |
| `.container` | Main white card |
| `header`, `header h1`, `header p` | Purple header bar |
| `.nav`, `.nav a` | Navigation bar under header |
| `.content` | Form content area |
| `fieldset.config-form-fieldset`, `legend.config-form-legend` | Field groups |
| `.config-form`, `.config-form-group`, `.config-form-label` | Form layout and field wrappers |
| `.config-form-input`, `input[type="text"]`, `input[type="password"]`, `input[type="number"]` | Text inputs |
| `.config-form-help` | Field help text |
| `.button-group` | Save/reload button row |
| `button[type="submit"]` | Save button |
| `button[type="button"]` | Reload button |
| `.config-tabs` | Tab bar container |
| `.config-tab` | Inactive tab |
| `button.config-tab.active` | Active tab (default: blue gradient) |
| `button.config-tab.dirty` | Tab with unsaved edits (dot marker) |
| `.config-tab-panel` | Tab content panel |
| `.config-loading-overlay` | Loading spinner overlay |
| `.message`, `.message.success`, `.message.error` | Flash messages |

Example:

```rust
#[config_ui(
    page_heading = "My App",
    extra_css = "body { background: #1a1a2e; } header { background: #16213e; }"
)]
```

See [wifi-caddy-proc](../wifi-caddy-proc/README.md) for all available
attributes (`#[config_store]`, `#[config_form]`, `#[config_server]`, `#[config_notify]`,
`#[config_ui]`).

## Public API

### Core (always available)

| Item | Description |
|------|-------------|
| `init(spawner, wifi)` | Initialize WiFi STA+AP; returns `(WifiStacks, WifiCommandSender)` |
| `WifiStacks` | Holds `sta: Stack` and `ap: Stack` (embassy-net stacks) |
| `WifiCommandSender` | Channel sender for `WifiCaddyCommand` |
| `WifiCaddyCommand` | `StaUp(ssid, pass)`, `APUp(prefix)`, `APDown` |
| `mk_static!` | Helper macro to create a `&'static T` from a value |

### Config stack (re-exported through this crate)

| Item | Description |
|------|-------------|
| `WifiCaddyConfig` | The `#[derive(WifiCaddyConfig)]` derive macro (`wifi-caddy-proc`) |
| `wifi_caddy` | The platform-agnostic core crate, re-exported as a module |
| `ConfigHandle`, `ConfigStorageParams`, `config_storage`, `ConfigServer`, `ConfigType`, `Error` | Core config types |
| `embassy_executor`, `embassy_net`, `embassy_sync`, `embassy_time`, `embassy_futures`, `static_cell`, `heapless` | Dependency re-exports (doc-hidden where they exist only for generated code) |
| `defmt` / `log` | Re-exported when the matching feature is enabled |

### Generated types (from `#[derive(WifiCaddyConfig)]`)

The `WifiCaddyConfig` derive macro emits several types into your crate's namespace.
These are used directly in application code:

| Type | Description |
|------|-------------|
| `ConfigChange` | `EnumSetType` enum with one variant per `notify = "..."` group (e.g. `Wifi`, `Example`). Use with `changed.contains(ConfigChange::Wifi)` |
| `ConfigUpdateReceiver` | Type alias for `&'static Channel<..., EnumSet<ConfigChange>, N>`. Passed to tasks that react to config changes |
| `ConfigUpdateChannel` | The underlying channel type (usually not referenced directly) |
| `ConfigKey` | Enum mapping field names to FNV-1a hash keys for storage |
| `<ConfigStruct><Page>Config` (e.g. `AppConfigMainConfig`) | Per-page DTO structs for JSON serialization (internal, may be renamed) |

### Config types (`wifi_init!`)

| Item | Description |
|------|-------------|
| `ConfigHandle<C>` | Type alias for `&'static Mutex<CriticalSectionRawMutex, C>`, returned by `wifi_init!` |
| `ConfigError` | Error type from `init_wifi` (e.g. backend, invalid data) |
| `config_storage::ConfigStorage` | Trait to implement an alternative storage backend |
| `config_storage::ConfigValue` | Trait to implement for custom field types in your config struct (serialization + getter) |
| `config_storage::MAX_VALUE_SIZE` | Max bytes per stored value (used by `ConfigStorage` default impls) |

## Memory footprint

Two sets of compile-time knobs decide how much static RAM the WiFi stacks and the
config portal reserve. Both are read by build scripts, so set them in `[env]` in
`.cargo/config.toml` or in the environment:

| Environment variable | Default | Effect |
| --- | --- | --- |
| `ESP_WIFI_CADDY_AP_SOCKETS` | `8` | Sockets on the access-point stack (`StackResources<N>`) |
| `ESP_WIFI_CADDY_STA_SOCKETS` | `4` | Sockets on the station stack |
| `WIFI_CADDY_HANDLER_TASKS` | `2` | HTTP workers (requests served in parallel) |
| `WIFI_CADDY_ACCEPTOR_TASKS` | `4` | HTTP acceptors (connections accepted at once) |
| `WIFI_CADDY_HTTP_BUF_SIZE` | `2048` | Per-connection HTTP work buffer (bytes) |
| `WIFI_CADDY_TCP_BUF_SIZE` | `1024` | Per-connection TCP receive *and* transmit buffer (bytes) |
| `WIFI_CADDY_HTTP_MAX_HEADERS` | `32` | Request headers parsed per connection |
| `WIFI_CADDY_IO_TIMEOUT_MS` | `5000` | Idle read/write timeout per connection (ms) |

Every socket slot costs 352 bytes plus a fixed per-stack overhead, an HTTP worker
costs 6,512 bytes (a work buffer plus a connection future) and an HTTP acceptor
only ~312 bytes, so these values dominate the portal's static RAM: with the
defaults the whole portal is ~38 KiB on ESP32-C6. The AP stack has to cover the
portal's *accepted* connections (`WIFI_CADDY_ACCEPTOR_TASKS`) plus the DHCP server
and the captive DNS server, plus headroom for clients that are connected but not
yet being served; the STA stack only needs the DHCP client plus whatever the
application opens.

The values are readable in code as `esp_wifi_caddy::AP_SOCKET_COUNT` /
`STA_SOCKET_COUNT` and `wifi_caddy::portal::{HANDLER_TASKS, HTTP_BUF_SIZE,
TCP_BUF_SIZE, HTTP_MAX_HEADERS}`. `examples/wifi-example/footprint.sh` reports
where a build's flash and RAM actually went.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `defmt` | no | defmt logging (forwards to `esp-radio`, `embassy-*` and `wifi-caddy`) |
| `log` | no | log crate logging (forwards to `esp-radio/log-04` and `wifi-caddy`; mutually exclusive with `defmt`) |
| `captive` | yes | DNS captive-portal redirect on AP |
| `debug-server` | no | Additional HTTP server on the STA interface (forwards to `wifi-caddy`) |
| `nightly` | no | Enables the `impl_trait_in_assoc_type` nightly feature. **Enable this if `embassy-executor` is built with its `nightly` feature**, so that task and async code compiles correctly. |
| chip feature | no | Selects the target chip — enable **exactly one** of `esp32`, `esp32c2`, `esp32c3`, `esp32c5`, `esp32c6`, `esp32c61`, `esp32s2`, `esp32s3`, `esp32s31` |

**Feature dependencies:**

- `defmt` and `log` are mutually exclusive — enable at most one.
- `captive` forwards to `wifi-caddy`'s `captive` feature (captive DNS on the AP).
- To build without captive DNS, set `default-features = false` on `esp-wifi-caddy` (or disable `captive` explicitly).

### Chip selection

Enable exactly one of the chip features; it forwards the chip to `esp-radio`
(including its Wi-Fi driver) and to `esp-hal`, `esp-rtos`, `esp-storage`,
`esp-alloc` and `esp-bootloader-esp-idf`:

```toml
[dependencies]
esp-wifi-caddy = { version = "0.1.0", features = ["esp32s3"] }
```

Because the chip selection travels with `esp-wifi-caddy`, an `esp-radio`
dependency (and its chip feature) is not needed in your own `Cargo.toml`. The same
goes for `esp-radio`'s `log-04` / `defmt` features, which follow this crate's
`log` / `defmt` features.

`esp-wifi-caddy` also enables `esp-hal`'s and `esp-radio`'s `unstable` features
by itself: the Wi-Fi connection task waits for a STA disconnect with
`WifiController::is_connected()` and `WifiController::subscribe()`, which are
gated behind `esp-radio/unstable`. Applications do not need to enable them.

The list covers every Wi-Fi capable chip `esp-radio` supports — ESP32-S31 is
RISC-V (`riscv32imafc-unknown-none-elf`), not Xtensa. `esp32h2` and `esp32p4` are
deliberately not offered: those parts have no Wi-Fi driver, and this crate always
enables `esp-radio/wifi`, which makes `esp-radio`'s build script abort for them.
`esp32s31` additionally needs a recent `esp-metadata-generated` (>= 0.5.2) in the
dependency graph for `esp-radio`'s Wi-Fi driver to be available.

Every chip feature compiles for the target. A full application is still subject to
each chip's link-time memory budget (ESP32-S2's DRAM is the tightest), and the
`esp-wifi-sys` blobs for ESP32-C61 / ESP32-S31 currently do not link with
`rust-lld`; see the
[example's chip notes](../examples/wifi-example/README.md#chip-notes).

## Prerequisites

1. **ESP Rust toolchain** — install via [espup](https://github.com/esp-rs/espup).
   This crate is built with Espressif's **1.97.0.0** toolchain (rustc 1.97), which
   is also its MSRV; `espup install` installs the latest release and
   `espup update` upgrades an existing install:

   ```bash
   cargo install espup
   espup install    # or: espup update
   ```

   A local `rust-toolchain.toml` (`channel = "esp"`, gitignored so each checkout
   can choose) makes cargo use that toolchain, whose `rust-src` component is
   required by the `-Zbuild-std` builds for Xtensa targets. RISC-V targets work
   with either the `esp` toolchain or `rustup target add` on a regular toolchain.

2. **espflash** — for flashing and monitoring:

   ```bash
   cargo install espflash
   ```

3. **Partition table** — if using flash config persistence, your partition table must include a `config` partition (type `data`, subtype `nvs` or custom). See the ESP-IDF [partition tables documentation](https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-guides/partition-tables.html).

4. **WiFi credentials** (optional): set env vars `WIFI_SSID` and `WIFI_PASS` for compile-time defaults, or configure via the captive portal at runtime.

## Examples

| Example | Description |
|---------|-------------|
| [wifi-example](../examples/wifi-example/README.md) | WiFi + config persistence + config change notifications. Best starting point for understanding the config system. |

Build and run from the example directory:

```bash
cd examples/wifi-example
cargo build-s3        # build for ESP32-S3
cargo run-s3          # flash and monitor
```

The example defines cargo aliases for multi-target builds (e.g. `cargo clippy-s3`, `cargo run-s3`). See its [README](../examples/wifi-example/README.md) for details.

## License

MIT OR Apache-2.0
