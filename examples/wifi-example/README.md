# wifi-example

Minimal example: connect to WiFi using **esp-wifi-caddy** and
**wifi-caddy-proc** with flash-backed config persistence. Demonstrates the
config UI, config change notifications, and boot-button AP toggle.

This repo uses **path** dependencies to the sibling crates. In your own project,
use crates.io versions instead, for example:

```toml
wifi-caddy        = "0.1.0"
wifi-caddy-proc   = "0.1.0"
esp-wifi-caddy    = "0.1.0"
```

## Features

- **esp32s3** (default with `cargo build-s3`): Build for ESP32-S3.
- **esp32**, **esp32c6**: Alternate targets (use `cargo build-32` / `cargo build-c6`).
- **log** (default): `log` crate logging.
- **defmt**: `defmt` logging (mutually exclusive with `log`).

## Config

- **wifi_ssid** / **wifi_pass**: WiFi credentials (env: `WIFI_SSID`, `WIFI_PASS`).
- **example_string**: Demonstration string field.
- **example_integer**: Demonstration integer field (`u32`).

Config is stored in flash via the `config` partition (requires a partition table
with a partition named `config`).

## Build and run

Cargo aliases are defined in `.cargo/config.toml` for each target:

```bash
# ESP32-S3
cargo clippy-s3       # lint
cargo build-s3        # build
cargo run-s3          # flash and monitor

# ESP32
cargo clippy-32       # lint
cargo build-32        # build
cargo run-32          # flash and monitor
```

Use the **esp** toolchain (`rust-toolchain.toml` sets `channel = "esp"`).
Install via [espup](https://github.com/esp-rs/espup):

```bash
cargo install espup
espup install
```

## How it works

1. `AppConfig::init_wifi(...)` initializes WiFi, mounts
   flash config storage from the `config` partition, loads saved config, and
   starts the HTTP config UI on the AP stack.
2. `config_updated_task` subscribes to config change notifications — when WiFi
   credentials change it sends `StaUp` to the WiFi manager; when example fields
   change it logs the new values.
3. `ip_address_task` polls and reports STA IP address changes.
4. The boot button (GPIO 0) toggles the AP on/off with each press.

## Captive portal

When the AP is up, an HTTP config UI is served at `http://192.168.2.1/`.
With the `captive` feature (default in esp-wifi-caddy), connecting to the AP
opens the config portal automatically via DNS redirect.

## License

MIT OR Apache-2.0
