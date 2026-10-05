//! What Disk Manager says, in words a person reads.

use crate::disks::{Disk, Filesystem, Kind, PolicyKind, Volume};
use crate::service::{Format, PartitionType};

/// A count of bytes, as the other apps say them: in the largest unit it
/// makes sense in.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    // Below ten, to a tenth, unless that tenth is nothing: 2047 MB is 2 GB.
    let tenths = (value * 10.0).round() / 10.0;
    if value < 10.0 && tenths.fract() != 0.0 {
        format!("{tenths:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Bytes exactly, with thousands apart: for the facts of a disk.
pub fn exact(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out} bytes")
}

/// One thing or many.
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What a disk is called: its model, or what kind of disk it is.
pub fn disk_title(disk: &Disk) -> String {
    if let Some(model) = &disk.model {
        return model.clone();
    }
    // Without a model (as a virtual disk has none), how it is connected
    // says most.
    match (disk.kind, &disk.transport) {
        (Kind::Fixed | Kind::Removable, Some(t)) => format!("{} Disk", transport(t)),
        (Kind::Fixed, None) => "Disk".into(),
        (Kind::Removable, None) => "Removable Disk".into(),
        (Kind::Optical, _) => "Disc Drive".into(),
        (Kind::Image, _) => "Disk Image".into(),
    }
}

pub fn kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Fixed => "disk",
        Kind::Removable => "removable disk",
        Kind::Optical => "disc drive",
        Kind::Image => "disk image",
    }
}

pub fn transport(transport: &str) -> String {
    match transport {
        "virtio" => "Virtio".into(),
        "sata" => "SATA".into(),
        "ata" => "ATA".into(),
        "usb" => "USB".into(),
        "nvme" => "NVMe".into(),
        "scsi" => "SCSI".into(),
        "mmc" => "SD card".into(),
        "iscsi" => "iSCSI".into(),
        other => other.to_uppercase(),
    }
}

/// A disk in a line: its size, how it is connected and what it is.
pub fn disk_line(disk: &Disk) -> String {
    let mut line = bytes(disk.size);
    if let Some(t) = &disk.transport {
        line.push_str(&format!(" {}", transport(t)));
    }
    line.push_str(&format!(" {}", kind(disk.kind)));
    line
}

pub fn table(table: &str) -> String {
    match table {
        "gpt" => "GPT".into(),
        "dos" => "MBR".into(),
        other => other.to_uppercase(),
    }
}

/// A filesystem's type, as people know it.
pub fn filesystem(fs: &Filesystem) -> String {
    match fs.kind.as_str() {
        "vfat" => match fs.version.as_deref() {
            Some(v) if v.starts_with("FAT") => v.to_string(),
            _ => "FAT".into(),
        },
        "ext2" | "ext3" | "ext4" | "xfs" | "btrfs" | "f2fs" => fs.kind.clone(),
        "iso9660" => "ISO 9660".into(),
        "udf" => "UDF".into(),
        "swap" => "Swap".into(),
        "squashfs" => "SquashFS".into(),
        "ntfs" | "ntfs3" => "NTFS".into(),
        "exfat" => "exFAT".into(),
        "crypto_LUKS" => "Encrypted (LUKS)".into(),
        "LVM2_member" => "LVM".into(),
        "linux_raid_member" => "RAID member".into(),
        other => other.to_string(),
    }
}

/// A mount's filesystem type, as the kernel names it.
pub fn mount_kind(kind: &str) -> String {
    match kind {
        "vfat" => "FAT".into(),
        "stratafs" => "StrataFS".into(),
        "9p" => "9P (shared folder)".into(),
        "devtmpfs" => "devtmpfs".into(),
        other => filesystem(&Filesystem { kind: other.into(), version: None, label: None, uuid: None }),
    }
}

/// What a partition's table says it is for.
pub fn partition_type(kind: &str) -> String {
    let named = match kind.to_ascii_lowercase().as_str() {
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" | "0xef" => "EFI System",
        "0fc63daf-8483-4772-8e79-3d69d8477de4" | "0x83" => "Linux Filesystem",
        "4f68bce3-e8cd-4db1-96e7-fbcaf984b709" => "Linux Root (x86-64)",
        "933ac7e1-2eb4-4f13-b844-0e14e2aef915" => "Linux Home",
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f" | "0x82" => "Linux Swap",
        "e6d6d379-f507-44c2-a23c-238f2a3df928" | "0x8e" => "Linux LVM",
        "a19d880f-05fc-4d3b-a006-743f0f84911e" | "0xfd" => "Linux RAID",
        "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7" | "0x07" => "Microsoft Basic Data",
        "e3c9e316-0b5c-4db8-817d-f92df00215ae" => "Microsoft Reserved",
        "de94bba4-06d1-4d40-a16a-bfd50179d6ac" | "0x27" => "Windows Recovery",
        "21686148-6449-6e6f-744e-656564454649" => "BIOS Boot",
        "0x0b" | "0x0c" => "FAT32",
        "0x05" | "0x0f" => "Extended",
        _ => return kind.to_string(),
    };
    named.to_string()
}

pub fn new_partition_type(kind: PartitionType) -> &'static str {
    match kind {
        PartitionType::Linux => "Linux Filesystem",
        PartitionType::Esp => "EFI System",
        PartitionType::Swap => "Linux Swap",
        PartitionType::MicrosoftData => "Microsoft Basic Data",
    }
}

pub fn format(format: Format) -> &'static str {
    match format {
        Format::Ext4 => "ext4",
        Format::Fat32 => "FAT32",
    }
}

/// A volume's name: a partition by its number, a whole disk as itself.
pub fn volume_title(volume: &Volume) -> String {
    match &volume.partition {
        Some(p) => format!("Partition {}", p.number),
        None => "Whole Disk".into(),
    }
}

/// What is on a volume, in a line: its size, its filesystem and label.
pub fn volume_line(volume: &Volume) -> String {
    let mut parts = vec![bytes(volume.size)];
    match &volume.filesystem {
        Some(fs) => {
            parts.push(filesystem(fs));
            if let Some(label) = &fs.label {
                parts.push(label.clone());
            }
        }
        None if volume.readable => parts.push("Not formatted".into()),
        None => parts.push("Contents unknown".into()),
    }
    parts.join(" · ")
}

/// What a policy does, as a choice is named.
pub fn policy(kind: PolicyKind) -> &'static str {
    match kind {
        PolicyKind::Unmanaged => "Not Managed",
        PolicyKind::DenyMissing => "Deny Access",
        PolicyKind::Ephemeral => "Give Permissions, Not Saved",
        PolicyKind::Persistent => "Give Permissions and Save Them",
    }
}

/// What a policy means, in a sentence.
pub fn policy_about(kind: PolicyKind) -> &'static str {
    match kind {
        PolicyKind::Unmanaged => "Permissions aren't checked here: the kernel doesn't manage this filesystem.",
        PolicyKind::DenyMissing => "A file without permissions of its own can't be opened by anyone until it is given some.",
        PolicyKind::Ephemeral => "A file without permissions of its own is given its folder's, or the template's, each time it is opened. Nothing is written to the disk.",
        PolicyKind::Persistent => "A file without permissions of its own is given its folder's, or the template's, and they are saved on it.",
    }
}

/// An I/O error, as a sentence ends.
pub fn error(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => "access was denied".into(),
        Some(libc::ENOENT) => "it isn't there".into(),
        _ => {
            let text = e.to_string();
            let text = text.split(" (os error").next().unwrap_or(&text);
            let mut chars = text.chars();
            chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_said_as_the_other_apps_say_them() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(512 * 1024 * 1024), "512 MB");
        assert_eq!(bytes(8 * 1024 * 1024 * 1024), "8 GB");
        assert_eq!(bytes(1_085_327_360), "1 GB");
        assert_eq!(bytes(2_500 * 1024 * 1024), "2.4 GB");
        assert_eq!(bytes(2_047 * 1024 * 1024), "2 GB");
        assert_eq!(exact(1_085_327_360), "1,085,327,360 bytes");
        assert_eq!(exact(512), "512 bytes");
    }

    #[test]
    fn partition_types_are_named_on_either_table() {
        assert_eq!(partition_type("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"), "EFI System");
        assert_eq!(partition_type("0x83"), "Linux Filesystem");
        assert_eq!(partition_type("deadbeef"), "deadbeef");
    }

    #[test]
    fn fat_says_its_size() {
        let fs = Filesystem { kind: "vfat".into(), version: Some("FAT32".into()), label: None, uuid: None };
        assert_eq!(filesystem(&fs), "FAT32");
    }
}
