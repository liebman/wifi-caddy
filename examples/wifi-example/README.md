# wifi-example

Minimal example: connect to WiFi using **esp-wifi-caddy** and
**wifi-caddy-proc** with flash-backed config persistence. Demonstrates the
config UI, config change notifications, and boot-button AP toggle.

This repo uses **path** dependencies to the sibling crates. The example depends
only on **esp-wifi-caddy** (plus `enumset`, which the `WifiCaddyConfig` derive
relies on) and on the `esp-hal` / `esp-rtos` / `embassy-*` crates its own
application code uses. The target chip is selected with an `esp-wifi-caddy`
feature, which forwards it to `esp-radio` — so `esp-radio` is not listed as a
dependency here. In your own project, use crates.io versions instead, for example:

```toml
esp-wifi-caddy = { version = "0.1.0", features = ["esp32s3"] }
enumset        = "1.1"
```

`#[derive(WifiCaddyConfig)]`, the config storage traits, the config page and
`wifi_init!` all come from `esp-wifi-caddy`: it re-exports the platform-agnostic
core crate and the crates that the generated code references, so no `wifi-caddy`
or `wifi-caddy-proc` entry is needed. See the
[esp-wifi-caddy README](../../esp-wifi-caddy/README.md) for the full list.

## Features

- **esp32s3** (default with `cargo build-s3`): Build for ESP32-S3.
- **esp32**, **esp32c6**: Alternate targets (use `cargo build-32` / `cargo build-c6`).

Each chip feature is forwarded to `esp-wifi-caddy`, which passes it on to
`esp-radio` and the rest of the `esp-*` stack. `esp-wifi-caddy` supports
`esp32`, `esp32c2`, `esp32c3`, `esp32c5`, `esp32c6`, `esp32c61`, `esp32s2`,
`esp32s3` and `esp32s31`; this example only wires up aliases for the three it
builds.

- **log** (default): `log` crate logging (also enables esp-radio's `log-04`).
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

Use the **esp** toolchain — keep a local `rust-toolchain.toml` with
`channel = "esp"` and the `rust-src` component (required by `-Zbuild-std`) so
cargo picks it up; the file is gitignored, so each checkout needs its own. The
example is built with Espressif's **1.97.0.0** toolchain (rustc 1.97). Install
via [espup](https://github.com/esp-rs/espup):

```bash
cargo install espup
espup install    # or: espup update
```

## How it works

1. `esp_wifi_caddy::wifi_init!(AppConfig, spawner, peripherals.WIFI, flash, "config")`
   initializes WiFi, mounts flash config storage from the `config` partition,
   loads saved config, and starts the HTTP config UI on the AP stack.
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
