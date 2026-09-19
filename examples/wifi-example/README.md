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

- **Chip features**: one per Wi-Fi capable chip esp-wifi-caddy supports —
  `esp32`, `esp32s2`, `esp32s3`, `esp32c2`, `esp32c3`, `esp32c5`, `esp32c6`,
  `esp32c61`, `esp32s31`. Each is forwarded to `esp-wifi-caddy`, which passes it
  on to `esp-radio` and the rest of the `esp-*` stack. Enable exactly one.
- **Aliases**: every chip feature has a matching `clippy-*` / `build-*` /
  `run-*` alias, except the three that do not link yet (`esp32s2`, `esp32c61`,
  `esp32s31`) — those aliases are commented out (see
  [Chip notes](#chip-notes)).
- **log** (default): `log` crate logging (also enables esp-radio's `log-04`).
- **defmt**: `defmt` logging (mutually exclusive with `log`).

## Config

- **wifi_ssid** / **wifi_pass**: WiFi credentials (env: `WIFI_SSID`, `WIFI_PASS`).
- **example_string**: Demonstration string field.
- **example_integer**: Demonstration integer field (`u32`).

Config is stored in flash via the `config` partition (requires a partition table
with a partition named `config`).

## Build and run

Per-chip cargo config files live in `.cargo/` (matrix-clock style):

```
.cargo/config.toml          # aliases only (+ shared env / build-std)
.cargo/config-32.toml       # [build] target, runner, link flags, chip [env]
.cargo/config-s2.toml       ... one per supported chip
```

Each alias passes its chip's file, e.g. `build-s3` runs
`cargo build --config .cargo/config-s3.toml --release --features esp32s3`, so the
target triple, the espflash runner, the link flags and any chip-specific `[env]`
values (such as the ESP32-S2 data cache) come from that file:

| Chip | Alias | Target | Alias enabled |
|------|-------|--------|---------------|
| ESP32 | `-32` | `xtensa-esp32-none-elf` | yes |
| ESP32-S2 | `-s2` | `xtensa-esp32s2-none-elf` | no — see [Chip notes](#chip-notes) |
| ESP32-S3 | `-s3` | `xtensa-esp32s3-none-elf` | yes |
| ESP32-C2 | `-c2` | `riscv32imc-unknown-none-elf` | yes |
| ESP32-C3 | `-c3` | `riscv32imc-unknown-none-elf` | yes |
| ESP32-C5 | `-c5` | `riscv32imac-unknown-none-elf` | yes |
| ESP32-C6 | `-c6` | `riscv32imac-unknown-none-elf` | yes |
| ESP32-C61 | `-c61` | `riscv32imac-unknown-none-elf` | no — see [Chip notes](#chip-notes) |
| ESP32-S31 | `-s31` | `riscv32imafc-unknown-none-elf` | no — see [Chip notes](#chip-notes) |

### Chip notes

`esp-wifi-caddy` compiles for every chip above (CI checks the library for each
RISC-V one), but this demo does not link for three of them yet, so their
`clippy-*`/`build-*`/`run-*` aliases are **commented out** in
`.cargo/config.toml`, together with their CI jobs in
`.github/workflows/ci.yml`. Their `config-<chip>.toml` files are kept ready to
use:

- **ESP32-S2** — the WiFi blobs plus both heaps exceed its DRAM (~152 KB after
  the caches), leaving less than the 8 KB main stack esp-hal requires.
  `config-s2.toml` has an `ESP_HAL_CONFIG_DATA_CACHE_SIZE = "0KB"` override
  (commented out): it frees 8 KB of cache SRAM as DRAM and makes the build link
  (verified locally), but it disables the data cache — so it stays commented
  until that has been checked on hardware. `main.rs` also keeps both heaps in the
  reclaimed region on S2 instead of using a DRAM heap.
- **ESP32-C61 / ESP32-S31** — upstream `esp-wifi-sys` blobs for these chips
  reference sections that `rust-lld --gc-sections` discards, and with GC
  disabled they reference ROM symbols the linker scripts do not provide, so the
  link fails before our code runs. Both are newer chips (S31 is still marked
  experimental by esp-hal). No linker-flag workaround was found.

The chip *features* (`esp32s2`, `esp32c61`, `esp32s31`) stay in place, so
`cargo check --features <chip> --target <triple>` still works and the aliases only
need uncommenting once the underlying issue is fixed.

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

To build every enabled chip in one go (this is the local equivalent of CI's
build matrix):

```bash
./build-all.sh                                           # log (default)
./build-all.sh --no-default-features --features defmt    # defmt
```

Extra arguments are passed on to each alias, so the chip feature and the logging
feature are combined.

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
4. The AP starts at boot when the config has no STA credentials (a fresh device
   has no other way in); the boot button (GPIO 0) toggles the AP off and on with
   each press.

## Captive portal

When the AP is up, an HTTP config UI is served at `http://192.168.2.1/`.
The AP SSID is `wifi-example-` followed by the AP MAC (for example
`wifi-example-3cdc75844da0`); the connection task logs the exact SSID when the AP
is enabled, so check the monitor output for `AP SSID: ...`.
With the `captive` feature (default in esp-wifi-caddy), connecting to the AP
opens the config portal automatically via DNS redirect.

## License

MIT OR Apache-2.0
