#!/bin/sh
# Runs on the host: make the disks Disk Manager is looked at with, and print
# the QEMU arguments that attach them, for ../gxwi/dev/boot.sh's VM_EXTRA:
#
#     VM_EXTRA="$(../gxwi-disk-manager/dev/disks.sh)" dev/boot.sh
#
# The live VM has only its medium. These give it a fixed disk with an EFI
# system partition, an ext4 one, one never formatted and free space after
# them, and a USB stick with one FAT partition. They are sparse files, made
# once and kept: delete dev/disks to start again.
set -eu
cd "$(dirname "$0")"
mkdir -p disks
data=$PWD/disks/data.img
stick=$PWD/disks/stick.img
# A partition's first sector and its size in KiB, as mkfs is given them.
first() { sgdisk -i "$2" "$1" | sed -n 's/^First sector: \([0-9]*\).*/\1/p'; }
kib() { sgdisk -i "$2" "$1" | sed -n 's/^Partition size: \([0-9]*\) sectors.*/\1/p' | awk '{ print int($1 / 2) }'; }
if [ ! -e "$data" ]; then
    truncate -s 8G "$data"
    sgdisk -o \
        -n 1:1MiB:+512MiB -t 1:ef00 -c 1:"EFI system partition" \
        -n 2:0:+4GiB -t 2:8300 -c 2:Data \
        -n 3:0:+1GiB -t 3:8300 -c 3:Spare \
        "$data" >/dev/null
    mkfs.vfat -F 32 -n SYSTEM --offset "$(first "$data" 1)" "$data" "$(kib "$data" 1)" >&2
    mke2fs -q -F -t ext4 -L data -E offset=$(($(first "$data" 2) * 512)) "$data" "$(kib "$data" 2)k" >&2
fi
if [ ! -e "$stick" ]; then
    truncate -s 2G "$stick"
    sgdisk -o -n 1:1MiB:0 -t 1:0700 -c 1:Stick "$stick" >/dev/null
    mkfs.vfat -F 32 -n STICK --offset "$(first "$stick" 1)" "$stick" "$(kib "$stick" 1)" >&2
fi
printf '%s' "-drive file=$data,if=none,id=data,format=raw -device virtio-blk-pci,drive=data,serial=PEIOS-DATA-1 -device qemu-xhci -drive file=$stick,if=none,id=stick,format=raw -device usb-storage,drive=stick,removable=on"
