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

if [[ -f "../kernel.kasan" ]]; then
    cp "../kernel.kasan" "${ESP_DIR}/kernel.kasan"
else
    echo "ERROR: kernel.kasan not found in parent directory"
    exit 1
fi

OVMF_CODE="../DEBUGX64_OVMF.fd"
if [[ ! -f "$OVMF_CODE" ]]; then
    curl -L https://retrage.github.io/edk2-nightly/bin/DEBUGX64_OVMF.fd -o "$OVMF_CODE"
fi

OVMF_VARS="../DEBUGX64_OVMF_VARS.fd"
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
    -cpu Penryn,+sse4.2,+x2apic,tsc-frequency=3000000000 \
    -smp 8 \
    -nographic \
    -no-reboot \
    -D target/qemu.log \
    "$@"
