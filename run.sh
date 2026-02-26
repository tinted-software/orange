#!/bin/bash
# Build and run the XNU bootloader on QEMU with UEFI firmware.
set -euo pipefail
cd "$(dirname "$0")"

PROFILE="debug"
CARGO_FLAGS=""
if [[ "${1:-}" == "--release" ]]; then
    PROFILE="release"
    CARGO_FLAGS="--release"
    shift
fi

echo "==> Building bootloader..."
cargo build --target x86_64-unknown-uefi -p xnuboot $CARGO_FLAGS

EFI_BINARY="target/x86_64-unknown-uefi/${PROFILE}/xnuboot.efi"

ESP_DIR="target/esp"
mkdir -p "${ESP_DIR}/EFI/BOOT"
cp "$EFI_BINARY" "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"

# Create kernel.kc from kernel.development

if [[ -f "kernel.development" ]]; then
    cargo run -p mkstatickc -- kernel.development "${ESP_DIR}/kernel.kc"
else
    echo "ERROR: kernel.development not found in parent directory"
    echo "  Run: cargo run --release -p mkstatickc -- ../kernel.kasan ../kernel.kc"
    exit 1
fi

# Build minimal launchd and create root filesystem ramdisk
LAUNCHD_SRC="tools/launchd/launchd.s"
LAUNCHD_BIN="tools/launchd/launchd"
ROOTFS_IMG="${ESP_DIR}/rootfs.dmg"

if [[ -f "$LAUNCHD_SRC" ]]; then
    echo "==> Building launchd..."
    clang -target x86_64-apple-macos -nostdlib -static -e _main -o "$LAUNCHD_BIN" "$LAUNCHD_SRC"

    echo "==> Creating rootfs.dmg..."
    rm -f /tmp/rootfs_build.dmg
    hdiutil create -size 4m -fs HFS+ -volname Root -layout NONE /tmp/rootfs_build.dmg
    hdiutil attach /tmp/rootfs_build.dmg -mountpoint /tmp/rootfs_mount
    mkdir -p /tmp/rootfs_mount/sbin
    cp "$LAUNCHD_BIN" /tmp/rootfs_mount/sbin/launchd
    hdiutil detach /tmp/rootfs_mount
    cp /tmp/rootfs_build.dmg "$ROOTFS_IMG"
    rm -f /tmp/rootfs_build.dmg
fi

OVMF_CODE="DEBUGX64_OVMF.fd"
if [[ ! -f "$OVMF_CODE" ]]; then
    curl -L https://retrage.github.io/edk2-nightly/bin/DEBUGX64_OVMF.fd -o "$OVMF_CODE"
fi

OVMF_VARS="DEBUGX64_OVMF_VARS.fd"
if [[ ! -f "$OVMF_VARS" ]]; then
    curl -L https://retrage.github.io/edk2-nightly/bin/DEBUGX64_OVMF_VARS.fd -o "$OVMF_VARS"
fi

echo "==> Starting QEMU..."
echo "    EFI:  $EFI_BINARY"
echo "    OVMF: $OVMF_CODE"
echo "    OVMF Vars: $OVMF_VARS"
echo "    ESP:  $ESP_DIR"

exec qemu-system-x86_64 \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
    -drive if=pflash,format=raw,file="$OVMF_VARS" \
    -drive format=raw,file=fat:rw:"${ESP_DIR}" \
    -machine q35 \
    -m 2048M \
    -cpu Penryn,+sse4.2,tsc-frequency=3000000000 \
    -smp 1 \
    -nographic \
    -no-reboot \
    -D target/qemu.log \
    "$@"
