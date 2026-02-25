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
cargo build --target x86_64-unknown-uefi $CARGO_FLAGS

EFI_BINARY="target/x86_64-unknown-uefi/${PROFILE}/xnuboot.efi"

ESP_DIR="target/esp"
mkdir -p "${ESP_DIR}/EFI/BOOT"
cp "$EFI_BINARY" "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"

if [[ -f "../kernel.development" ]]; then
    cp "../kernel.development" "${ESP_DIR}/kernel.development"
else
    echo "ERROR: kernel.development not found in parent directory"
    exit 1
fi

OVMF_CODE="$(find /opt/homebrew -name 'edk2-x86_64-code.fd' 2>/dev/null | head -1)"
if [[ -z "$OVMF_CODE" ]]; then
    echo "ERROR: OVMF firmware not found."
    exit 1
fi

OVMF_VARS="target/ovmf_vars.fd"
if [[ ! -f "$OVMF_VARS" ]]; then
    OVMF_VARS_SRC="$(find /opt/homebrew -name 'edk2-i386-vars.fd' 2>/dev/null | head -1)"
    if [[ -n "$OVMF_VARS_SRC" ]]; then
        cp "$OVMF_VARS_SRC" "$OVMF_VARS"
    else
        dd if=/dev/zero of="$OVMF_VARS" bs=256k count=1 2>/dev/null
    fi
fi

echo "==> Starting QEMU..."
echo "    EFI:  $EFI_BINARY"

exec qemu-system-x86_64 \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
    -drive if=pflash,format=raw,file="$OVMF_VARS" \
    -drive format=raw,file=fat:rw:"${ESP_DIR}" \
    -machine q35 \
    -m 512M \
    -cpu Penryn,-vmx \
    -smp 1 \
    -nographic \
    -no-reboot \
    -D target/qemu.log \
    -monitor unix:target/monitor.sock,server,nowait \
    -s \
    "$@"
