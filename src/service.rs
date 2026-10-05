//! Every change Disk Manager makes, and the one place they are made.
//!
//! Partitioning, formatting and mounting need authority a person's own
//! token doesn't carry (`part`'s re-read of a table is SeTcb's, and a new
//! filesystem's top-folder permissions need SeRestore), so they are made
//! by a privileged disk service, as the installer's are by installerd. That
//! service is phase 2 of PEI-1218 and doesn't exist yet: until it does,
//! [`request`] makes nothing and says so. Its client goes here, and
//! nothing else in this program knows how a change is carried.

use std::sync::Arc;

use crate::disks::PolicyKind;

/// A filesystem a disk is formatted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ext4,
    Fat32,
}

impl Format {
    pub const ALL: [Format; 2] = [Format::Ext4, Format::Fat32];

    pub fn id(self) -> &'static str {
        match self {
            Format::Ext4 => "ext4",
            Format::Fat32 => "fat32",
        }
    }

    pub fn by(id: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.id() == id)
    }

    /// The longest label it takes.
    pub fn label_limit(self) -> usize {
        match self {
            Format::Ext4 => 16,
            Format::Fat32 => 11,
        }
    }

    /// Whether files on it keep permissions of their own.
    pub fn keeps_permissions(self) -> bool {
        self == Format::Ext4
    }
}

/// What a new partition is for, as its table records it (`part add --type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionType {
    Linux,
    Esp,
    Swap,
    MicrosoftData,
}

impl PartitionType {
    pub const ALL: [PartitionType; 4] = [PartitionType::Linux, PartitionType::Esp, PartitionType::Swap, PartitionType::MicrosoftData];

    pub fn id(self) -> &'static str {
        match self {
            PartitionType::Linux => "linux",
            PartitionType::Esp => "esp",
            PartitionType::Swap => "swap",
            PartitionType::MicrosoftData => "msdata",
        }
    }

    pub fn by(id: &str) -> Option<PartitionType> {
        PartitionType::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// Where a mount's permissions come from for files without their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPolicy {
    pub kind: PolicyKind,
    /// The descriptor given to those files, for a policy that makes them.
    pub template: Option<Vec<u8>>,
}

/// A filesystem, as a mount at startup finds it: by its UUID, so a disk
/// that moves to another port is still found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    pub id: String,
    pub uuid: String,
    /// What it was called when added, to name it while it isn't attached.
    pub shown: String,
    pub target: String,
    pub read_only: bool,
    pub policy: MountPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A new, empty GPT table, which loses everything on the disk.
    NewTable { disk: String },
    AddPartition { disk: String, start: u64, size: u64, kind: PartitionType, name: String, format: Option<(Format, String)>, root: Option<Vec<u8>> },
    DeletePartition { volume: String },
    Format { volume: String, format: Format, label: String, root: Option<Vec<u8>> },
    Mount { volume: String, target: String, read_only: bool, policy: MountPolicy, at_startup: bool },
    Unmount { target: String },
    SetPolicy { target: String, policy: MountPolicy },
    AddStartup { startup: Startup },
    RemoveStartup { id: String },
    Eject { disk: String },
}

impl Change {
    /// What is being done, while it is.
    pub fn doing(&self) -> String {
        match self {
            Change::NewTable { disk } => format!("Erasing {disk}…"),
            Change::AddPartition { disk, .. } => format!("Adding a partition to {disk}…"),
            Change::DeletePartition { volume } => format!("Deleting {volume}…"),
            Change::Format { volume, .. } => format!("Formatting {volume}…"),
            Change::Mount { volume, target, .. } => format!("Mounting {volume} at {target}…"),
            Change::Unmount { target } => format!("Unmounting {target}…"),
            Change::SetPolicy { target, .. } => format!("Changing the policy of {target}…"),
            Change::AddStartup { startup } => format!("Adding {} to startup…", startup.target),
            Change::RemoveStartup { .. } => "Removing it from startup…".into(),
            Change::Eject { disk } => format!("Ejecting {disk}…"),
        }
    }

    /// What was done, once it is.
    pub fn done(&self) -> String {
        match self {
            Change::NewTable { disk } => format!("{disk} erased, with an empty partition table."),
            Change::AddPartition { disk, .. } => format!("Partition added to {disk}."),
            Change::DeletePartition { volume } => format!("{volume} deleted."),
            Change::Format { volume, .. } => format!("{volume} formatted."),
            Change::Mount { target, .. } => format!("Mounted at {target}."),
            Change::Unmount { target } => format!("{target} unmounted."),
            Change::SetPolicy { target, .. } => format!("Policy of {target} changed."),
            Change::AddStartup { startup } => format!("{} is mounted at every startup now.", startup.target),
            Change::RemoveStartup { .. } => "Removed from startup.".into(),
            Change::Eject { disk } => format!("{disk} ejected: it can be taken out."),
        }
    }
}

/// What the service says of a change as it is made.
#[derive(Debug, Clone, PartialEq, Eq)]
// The disk service sends every one of these; the stand-in for it, only
// `Failed`.
#[allow(dead_code)]
pub enum Heard {
    /// How far it has got: `done` of `of` (0 when not known), and what it
    /// is doing now.
    Progress { done: usize, of: usize, text: String },
    /// Made: what was done, or nothing for the change's own words.
    Done(String),
    Failed(String),
}

impl Heard {
    pub fn ends(&self) -> bool {
        !matches!(self, Heard::Progress { .. })
    }
}

/// Why nothing changes, until the service exists.
pub const WAITING: &str = "Nothing was changed: changing disks waits for Peios's disk service, which isn't built yet.";

/// Makes `change`, on a thread, telling `heard` how it goes until it ends.
pub fn request(change: Change, heard: impl Fn(Heard) + Send + Sync + 'static) {
    let heard = Arc::new(heard);
    std::thread::spawn(move || {
        let _ = change;
        heard(Heard::Failed(WAITING.into()));
    });
}

/// Makes `change` and waits for it to end: for a change asked for where
/// nothing can wait, as the permissions editor's Apply is.
pub fn make(change: Change) -> Result<String, String> {
    let (send, receive) = std::sync::mpsc::channel();
    let send = std::sync::Mutex::new(send);
    request(change, move |heard| {
        if heard.ends() {
            let _ = send.lock().map(|s| s.send(heard));
        }
    });
    match receive.recv() {
        Ok(Heard::Done(said)) => Ok(said),
        Ok(Heard::Failed(why)) => Err(why),
        _ => Err("The disk service stopped without saying how the change ended.".into()),
    }
}

/// Whether the person may change this machine's storage, or why not.
///
/// The disk service will answer this from its own access check. Until it
/// exists, the nearest answer the system gives is whether the person's
/// token holds the Manage Volumes privilege, which mounting, unmounting and
/// changing a policy need of the kernel.
pub fn may() -> Result<(), String> {
    use peios::security::Privileges;
    use peios::token::{Token, TokenAccess};
    let held = Token::open_self(false, TokenAccess::QUERY).and_then(|t| t.privileges()).map(|p| p.present.contains(Privileges::MANAGE_VOLUME));
    match held {
        Ok(true) => Ok(()),
        Ok(false) => Err("Changing disks needs the Manage Volumes privilege, which Administrators hold. You can look at everything here.".into()),
        Err(_) => Err("Whether you may change disks couldn't be found out. You can look at everything here.".into()),
    }
}

/// The filesystems mounted at every startup that someone added. The disk
/// service will keep them; until it does there are none.
pub fn startup() -> Result<Vec<Startup>, String> {
    Ok(Vec::new())
}
