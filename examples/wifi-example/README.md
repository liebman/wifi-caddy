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
   has no other way in); the devkit BOOT button (GPIO0 on the ESP32/S2/S3, GPIO9
   on the C2/C3/C6/C61, GPIO28 on the C5 — the chip's download-boot strapping pin)
   toggles the AP off and on with each press.
5. The **Network** and **Example** tabs save independently: each has its own Save
   button, so the WiFi credentials persist only when you press Save on the
   **Network** tab. A tab with unsaved edits shows a dot until it is saved.

## Captive portal

When the AP is up, an HTTP config UI is served at `http://192.168.2.1/`.
The AP SSID is `wifi-example-` followed by the AP MAC (for example
`wifi-example-3cdc75844da0`); the connection task logs the exact SSID when the AP
is enabled, so check the monitor output for `AP SSID: ...`.
With the `captive` feature (default in esp-wifi-caddy), connecting to the AP
opens the config portal automatically via DNS redirect.

## Memory footprint

`footprint.sh` reports where flash and RAM go in a release build. It reads the
ELF with the LLVM binutils from Espressif's toolchain (falling back to
`nm`/`readelf` on `PATH`), so it works for both the RISC-V and Xtensa chips:

```bash
./footprint.sh --chip c6 --build    # build, then report
./footprint.sh --chip s3            # report the ELF already in target/

# Compare before/after a change:
./footprint.sh --chip c6 > /tmp/before.txt
#  ...edit code, then...
./footprint.sh --chip c6 --build > /tmp/after.txt
diff -u /tmp/before.txt /tmp/after.txt
```

It prints the section totals (flash vs DRAM), the largest RAM statics and a
per-crate text/rodata attribution. Two things to know when reading it:

- **RAM is dominated by static buffers, not code.** On ESP32-C6 (`log` features,
  release) the portal's static RAM is now about 38 KB (down from 80 KB): the HTTP
  server's task pool (14,672 B — two worker tasks fed from a four-slot accept
  queue, i.e. `Server<2, 2048, 32>` with `run_with_socket_queue::<4>`), the TCP
  buffer pool (8,197 B — `TcpBuffers<4, 1024, 1024>`, one pair per accepted
  connection), the DHCP and DNS tasks (5,264 B + 2,408 B, down from 7,152 + 6,392
  with 1500-byte UDP buffers) and the `StackResources` pair (7,664 B, down from
  10,480 with 10 sockets per stack). The original tuning (four self-listening
  workers, 4 KiB HTTP buffers, 2 KiB TCP buffers, 64 header slots, no IO timeout)
  spent 41,600 + 16,389 B on those two pools plus 10,480 B on the socket resources.
- **The C5/C6/C61 aliases share one target directory**, so the ELF in it is
  whatever was linked last. The report prints the chip it detects *inside* the
  ELF and warns when that does not match `--chip`; use `--build` when in doubt.

The portal's concurrency, buffers and socket counts are compile-time tunables read
by build scripts, so set them in the `[env]` section of `.cargo/config.toml`
(which is what this example does — the values below are spelled out there):

| variable | default | effect |
| --- | --- | --- |
| `WIFI_CADDY_HANDLER_TASKS` | 2 | worker tasks = requests served in parallel; each costs a work buffer (~2 KiB) plus a ~4.5 KiB connection future (6,512 B measured) |
| `WIFI_CADDY_ACCEPTOR_TASKS` | 4 | acceptor tasks = how many connections can be accepted at once; each costs ~312 B. Must be >= `WIFI_CADDY_HANDLER_TASKS`; the TCP pool and the AP socket count are sized with it |
| `WIFI_CADDY_HTTP_BUF_SIZE` | 2048 | per-connection HTTP work buffer (only has to hold the request head) |
| `WIFI_CADDY_TCP_BUF_SIZE` | 1024 | per-connection TCP receive *and* transmit buffer size |
| `WIFI_CADDY_HTTP_MAX_HEADERS` | 32 | request headers parsed per connection (16 bytes each, per connection) |
| `WIFI_CADDY_IO_TIMEOUT_MS` | 5000 | idle read/write timeout per connection; without it a stalled client pins a worker forever |
| `WIFI_CADDY_KEEPALIVE_TIMEOUT_MS` | 3000 | idle keepalive timeout |
| `ESP_WIFI_CADDY_AP_SOCKETS` | 8 | sockets on the AP stack (`StackResources<N>`), 352 B + fixed overhead each |
| `ESP_WIFI_CADDY_STA_SOCKETS` | 4 | sockets on the STA stack |

**Why there are two counts.** smoltcp has no listen backlog, so with the naive
layout (each worker listening for itself) the number of listening sockets equals
the number of *idle* workers, and a connection arriving while every worker is busy
is reset — one stalled connection was enough to take the portal down on an iPhone.
`edge-http`'s socket-queue mode separates the two roles: `WIFI_CADDY_ACCEPTOR_TASKS`
acceptors keep feeding a queue of accepted connections to `WIFI_CADDY_HANDLER_TASKS`
workers, so a burst is accepted and waits instead of being reset. Capacity (Q) is
cheap and parallelism (P) is expensive, so the defaults are `2 + 4`: the same
capacity as four self-listening workers for ~11.6 KiB less RAM, with two
connections able to wait in the queue. `ESP_WIFI_CADDY_AP_SOCKETS` has to cover
`WIFI_CADDY_ACCEPTOR_TASKS` plus the DHCP and DNS servers, plus headroom.

Because the connection future nests the TCP, timeout and HTTP state machines, the
application needs a higher type-layout recursion limit — `src/main.rs` starts with
`#![recursion_limit = "256"]` for that reason.

The config page (HTML + CSS + JS, ~14 KiB of `.rodata`) is generated by
`wifi-caddy-proc`. It is an anonymous constant with no crate name in the symbol
table, so it shows up under `(everything else)` in the per-crate table.

## License

MIT OR Apache-2.0
