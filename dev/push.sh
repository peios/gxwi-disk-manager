#!/bin/sh
# Runs on the host: build Disk Manager and put it in the share of the VM that
# ../gxwi/dev/boot.sh started, with its icon for the base theme and its
# declaration for the catalogue. GXWI's dev service puts them where a package
# would, and its dev loop restarts on a new one. Boot the VM with the disks
# dev/disks.sh makes, to have something on them to look at.
#
# The lsblk it reads goes on the share too, built on its own from
# ../peiosutils, until a peiosutils whose lsblk gives partitions' types,
# names and ids is in the image.
#
# The rename makes the new binary appear whole, never half-written.
set -eu
cd "$(dirname "$0")/.."
. dev/env.sh
[ -d ../gxwi/target/vmshare ] || { echo "no ../gxwi/target/vmshare: boot the VM from ../gxwi first" >&2; exit 1; }
share=$(cd ../gxwi/target/vmshare && pwd)
cargo build --release
(cd ../peiosutils && cargo build --release -p pu_lsblk --bin lsblk)
cp ../peiosutils/target/release/lsblk "$share/lsblk.new"
mv "$share/lsblk.new" "$share/lsblk"
mkdir -p "$share/icons/base"
cp gxwi-disk-manager.svg "$share/icons/base/dev.peios.gxwi-disk-manager.svg"
mkdir -p "$share/apps"
cp dev.peios.gxwi-disk-manager.toml "$share/apps/dev.peios.gxwi-disk-manager.toml"
cp target/release/gxwi-disk-manager "$share/gxwi-disk-manager.new"
mv "$share/gxwi-disk-manager.new" "$share/gxwi-disk-manager"
