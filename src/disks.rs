//! What this machine's storage is: its disks, their partitions and the
//! space between them, the filesystems on them, and everything mounted.
//!
//! All of it is read as the person, and none of it changes anything:
//! lsblk for each disk and what is on it, sysfs for where partitions lie,
//! the kernel's mount table for what is mounted, statvfs for how full each
//! filesystem is, the kernel for each mount's policy, and stratafs for the
//! system's views. What can't be read says why, where it would be.

use std::collections::HashMap;
use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Command;

use peios::file::{File, MountPolicyKind};
use serde::Deserialize;

use crate::service::{self, Startup};

/// The space a partition table keeps at each end, and where partitions are
/// put: on whole mebibytes, as `part` puts them.
pub const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Part of the machine.
    Fixed,
    /// Plugged in, and taken out again: a USB stick, a card.
    Removable,
    /// A drive for discs.
    Optical,
    /// A file attached as a disk (a loop device).
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filesystem {
    /// As lsblk names it: `ext4`, `vfat`, `iso9660`, `swap`.
    pub kind: String,
    pub version: Option<String>,
    pub label: Option<String>,
    pub uuid: Option<String>,
}

/// Somewhere a filesystem can be: a partition, or a whole disk without a
/// partition table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// `vdb2`, or the disk's own name.
    pub name: String,
    /// `/dev/vdb2`.
    pub path: String,
    pub size: u64,
    pub read_only: bool,
    /// Whether the person could open its disk to look: when not, no
    /// filesystem means not known, rather than none.
    pub readable: bool,
    pub filesystem: Option<Filesystem>,
    /// Where it is mounted, as lsblk saw it.
    pub mounted: Vec<String>,
    /// For a partition: where it lies and what its table says of it.
    pub partition: Option<Partition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub number: u32,
    /// Bytes from the start of the disk.
    pub start: u64,
    /// The table's type: a GUID on GPT, a byte (`0x83`) on MBR.
    pub kind: Option<String>,
    /// GPT's name for it.
    pub name: Option<String>,
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub name: String,
    pub path: String,
    pub kind: Kind,
    pub size: u64,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// How it is connected: `virtio`, `sata`, `usb`, `nvme`.
    pub transport: Option<String>,
    pub read_only: bool,
    /// `gpt` or `dos`; none for a disk without a partition table, or one
    /// that couldn't be read.
    pub table: Option<String>,
    /// Whether the person could open it to read its table and filesystems.
    pub readable: bool,
    pub table_uuid: Option<String>,
    pub sector: u64,
    /// A filesystem on the whole disk, without a partition table (or, on a
    /// hybrid medium, beside one).
    pub whole: Option<Volume>,
    pub partitions: Vec<Volume>,
    /// For an image: the file it is.
    pub backing: Option<String>,
}

/// A stretch of a disk, in order: a partition, or space nothing uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stretch<'a> {
    Used(&'a Volume),
    Free { start: u64, size: u64 },
}

impl Disk {
    pub fn volumes(&self) -> impl Iterator<Item = &Volume> {
        self.whole.iter().chain(self.partitions.iter())
    }

    /// Whether anything on it is mounted.
    pub fn in_use(&self) -> bool {
        self.volumes().any(|v| !v.mounted.is_empty())
    }

    /// Its partitions and the space between them, in order. Space smaller
    /// than a mebibyte is the table's own, or rounding: none worth showing.
    pub fn stretches(&self) -> Vec<Stretch<'_>> {
        // Partitions mean a table, even one that couldn't be read: the
        // kernel lists them, with where they lie, for anyone.
        if self.table.is_none() && self.partitions.is_empty() {
            return Vec::new();
        }
        // GPT keeps 33 sectors at the end for its copy of the table.
        let end = if self.table.as_deref() == Some("dos") { self.size } else { self.size.saturating_sub(33 * self.sector) };
        let mut out = Vec::new();
        let mut at = MIB;
        let mut parts: Vec<&Volume> = self.partitions.iter().collect();
        parts.sort_by_key(|v| v.partition.as_ref().map_or(0, |p| p.start));
        for volume in parts {
            let start = volume.partition.as_ref().map_or(0, |p| p.start);
            if start >= at + MIB {
                out.push(Stretch::Free { start: at, size: start - at });
            }
            out.push(Stretch::Used(volume));
            at = at.max(align_up(start + volume.size));
        }
        if end >= at + MIB {
            out.push(Stretch::Free { start: at, size: (end - at) / MIB * MIB });
        }
        out
    }
}

fn align_up(n: u64) -> u64 {
    n.div_ceil(MIB) * MIB
}

/// One stratum of a StrataFS view.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Stratum {
    pub path: String,
    #[serde(default)]
    pub flags: Vec<String>,
    pub state: String,
}

/// A mount policy, as the kernel holds it for a filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub kind: PolicyKind,
    /// The template's SDDL, for a policy that makes descriptors.
    pub template: Option<String>,
    /// The descriptor itself, for the permissions editor.
    pub template_sd: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyKind {
    Unmanaged,
    DenyMissing,
    Ephemeral,
    Persistent,
}

impl PolicyKind {
    pub const CHOSEN: [PolicyKind; 3] = [PolicyKind::DenyMissing, PolicyKind::Ephemeral, PolicyKind::Persistent];

    fn from_raw(kind: MountPolicyKind) -> Option<PolicyKind> {
        Some(match kind {
            MountPolicyKind::UNMANAGED => PolicyKind::Unmanaged,
            MountPolicyKind::DENY_MISSING => PolicyKind::DenyMissing,
            MountPolicyKind::SYNTHESIZE_EPHEMERAL => PolicyKind::Ephemeral,
            MountPolicyKind::SYNTHESIZE_PERSISTENT => PolicyKind::Persistent,
            _ => return None,
        })
    }

    /// As `mount -o policy=` names it, and a field holds it.
    pub fn id(self) -> &'static str {
        match self {
            PolicyKind::Unmanaged => "unmanaged",
            PolicyKind::DenyMissing => "deny-missing",
            PolicyKind::Ephemeral => "synth-ephemeral",
            PolicyKind::Persistent => "synth-persist",
        }
    }

    pub fn by(id: &str) -> Option<PolicyKind> {
        [PolicyKind::Unmanaged, PolicyKind::DenyMissing, PolicyKind::Ephemeral, PolicyKind::Persistent].into_iter().find(|k| k.id() == id)
    }

    /// Whether files without permissions of their own are given the
    /// template's.
    pub fn makes(self) -> bool {
        matches!(self, PolicyKind::Ephemeral | PolicyKind::Persistent)
    }
}

/// Where a mount comes from, as the mounts page groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// A filesystem on a disk.
    Storage,
    /// The system's own tree: its root and its StrataFS views.
    System,
    /// A folder shared from another machine, or from the host of this one.
    Shared,
    /// The kernel's own filesystems, and those held in memory.
    Kernel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub id: u32,
    pub target: String,
    /// The filesystem's type, as the kernel names it.
    pub kind: String,
    pub source: String,
    /// Options of the mount and of its filesystem, together.
    pub options: Vec<String>,
    pub read_only: bool,
    /// `major:minor` of what it is mounted from.
    pub device: String,
    pub origin: Origin,
    /// A later mount covers this one, at the same place or above it.
    pub hidden: bool,
    /// Bytes in all and bytes free, where they could be read.
    pub usage: Option<(u64, u64)>,
    pub policy: Result<Policy, String>,
    pub strata: Vec<Stratum>,
}

impl Mount {
    /// Why Peios would stop working without it, or nothing for a mount
    /// that can be taken away.
    pub fn needed(&self) -> Option<&'static str> {
        const PLACES: [&str; 8] = ["/", "/media/peios", "/boot", "/boot/efi", "/usr", "/lcl", "/system", "/home"];
        if self.origin == Origin::Kernel || self.origin == Origin::System {
            return Some("Peios needs it to run.");
        }
        if PLACES.contains(&self.target.as_str()) {
            return Some("Peios runs from it.");
        }
        if ["/proc", "/sys", "/dev", "/run"].iter().any(|p| self.target.starts_with(&format!("{p}/"))) {
            return Some("Peios needs it to run.");
        }
        None
    }

    /// Whether its policy can be changed: the kernel keeps one it can't for
    /// StrataFS views and for filesystems it doesn't manage.
    pub fn fixed_policy(&self) -> Option<&'static str> {
        if self.kind == "stratafs" {
            return Some("A system view always denies files without permissions of their own: StrataFS fixes its policy.");
        }
        match &self.policy {
            Ok(p) if p.kind == PolicyKind::Unmanaged => Some("The kernel doesn't manage permissions on this filesystem, so it has no policy to change."),
            _ => None,
        }
    }
}

pub struct Machine {
    pub disks: Result<Vec<Disk>, String>,
    pub mounts: Result<Vec<Mount>, String>,
    pub startup: Result<Vec<Startup>, String>,
    /// Whether the person may change this machine's storage, or why not.
    pub may: Result<(), String>,
}

impl Machine {
    pub fn empty() -> Machine {
        Machine { disks: Ok(Vec::new()), mounts: Ok(Vec::new()), startup: Ok(Vec::new()), may: Err(String::new()) }
    }

    pub fn disk(&self, name: &str) -> Option<&Disk> {
        self.disks.as_ref().ok()?.iter().find(|d| d.name == name)
    }

    /// A volume by its name, and the disk it is on.
    pub fn volume(&self, name: &str) -> Option<(&Disk, &Volume)> {
        self.disks.as_ref().ok()?.iter().find_map(|d| d.volumes().find(|v| v.name == name).map(|v| (d, v)))
    }

    pub fn mount(&self, id: u32) -> Option<&Mount> {
        self.mounts.as_ref().ok()?.iter().find(|m| m.id == id)
    }

    /// The mounts of a volume, by the device they are mounted from.
    pub fn mounts_of(&self, volume: &Volume) -> Vec<&Mount> {
        let Ok(mounts) = &self.mounts else { return Vec::new() };
        let device = device_number(&volume.name);
        mounts.iter().filter(|m| device.as_deref() == Some(m.device.as_str()) || m.source == volume.path).collect()
    }

    /// Why a disk can't be changed as a whole, or nothing: what holds the
    /// running system first, then what is mounted.
    pub fn disk_busy(&self, disk: &Disk) -> Option<String> {
        if self.holds_system(disk) {
            return Some("It holds the running system.".into());
        }
        if disk.in_use() {
            return Some("Something on it is mounted: unmount it first.".into());
        }
        None
    }

    /// Whether the system runs from something on the disk.
    pub fn holds_system(&self, disk: &Disk) -> bool {
        disk.volumes().any(|v| self.mounts_of(v).iter().any(|m| m.needed().is_some()))
            || disk.backing.as_deref().is_some_and(|file| {
                // An image the system runs from is on the medium it started
                // from, as the live system's is.
                self.mounts.as_ref().is_ok_and(|mounts| mounts.iter().any(|m| m.needed().is_some() && m.origin == Origin::Storage && file.starts_with(&m.target) && m.target != "/"))
            })
            || disk.kind == Kind::Image && disk.backing.is_none()
    }
}

/// Everything, read again.
pub fn read() -> Machine {
    let mounts = mounts();
    let mut disks = disks();
    // A disk the person can't open still says what is mounted from it, in
    // the mount table: the type of that filesystem, at least, is known.
    if let (Ok(disks), Ok(mounts)) = (&mut disks, &mounts) {
        for volume in disks.iter_mut().filter(|d| !d.readable).flat_map(|d| d.whole.iter_mut().chain(d.partitions.iter_mut())) {
            if volume.filesystem.is_none()
                && let Some(device) = device_number(&volume.name)
                && let Some(m) = mounts.iter().find(|m| m.device == device)
            {
                volume.filesystem = Some(Filesystem { kind: m.kind.clone(), version: None, label: None, uuid: None });
            }
        }
    }
    Machine { disks, mounts, startup: service::startup(), may: service::may() }
}

/// What changes when disks come and go, cheaply: the block devices and
/// their sizes. A change means reading everything again.
pub fn signature() -> String {
    let Ok(entries) = fs::read_dir("/sys/class/block") else { return String::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let size = fs::read_to_string(e.path().join("size")).unwrap_or_default();
            format!("{name}={}", size.trim())
        })
        .collect();
    names.sort();
    let mounts = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    format!("{}|{}", names.join(","), mounts.len() + mounts.lines().count())
}

#[derive(Debug, Deserialize)]
struct Listing {
    blockdevices: Vec<Device>,
}

#[derive(Debug, Deserialize)]
struct Device {
    name: String,
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<String>,
    fstype: Option<String>,
    fsver: Option<String>,
    label: Option<String>,
    uuid: Option<String>,
    pttype: Option<String>,
    ptuuid: Option<String>,
    parttype: Option<String>,
    partlabel: Option<String>,
    partuuid: Option<String>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    rm: bool,
    #[serde(default)]
    ro: bool,
    model: Option<String>,
    serial: Option<String>,
    tran: Option<String>,
    #[serde(default)]
    children: Vec<Device>,
}

const COLUMNS: &str = "NAME,PATH,TYPE,SIZE,FSTYPE,FSVER,LABEL,UUID,PTTYPE,PTUUID,PARTTYPE,PARTLABEL,PARTUUID,MOUNTPOINTS,RM,RO,MODEL,SERIAL,TRAN";

fn disks() -> Result<Vec<Disk>, String> {
    let output = Command::new("lsblk").args(["-J", "-b", "-o", COLUMNS]).output().map_err(|e| format!("lsblk couldn't be run: {e}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if said.is_empty() { "lsblk couldn't list the disks".into() } else { said });
    }
    let listing: Listing = serde_json::from_slice(&output.stdout).map_err(|e| format!("lsblk's answer couldn't be read: {e}"))?;
    let mut disks: Vec<Disk> = listing.blockdevices.into_iter().filter_map(disk).collect();
    disks.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    Ok(disks)
}

fn sysfs(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn size(text: &Option<String>) -> u64 {
    text.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn filesystem(device: &Device) -> Option<Filesystem> {
    device.fstype.as_ref().map(|kind| Filesystem {
        kind: kind.clone(),
        version: device.fsver.clone(),
        label: device.label.clone(),
        uuid: device.uuid.clone(),
    })
}

fn mounted(device: &Device) -> Vec<String> {
    device.mountpoints.iter().flatten().cloned().collect()
}

fn disk(device: Device) -> Option<Disk> {
    let at = Path::new("/sys/block").join(&device.name);
    let size = size(&device.size);
    let kind = match device.kind.as_str() {
        "rom" => Kind::Optical,
        "loop" => Kind::Image,
        "disk" if device.rm || device.tran.as_deref() == Some("usb") => Kind::Removable,
        "disk" => Kind::Fixed,
        _ => return None,
    };
    // A loop device with nothing attached, and drives that hold nothing a
    // person would call a disk: a floppy drive's few kilobytes, RAM disks,
    // device-mapper views of other disks.
    if ["ram", "zram", "dm-", "md", "fd"].iter().any(|p| device.name.starts_with(p)) {
        return None;
    }
    if kind == Kind::Image && size == 0 {
        return None;
    }
    let sector = sysfs(at.join("queue/logical_block_size")).and_then(|s| s.parse().ok()).unwrap_or(512);
    let readable = can_open(&device.path);
    let partitions: Vec<Volume> = device
        .children
        .iter()
        .filter(|c| c.kind == "part")
        .map(|c| {
            let part = at.join(&c.name);
            let number = sysfs(part.join("partition")).and_then(|s| s.parse().ok()).unwrap_or(0);
            // sysfs counts in 512-byte sectors, whatever the disk's own.
            let start = sysfs(part.join("start")).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) * 512;
            Volume {
                name: c.name.clone(),
                path: c.path.clone(),
                size: self::size(&c.size),
                read_only: c.ro,
                readable,
                filesystem: filesystem(c),
                mounted: mounted(c),
                partition: Some(Partition { number, start, kind: c.parttype.clone(), name: c.partlabel.clone(), uuid: c.partuuid.clone() }),
            }
        })
        .collect();
    let whole = filesystem(&device).map(|fs| Volume {
        name: device.name.clone(),
        path: device.path.clone(),
        size,
        read_only: device.ro,
        readable,
        filesystem: Some(fs),
        mounted: mounted(&device),
        partition: None,
    });
    let model = device.model.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(String::from).or_else(|| sysfs(at.join("device/model")));
    Some(Disk {
        name: device.name.clone(),
        path: device.path.clone(),
        kind,
        size,
        model,
        serial: device.serial.clone().filter(|s| !s.trim().is_empty()),
        transport: device.tran.clone(),
        read_only: device.ro,
        table: device.pttype.clone(),
        readable,
        table_uuid: device.ptuuid.clone(),
        sector,
        whole,
        partitions,
        backing: if kind == Kind::Image { sysfs(at.join("loop/backing_file")) } else { None },
    })
}

/// Whether the person may open a disk to read what is on it. lsblk reads
/// partition tables and filesystems by opening the disk, and says nothing
/// when it can't: this tells nothing from couldn't-look. Only a refusal
/// counts; a drive with no disc in it is simply empty.
fn can_open(path: &str) -> bool {
    match fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(path) {
        Ok(_) => true,
        Err(e) => !matches!(e.raw_os_error(), Some(libc::EACCES) | Some(libc::EPERM)),
    }
}

/// `major:minor` of a block device, as the mount table names it.
fn device_number(name: &str) -> Option<String> {
    sysfs(Path::new("/sys/class/block").join(name).join("dev"))
}

/// The kernel's mount table: `/proc/self/mountinfo`, one line a mount.
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    let mut mounts: Vec<Mount> = text
        .lines()
        .filter_map(|line| {
            let (before, after) = line.split_once(" - ")?;
            let before: Vec<&str> = before.split(' ').collect();
            let after: Vec<&str> = after.split(' ').collect();
            if before.len() < 6 || after.len() < 3 {
                return None;
            }
            let kind = unescape(after[0]);
            let source = unescape(after[1]);
            let mount_options: Vec<String> = before[5].split(',').map(String::from).collect();
            let read_only = mount_options.iter().any(|o| o == "ro");
            let mut options = mount_options;
            options.extend(after[2].split(',').filter(|o| *o != "rw" && *o != "ro").map(String::from));
            let origin = origin(&kind, &source);
            Some(Mount {
                id: before[0].parse().ok()?,
                target: unescape(before[4]),
                kind,
                source,
                options,
                read_only,
                device: before[2].to_string(),
                origin,
                hidden: false,
                usage: None,
                policy: Err(String::new()),
                strata: Vec::new(),
            })
        })
        .collect();
    // A later mount (mount ids only grow) at the same place, or at a place
    // above, covers an earlier one: the person sees only what is on top.
    // The root is mounted early and moved into place, so it covers nothing.
    let covers: Vec<(u32, String)> = mounts.iter().map(|m| (m.id, m.target.clone())).collect();
    for mount in &mut mounts {
        mount.hidden = covers.iter().any(|(id, target)| *id > mount.id && (*target == mount.target || (target != "/" && is_under(&mount.target, target))));
    }
    mounts
}

fn is_under(path: &str, above: &str) -> bool {
    path.strip_prefix(above).is_some_and(|rest| rest.starts_with('/'))
}

/// mountinfo writes a space, a tab, a newline and a backslash as octal.
fn unescape(text: &str) -> String {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b)) {
            let n = (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(n);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn origin(kind: &str, source: &str) -> Origin {
    match kind {
        "stratafs" | "overlay" => Origin::System,
        "9p" | "nfs" | "nfs4" | "cifs" | "smb3" | "virtiofs" | "sshfs" | "fuse.sshfs" => Origin::Shared,
        _ if source.starts_with("/dev/") => Origin::Storage,
        _ => Origin::Kernel,
    }
}

fn mounts() -> Result<Vec<Mount>, String> {
    let text = fs::read_to_string("/proc/self/mountinfo").map_err(|e| format!("The mount table couldn't be read: {e}"))?;
    let mut mounts = parse_mountinfo(&text);
    let strata = strata();
    for mount in &mut mounts {
        if mount.hidden {
            mount.policy = Err("Another mount covers it, so it can't be reached to ask.".into());
            continue;
        }
        if mount.origin != Origin::Kernel {
            mount.usage = statvfs(&mount.target);
        }
        mount.policy = policy(&mount.target);
        if mount.kind == "stratafs" {
            mount.strata = strata.get(&mount.target).cloned().unwrap_or_default();
        }
    }
    Ok(mounts)
}

fn statvfs(path: &str) -> Option<(u64, u64)> {
    let path = std::ffi::CString::new(path).ok()?;
    // SAFETY: a live path and a struct statvfs to fill.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let block = s.f_frsize;
    (s.f_blocks > 0).then_some((s.f_blocks * block, s.f_bavail * block))
}

/// The policy of the filesystem mounted at `target`, asked of the kernel
/// through a handle on it that opens nothing.
fn policy(target: &str) -> Result<Policy, String> {
    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(target)
        .map_err(|e| format!("It couldn't be opened to ask: {}", crate::words::error(&e)))?;
    let file = File::from(OwnedFd::from(handle));
    let got = file.mount_get_policy().map_err(|e| {
        if e.raw_os_error() == Some(libc::EPERM) || e.raw_os_error() == Some(libc::EACCES) {
            "Reading it needs the Manage Volumes privilege.".to_string()
        } else {
            format!("The kernel didn't say: {}", crate::words::error(&e.into()))
        }
    })?;
    let kind = PolicyKind::from_raw(got.kind).ok_or("The kernel gave a policy this window doesn't know.")?;
    let template_sd = got.template_sd.map(|sd| sd.as_bytes().to_vec());
    let template = template_sd.as_deref().and_then(|sd| peios::security::sddl::format(sd).ok());
    Ok(Policy { kind, template, template_sd })
}

/// The strata of every StrataFS view, by where it is mounted.
fn strata() -> HashMap<String, Vec<Stratum>> {
    #[derive(Deserialize)]
    struct View {
        mount: String,
        strata: Vec<Stratum>,
    }
    let Ok(output) = Command::new("stratafs").args(["list", "--json"]).output() else { return HashMap::new() };
    serde_json::from_slice::<Vec<View>>(&output.stdout).map(|views| views.into_iter().map(|v| (v.mount, v.strata)).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
26 42 0:24 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw
37 42 254:0 / /media/peios ro,relatime - iso9660 /dev/vda ro,nojoliet
42 2 0:35 / / rw,relatime - overlay overlay rw,lowerdir=/mnt/rootfs.lower
47 42 0:42 / /share rw,relatime - stratafs none rw,strata=/lcl/share+create:/usr/share+ro+am
57 47 0:51 / /share rw,relatime - 9p hostshare rw,access=client,trans=virtio
60 42 254:18 / /mnt/my\\040data rw,relatime - ext4 /dev/vdb2 rw";

    #[test]
    fn the_mount_table_is_read() {
        let mounts = parse_mountinfo(MOUNTINFO);
        assert_eq!(mounts.len(), 6);
        let medium = &mounts[1];
        assert_eq!((medium.target.as_str(), medium.kind.as_str(), medium.source.as_str()), ("/media/peios", "iso9660", "/dev/vda"));
        assert!(medium.read_only);
        assert_eq!(medium.origin, Origin::Storage);
        assert_eq!(mounts[0].origin, Origin::Kernel);
        assert_eq!(mounts[3].origin, Origin::System);
        assert_eq!(mounts[4].origin, Origin::Shared);
        assert_eq!(mounts[5].target, "/mnt/my data");
        assert_eq!(mounts[5].device, "254:18");
    }

    #[test]
    fn a_mount_covered_by_a_later_one_is_hidden() {
        let mounts = parse_mountinfo(MOUNTINFO);
        // The view at /share is under the host's share mounted over it.
        assert!(mounts[3].hidden);
        assert!(!mounts[4].hidden);
        // Everything is under /, which covers nothing it was mounted first.
        assert!(!mounts[2].hidden);
        assert!(!mounts[0].hidden);
    }

    #[test]
    fn what_peios_runs_from_is_needed() {
        let mounts = parse_mountinfo(MOUNTINFO);
        assert!(mounts[1].needed().is_some());
        assert!(mounts[0].needed().is_some());
        assert!(mounts[5].needed().is_none());
    }

    fn part(name: &str, start: u64, size: u64) -> Volume {
        Volume {
            name: name.into(),
            path: format!("/dev/{name}"),
            size,
            read_only: false,
            readable: true,
            filesystem: None,
            mounted: Vec::new(),
            partition: Some(Partition { number: 1, start, kind: None, name: None, uuid: None }),
        }
    }

    fn disk(size: u64, partitions: Vec<Volume>) -> Disk {
        Disk {
            name: "vdb".into(),
            path: "/dev/vdb".into(),
            kind: Kind::Fixed,
            size,
            model: None,
            serial: None,
            transport: None,
            read_only: false,
            table: Some("gpt".into()),
            readable: true,
            table_uuid: None,
            sector: 512,
            whole: None,
            partitions,
            backing: None,
        }
    }

    #[test]
    fn free_space_is_found_between_and_after_partitions() {
        let gib = 1024 * MIB;
        let d = disk(8 * gib, vec![part("vdb2", 600 * MIB, gib), part("vdb1", MIB, 512 * MIB)]);
        let s = d.stretches();
        assert_eq!(s.len(), 4);
        assert!(matches!(s[0], Stretch::Used(v) if v.name == "vdb1"));
        assert_eq!(s[1], Stretch::Free { start: 513 * MIB, size: 87 * MIB });
        assert!(matches!(s[2], Stretch::Used(v) if v.name == "vdb2"));
        // To the end, less GPT's copy of its table, in whole mebibytes.
        assert_eq!(s[3], Stretch::Free { start: 1624 * MIB, size: 8 * gib - MIB - 1624 * MIB });
    }

    #[test]
    fn an_empty_table_is_all_free_and_no_table_is_nothing() {
        let d = disk(1024 * MIB, Vec::new());
        assert!(matches!(d.stretches()[..], [Stretch::Free { start, .. }] if start == MIB));
        let mut bare = disk(1024 * MIB, Vec::new());
        bare.table = None;
        assert!(bare.stretches().is_empty());
    }

    #[test]
    fn policies_are_named_as_mount_names_them() {
        for kind in PolicyKind::CHOSEN {
            assert_eq!(PolicyKind::by(kind.id()), Some(kind));
        }
    }
}
