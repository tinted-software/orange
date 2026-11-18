#!/bin/sh
# Boots an ESP directory (see //tools/uefi-loader:esp) under QEMU and OVMF.
#
#   tools/uefi-loader/run_qemu.sh <esp-dir>
#
# OVMF is looked up in $OVMF, then the usual Linux locations. Serial output
# (the loader's log) goes to stdout; the window is disabled.
set -e
if [ -z "$1" ]; then
    echo "usage: $0 <esp-dir>" >&2
    exit 2
fi
if [ -z "$OVMF" ]; then
    for f in /usr/share/ovmf/x64/OVMF.4m.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/x64/OVMF.fd; do
        if [ -f "$f" ]; then OVMF=$f; break; fi
    done
fi
if [ ! -f "$OVMF" ]; then
    echo "OVMF firmware not found; set OVMF=/path/to/OVMF.fd" >&2
    exit 2
fi
exec qemu-system-x86_64 -machine q35 -m 512 -bios "$OVMF" \
    -drive file="fat:rw:$1",format=raw,if=ide \
    -display none -serial stdio -no-reboot
