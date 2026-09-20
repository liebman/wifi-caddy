# wifi-caddy

Drop-in WiFi management for ESP32 with [Embassy](https://embassy.dev).
Derive a config struct and get a captive portal with HTML forms, flash
persistence, and dual STA+AP control — all generated at compile time.

## Highlights

- **Derive macro does the heavy lifting** — `#[derive(WifiCaddyConfig)]`
  generates config storage, HTML/JS forms, a JSON HTTP API, and WiFi init
  from a single struct definition.
- **Captive portal** — phones and laptops auto-open the config page when they
  connect to the AP. No manual URL entry needed.
- **Flash-backed persistence** — settings survive power cycles via
  `sequential-storage` on a named partition.
- **Command-driven WiFi** — `StaUp(ssid, pass)`, `APUp(prefix)`, `APDown`.
  Nothing starts until you say so.

## Supported hardware

Every Wi-Fi capable chip supported by
[esp-hal](https://github.com/esp-rs/esp-hal) and
[esp-radio](https://github.com/esp-rs/esp-radio): ESP32, ESP32-C2, ESP32-C3,
ESP32-C5, ESP32-C6, ESP32-C61, ESP32-S2, ESP32-S3 and ESP32-S31. The chip is
selected with a feature on `esp-wifi-caddy` — see
[Chip selection](esp-wifi-caddy/README.md#chip-selection).

## Quick look

Define your config:

```rust
#[derive(Clone, Debug, WifiCaddyConfig)]
#[config_server]
#[config_notify]
#[config_ui(default_group = "Network")]
pub struct AppConfig {
    #[config_store(env_default = "WIFI_SSID", notify = "Wifi")]
    #[config_form(page = "Network", fieldset = "WiFi", help = "Network name")]
    wifi_ssid: String,
    #[config_store(env_default = "WIFI_PASS", notify = "Wifi")]
    #[config_form(page = "Network", fieldset = "WiFi", input_type = "password")]
    wifi_pass: String,
}
```

Initialize everything in one call:

```rust
let (wifi_stacks, wifi_sender, config, config_rx) =
    esp_wifi_caddy::wifi_init!(AppConfig, spawner, peripherals.WIFI, flash, "config")
        .expect("wifi_init");
```

This single macro call initializes WiFi (STA + AP), mounts flash storage,
loads saved config, and starts the HTTP config server with DHCP and captive
DNS on the AP stack.

## Architecture

```text
┌──────────────────────────────────────────────────────────────┐
│  Your Application                                            │
│                                                              │
│  #[derive(WifiCaddyConfig)]     ← wifi-caddy-proc           │
│  struct AppConfig { ... }            generates:              │
│                                      • ConfigStorage impl    │
│                                      • HTML config form      │
│                                      • HTTP config API       │
│                                      • config change channel │
├──────────────────────────────────────────────────────────────┤
│  esp-wifi-caddy                                              │
│                                                              │
│  wifi_init!(AppConfig, ...)  →  WifiStacks + WifiCommandSender│
│        │                              │                      │
│        ├─ STA stack                  ├─ StaUp(ssid, pass)    │
│        ├─ AP stack                   ├─ APUp(prefix)         │
│        └─ WifiRunner loop            └─ APDown               │
│                                                              │
│  Config path (feature "config"):                             │
│  ┌─────────────────────────────────────────────────┐         │
│  │ Flash storage  · HTTP config UI  · DHCP  · DNS  │         │
│  └─────────────────────────────────────────────────┘         │
├──────────────────────────────────────────────────────────────┤
│  wifi-caddy (core)                                           │
│                                                              │
│  Platform-agnostic traits · HTTP portal · form generation    │
└──────────────────────────────────────────────────────────────┘
```

## Crates

| Crate | Description |
|-------|-------------|
| **[wifi-caddy](wifi-caddy/README.md)** | Platform-agnostic core: config storage traits, HTTP portal, DHCP, captive DNS, form generation. |
| **[wifi-caddy-proc](wifi-caddy-proc/README.md)** | Proc macro `#[derive(WifiCaddyConfig)]` — generates storage, HTML forms, HTTP API, and config statics. |
| **[esp-wifi-caddy](esp-wifi-caddy/README.md)** | ESP32 runtime: WiFi STA+AP stacks, flash storage backend, `wifi_init!` macro. **Start here for full docs.** |

## Getting started

1. **Install the ESP Rust toolchain** via [espup](https://github.com/esp-rs/espup).
   This repo is built with Espressif's **1.97.0.0** toolchain (rustc 1.97), which
   is also the MSRV of the esp-* crates. `espup install` installs the latest
   release; `espup update` upgrades an existing install:

   ```bash
   cargo install espup
   espup install    # or: espup update
   ```

   Cargo picks up the `esp` toolchain through a local `rust-toolchain.toml`
   (`channel = "esp"`, gitignored so each checkout can choose); the Espressif
   toolchain ships the `rust-src` component required by the `-Zbuild-std` Xtensa
   builds.

2. **Install espflash** for flashing and monitoring:

   ```bash
   cargo install espflash
   ```

3. **Add dependencies** to your project (see
   [wifi-example/Cargo.toml](examples/wifi-example/Cargo.toml) for a complete
   example):

   ```toml
   [dependencies]
   esp-wifi-caddy    = { version = "0.1.0", features = ["esp32s3"] }
   enumset           = "1.1"
   esp-storage       = "0.10"
   ```

   The chip feature (`esp32s3` above, or any of `esp32`, `esp32c2`, `esp32c3`,
   `esp32c5`, `esp32c6`, `esp32c61`, `esp32s2`, `esp32s3`, `esp32s31`) is
   forwarded to `esp-radio` and the rest of the `esp-*` stack, so `esp-radio`
   needs no entry of its own. See
   [Chip selection](esp-wifi-caddy/README.md#chip-selection).

   `esp-wifi-caddy` re-exports `wifi-caddy`, the `WifiCaddyConfig` derive macro,
   and the crates that macro-generated code references, so `wifi-caddy` and
   `wifi-caddy-proc` need no entries of their own. `enumset` is required because
   the `EnumSetType` derive resolves its own crate by name; `serde_json_core` is
   available as `esp_wifi_caddy::serde_json_core` for direct JSON access.

4. **Define your config struct** with `#[derive(WifiCaddyConfig)]` and the
   field/struct attributes you need. See the
   [proc macro README](wifi-caddy-proc/README.md) for the full attribute
   reference.

5. **Call `wifi_init!`** in your `main` to start WiFi, load config, and launch
   the HTTP portal.

6. **Run the example** to see it all in action:

   ```bash
   cd examples/wifi-example
   cargo run-s3          # ESP32-S3
   # Every working chip has clippy/build/run aliases: -32, -s3, -c2, -c3, -c5,
   # -c6 (see the example README for the target list and the chips still waiting
   # on link fixes).
   ```

For the full integration guide, API reference, and feature flags, see the
[esp-wifi-caddy README](esp-wifi-caddy/README.md).

## Example

The [wifi-example](examples/wifi-example/README.md) demonstrates WiFi
connection, flash-backed config, and config change notifications. Press the
devkit BOOT button (GPIO0 on the ESP32/S2/S3, GPIO9 on the C2/C3/C6/C61, GPIO28
on the C5) to toggle the AP on and off — when the AP is up, the
captive config portal is served at `192.168.2.1` and phones will open it
automatically. It's the best starting point for understanding the system end
to end.

## AI assistance

Some portions of this codebase — including the proc macro, HTTP portal,
and documentation — were developed with assistance from AI
(Claude / Cursor). The author directed architecture decisions, reviewed all
output, and tested on hardware. AI-generated code was iteratively refined for style and
to meet embedded constraints.

## License

MIT OR Apache-2.0
