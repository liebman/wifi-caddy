#!/bin/sh
# Memory footprint report for wifi-example.
#
# Reports, for one chip's release ELF:
#   * section totals (flash vs RAM),
#   * the statics that dominate RAM (embassy task pools, TCP/UDP buffer pools,
#     heaps and embassy-net socket resources),
#   * a per-crate text/rodata/bss attribution.
#
# Usage:
#   ./footprint.sh                    # report for the default chip (c6)
#   ./footprint.sh --chip s3          # report for another chip
#   ./footprint.sh --build            # build first (cargo build-<chip>)
#   ./footprint.sh --portal           # one line: the portal's own RAM (for diffs)
#
# The report goes to stdout, so two runs (before/after a change) can be compared:
#
#   ./footprint.sh --chip c6 > /tmp/before.txt
#   ... change something ...
#   ./footprint.sh --chip c6 --build > /tmp/after.txt
#   diff -u /tmp/before.txt /tmp/after.txt
#
# It reads the ELF with the LLVM binutils shipped with Espressif's toolchain
# (they handle both Xtensa and RISC-V ELF), falling back to `nm`/`readelf` on
# PATH. Sizes are reported in bytes.

set -eu

CHIP=c6
BUILD=0
PORTAL=0

while [ $# -gt 0 ]; do
    case "$1" in
        --chip) shift; CHIP="${1:-}" ;;
        --build) BUILD=1 ;;
        --portal) PORTAL=1 ;;
        -h|--help) sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "footprint.sh: unknown argument '$1'" >&2; exit 2 ;;
    esac
    shift
done

case "$CHIP" in
    32) TARGET=xtensa-esp32-none-elf; CHIPNAME=esp32 ;;
    s2) TARGET=xtensa-esp32s2-none-elf; CHIPNAME=esp32s2 ;;
    s3) TARGET=xtensa-esp32s3-none-elf; CHIPNAME=esp32s3 ;;
    c2|c3) TARGET=riscv32imc-unknown-none-elf; CHIPNAME="esp32$CHIP" ;;
    c5|c6|c61) TARGET=riscv32imac-unknown-none-elf; CHIPNAME="esp32$CHIP" ;;
    s31) TARGET=riscv32imafc-unknown-none-elf; CHIPNAME=esp32s31 ;;
    *) echo "footprint.sh: unsupported chip '$CHIP' (see .cargo/config.toml)" >&2; exit 2 ;;
esac

cd "$(dirname "$0")" || exit 1

if [ "$BUILD" = 1 ]; then
    cargo "build-$CHIP"
fi

ELF="target/$TARGET/release/wifi-example"
if [ ! -f "$ELF" ]; then
    echo "footprint.sh: $ELF not found — run './footprint.sh --chip $CHIP --build'" >&2
    exit 1
fi

# Prefer the toolchain LLVM binutils; they understand every supported target.
LLVM_BIN=""
for d in "$HOME"/.rustup/toolchains/esp/*/*/esp-clang/bin; do
    if [ -x "$d/llvm-nm" ] && [ -x "$d/llvm-readelf" ]; then
        LLVM_BIN="$d"
        break
    fi
done
if [ -n "$LLVM_BIN" ]; then
    NM="$LLVM_BIN/llvm-nm"
    READELF="$LLVM_BIN/llvm-readelf"
elif command -v llvm-nm >/dev/null 2>&1; then
    NM=llvm-nm
    READELF=$(command -v llvm-readelf || true)
else
    NM=nm
    READELF=$(command -v readelf || true)
fi

SECTIONS=$(mktemp)
trap 'rm -f "$SECTIONS"' EXIT INT TERM

# The C5/C6/C61 aliases share one target directory, and the ELF in it is
# whatever was linked last, so report what the ELF itself says as a cross-check.
detect_elf_chip() {
    LC_ALL=C tr -c '[:print:]' '\n' < "$1" 2>/dev/null \
        | grep -oE 'esp32c61|esp32c6|esp32c5|esp32c3|esp32c2|esp32s31|esp32s3|esp32s2|esp32' \
        | sort | uniq -c | sort -rn | head -1 | awk '{print $2}'
}

echo "== wifi-example footprint"
ELF_CHIP=$(detect_elf_chip "$ELF")
echo "chip:      $CHIPNAME"
echo "target:    $TARGET"
echo "elf:       $ELF ($(wc -c < "$ELF" | tr -d ' ') bytes on disk, incl. debug info)"
echo "elf chip:  ${ELF_CHIP:-unknown} (detected from strings in the ELF)"
echo "nm:        $NM"
if [ -n "$ELF_CHIP" ] && [ "$ELF_CHIP" != "$CHIPNAME" ]; then
    echo
    echo "WARNING: this ELF looks like an '$ELF_CHIP' build, but --chip $CHIP was given."
    echo "         One target directory can hold several chips' builds (c5/c6/c61 share"
    echo "         riscv32imac): run './footprint.sh --chip $CHIP --build' first."
fi

if [ "$PORTAL" = 1 ]; then
    # One-line summary of the portal's own static RAM, for quick before/after
    # diffs (and possible use as a CI budget).
    "$NM" --print-size --size-sort --radix=d --demangle "$ELF" 2>/dev/null | awk -v chip="$CHIPNAME" '
        {
            if (NF < 4) next
            size = $2 + 0
            typ = tolower($3)
            if (typ != "b" && typ != "d") next
            if ($0 ~ /_config_http_worker::POOL/) http += size
            else if ($0 ~ /serve_loop.*TCP_BUF/) tcp += size
            else if ($0 ~ /portal::dhcp::run::POOL/) dhcp += size
            else if ($0 ~ /portal::dns::run::POOL/) dns += size
            else if ($0 ~ /esp_wifi_caddy::init.*STATIC_CELL/) sock += size
        }
        END {
            printf "%-8s HTTP task pool %6d B | TCP buffers %6d B | DHCP %6d B | DNS %6d B | esp-wifi-caddy statics %6d B | portal total %6d B\n", \
                chip, http, tcp, dhcp, dns, sock, http + tcp + dhcp + dns + sock
        }
    '
    exit 0
fi

# One "name hexsize" pair per line, from the section header table.
if [ -n "$READELF" ]; then
    "$READELF" -S -W "$ELF" 2>/dev/null \
        | sed -E 's/^ *\[ *[0-9]+\] +//' \
        | awk '$1 ~ /^\./ && NF >= 5 { print $1, $5 }' > "$SECTIONS"
fi

section_size() {
    awk -v s="$1" '$1 == s { print $2; f = 1 } END { if (!f) print "0" }' "$SECTIONS"
}

# readelf prints section sizes in hex.
dec() {
    printf '%d' "0x$1"
}

kib() {
    awk -v b="$1" 'BEGIN { printf "%.1f", b / 1024 }'
}

if [ -s "$SECTIONS" ]; then
    echo
    echo "-- Sections (bytes) --"
    for s in .trap .rwtext .rwtext.wifi .text .flash.appdesc .rodata .rodata.wifi \
             .data .data.wifi .bss .dram2_uninit .stack; do
        hex=$(section_size "$s")
        if [ "$hex" = "0" ]; then
            continue
        fi
        bytes=$(dec "$hex")
        printf '%-16s %9d  %8.1f KiB\n' "$s" "$bytes" "$(kib "$bytes")"
    done

    flash=0
    for s in .trap .rwtext .rwtext.wifi .text .flash.appdesc .rodata .rodata.wifi; do
        flash=$((flash + $(dec "$(section_size "$s")")))
    done
    dram_data=0
    for s in .data .data.wifi; do
        dram_data=$((dram_data + $(dec "$(section_size "$s")")))
    done
    dram_bss=0
    for s in .bss .dram2_uninit; do
        dram_bss=$((dram_bss + $(dec "$(section_size "$s")")))
    done
    stack=$(dec "$(section_size .stack)")

    echo
    printf '%-16s %9d  %8.1f KiB\n' "FLASH total" "$flash" "$(kib "$flash")"
    printf '%-16s %9d  %8.1f KiB\n' "DRAM .data" "$dram_data" "$(kib "$dram_data")"
    printf '%-16s %9d  %8.1f KiB\n' "DRAM .bss" "$dram_bss" "$(kib "$dram_bss")"
    printf '%-16s %9d  %8.1f KiB  (reserved NOBITS, not stored)\n' "stack region" "$stack" "$(kib "$stack")"
else
    echo
    echo "-- Sections: skipped (no readelf found) --"
fi

echo
echo "-- Largest RAM statics (.bss/.data, >= 256 bytes) --"
"$NM" --print-size --size-sort --radix=d --demangle "$ELF" 2>/dev/null | awk '
    {
        if (NF < 4) next
        size = $2 + 0
        typ = tolower($3)
        if (typ != "b" && typ != "d") next
        if (size < 256) next
        name = $4
        for (i = 5; i <= NF; i++) name = name " " $i
        n++
        sizes[n] = size
        names[n] = name
    }
    END {
        for (i = 1; i <= n; i++) {
            best = i
            for (j = i + 1; j <= n; j++) if (sizes[j] > sizes[best]) best = j
            tmp = sizes[i]; sizes[i] = sizes[best]; sizes[best] = tmp
            t = names[i]; names[i] = names[best]; names[best] = t
        }
        total = 0
        for (i = 1; i <= n; i++) {
            total += sizes[i]
            name = names[i]
            if (length(name) > 110) name = substr(name, 1, 107) "..."
            printf "%9d  %8.1f KiB  %s\n", sizes[i], sizes[i] / 1024, name
        }
        printf "%9d  %8.1f KiB  == subtotal of the %d statics above\n", total, total / 1024, n
    }
'

echo
echo "-- Per-crate attribution (text / rodata / bss+data) --"
echo "   Note: constants with no crate name in the symbol table (anonymous"
echo "   .Lanon.* entries, e.g. the ~14 KB config page HTML/CSS/JS) land under"
echo "   '(everything else)', as do crates not listed in this script."
"$NM" --print-size --size-sort --radix=d --demangle "$ELF" 2>/dev/null | awk '
    BEGIN {
        n = split("wifi_caddy esp_wifi_caddy wifi_caddy_proc edge_http edge_dhcp edge_captive \
                   edge_nal_embassy edge_nal edge_raw embassy_net smoltcp sequential_storage \
                   serde_json_core serde_core serde esp_radio esp_wifi_sys esp_hal esp_phy \
                   embassy_executor embassy_sync embedded_io_async heapless enumset static_cell \
                   httparse domain jiff defmt log alloc core",
                  keys, /[ \t]+/)
    }
    {
        if (NF < 4) next
        size = $2 + 0
        typ = tolower($3)
        if (typ != "t" && typ != "r" && typ != "d" && typ != "b") next
        idx = (typ == "t") ? 1 : ((typ == "r") ? 2 : 3)
        # Boundary-safe crate-name match: replace every non-identifier character
        # with a space, then look for " key " in the padded name.
        pad = " " $0 " "
        gsub(/[^A-Za-z0-9_]/, " ", pad)
        pad = " " pad " "
        found = 0
        for (i = 1; i <= n; i++) {
            if (index(pad, " " keys[i] " ") > 0) {
                tot[keys[i], idx] += size
                found = 1
                break
            }
        }
        if (!found) tot["(everything else)", idx] += size
    }
    END {
        m = n + 1
        for (i = 1; i <= n; i++) order[i] = keys[i]
        order[m] = "(everything else)"
        for (i = 1; i <= m; i++) {
            best = i
            for (j = i + 1; j <= m; j++) {
                tj = tot[order[j],1] + tot[order[j],2] + tot[order[j],3]
                tb = tot[order[best],1] + tot[order[best],2] + tot[order[best],3]
                if (tj > tb) best = j
            }
            tmp = order[i]; order[i] = order[best]; order[best] = tmp
        }
        printf "%-22s %9s %9s %9s\n", "crate", "text", "rodata", "bss+data"
        for (i = 1; i <= m; i++) {
            k = order[i]
            t = tot[k,1] + 0; r = tot[k,2] + 0; b = tot[k,3] + 0
            if (t + r + b == 0) continue
            printf "%-22s %9d %9d %9d\n", k, t, r, b
        }
    }
'


