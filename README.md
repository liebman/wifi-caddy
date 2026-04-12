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

ESP32, ESP32-S3, ESP32-C6 — any target supported by
[esp-hal](https://github.com/esp-rs/esp-hal) and
[esp-radio](https://github.com/esp-rs/esp-radio).

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

1. **Install the ESP Rust toolchain** via [espup](https://github.com/esp-rs/espup):

   ```bash
   cargo install espup
   espup install
   ```

2. **Install espflash** for flashing and monitoring:

   ```bash
   cargo install espflash
   ```

3. **Add dependencies** to your project (see
   [wifi-example/Cargo.toml](examples/wifi-example/Cargo.toml) for a complete
   example):

   ```toml
   [dependencies]
   wifi-caddy        = "0.1.0"
   wifi-caddy-proc   = "0.1.0"
   esp-wifi-caddy    = "0.1.0"
   serde             = { version = "1", default-features = false, features = ["derive", "alloc"] }
   serde-json-core   = "0.6"
   esp-storage       = "0.8"
   ```

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
   cargo run-32          # ESP32
   cargo run-c6          # ESP32-C6
   ```

For the full integration guide, API reference, and feature flags, see the
[esp-wifi-caddy README](esp-wifi-caddy/README.md).

## Example

The [wifi-example](examples/wifi-example/README.md) demonstrates WiFi
connection, flash-backed config, and config change notifications. Press the
boot button (GPIO 0) to toggle the AP on and off — when the AP is up, the
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
