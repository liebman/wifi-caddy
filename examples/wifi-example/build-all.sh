#!/bin/sh
# Build the example for every chip whose alias is enabled in .cargo/config.toml.
#
#   ./build-all.sh                                           # default (`log`) features
#   ./build-all.sh --no-default-features --features defmt    # defmt instead of log
#
# Extra arguments are passed to every cargo invocation, so the chip feature from
# the alias and the logging feature are combined.
#
# The ESP32-S2, ESP32-C61 and ESP32-S31 aliases are commented out because those
# builds do not link yet (see "Chip notes" in README.md), so they are skipped
# here too.

set -x

cd "$(dirname "$0")" || exit 1

cargo build-32 "$@" || exit 1
cargo build-s3 "$@" || exit 1
cargo build-c2 "$@" || exit 1
cargo build-c3 "$@" || exit 1
cargo build-c5 "$@" || exit 1
cargo build-c6 "$@" || exit 1

# Not linkable yet — uncomment along with the aliases once fixed:
# cargo build-s2  "$@" || exit 1
# cargo build-c61 "$@" || exit 1
# cargo build-s31 "$@" || exit 1
