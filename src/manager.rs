//! The window: this machine's disks down the side, then its mounts and
//! what is mounted at startup; the one chosen beside them.
//!
//! Everything shown is read as the person (`disks::read`). Every change is
//! a [`Change`] made by `service::request`, which until the disk service
//! exists makes nothing and says so: the forms, the questions and the
//! checks before them are all here, so the service plugs in under them.
//! Whether the person may change anything is asked of the system
//! (`service::may`); where they may not, everything is shown as it is, with
//! the reason said once.

use std::collections::HashSet;
use std::sync::{Mutex, Weak};
use std::time::Duration;

use libgxwi::settings::{self, Glyph, Nav, Section, Tile};
use libgxwi::{Facts, Fields, Live, Surface, Value};

use crate::disks::{self, Kind, Machine, PolicyKind};
use crate::service::{self, Change, Format, Heard, MountPolicy, PartitionType, Startup};
use crate::{page, permissions, words};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Disk(String),
    Mounts,
    Startup,
}

impl View {
    /// What the side knows it by.
    pub fn id(&self) -> &'static str {
        match self {
            View::Disk(name) => interned(&format!("disk:{name}")),
            View::Mounts => "mounts",
            View::Startup => "startup",
        }
    }

    fn by(id: &str) -> Option<View> {
        match id {
            "mounts" => Some(View::Mounts),
            "startup" => Some(View::Startup),
            _ => id.strip_prefix("disk:").map(|name| View::Disk(name.to_string())),
        }
    }
}

/// The side's ids and titles are `&'static str`, and disks come and go:
/// each name is kept once, for as long as the program runs. There are only
/// ever a few.
fn interned(text: &str) -> &'static str {
    static KEPT: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let kept = kept.get_or_insert_with(HashSet::new);
    if let Some(found) = kept.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    kept.insert(leaked);
    leaked
}

/// A page opened from a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Open {
    Volume(String),
    Mount(u32),
}

/// What is open under a row: a form to fill in, or a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asking {
    NewTable(String),
    Add { disk: String, start: u64, size: u64 },
    Delete(String),
    Format(String),
    Mount(String),
    AddStartup,
}

/// A descriptor being made for a form: a new filesystem's top folder, or
/// the template of a mount being made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drafts {
    pub root: Option<Vec<u8>>,
    pub template: Option<Vec<u8>>,
}

/// A change being made, or the last one, until it is put away.
pub struct Job {
    pub number: u64,
    pub change: Change,
    pub progress: Option<(usize, usize, String)>,
    pub ended: Option<Result<String, String>>,
}

impl Job {
    pub fn going(&self) -> bool {
        self.ended.is_none()
    }
}

pub struct Manager {
    pub window: Weak<Surface<Manager>>,
    pub view: View,
    pub open: Option<Open>,
    pub machine: Machine,
    /// The machine has been read at least once.
    pub read: bool,
    pub asking: Option<Asking>,
    pub drafts: Drafts,
    /// A permissions editor is open.
    pub editing: bool,
    pub job: Option<Job>,
    jobs: u64,
    close_when_done: bool,
    pub said: Option<Result<String, String>>,
}

impl Manager {
    pub fn new() -> Manager {
        Manager {
            window: Weak::new(),
            view: View::Mounts,
            open: None,
            machine: Machine::empty(),
            read: false,
            asking: None,
            drafts: Drafts::default(),
            editing: false,
            job: None,
            jobs: 0,
            close_when_done: false,
            said: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(Job::going)
    }

    /// Whether a change may be started now.
    pub fn may(&self) -> bool {
        self.machine.may.is_ok() && !self.busy()
    }

    /// Reads everything again, on a thread, and shows it when it's read.
    pub fn reread(&self) {
        let window = self.window.clone();
        std::thread::spawn(move || {
            let read = disks::read();
            if let Some(window) = window.upgrade() {
                window.update(|manager, fields| manager.heard(read, fields));
            }
        });
    }

    /// Reads everything again whenever a disk comes or goes, or something
    /// is mounted or unmounted, for as long as the window is open. Nothing
    /// tells a program when, so it looks every two seconds.
    pub fn watch(&self) {
        let window = self.window.clone();
        std::thread::spawn(move || {
            let mut seen = disks::signature();
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(open) = window.upgrade() else { return };
                let now = disks::signature();
                if now != seen {
                    seen = now;
                    open.update(|manager, _| manager.reread());
                }
            }
        });
    }

    fn heard(&mut self, read: Machine, fields: &mut Fields) {
        let first = !self.read;
        self.machine = read;
        self.read = true;
        // Each mount's policy, as its chooser shows it.
        for mount in self.machine.mounts.iter().flatten() {
            if let Ok(policy) = &mount.policy {
                fields.set(&format!("policy:{}", mount.id), policy.kind.id());
            }
        }
        // The first disk is the first thing shown, once there is one.
        if first && let Ok(disks) = &self.machine.disks && let Some(disk) = disks.first() {
            self.view = View::Disk(disk.name.clone());
        }
        // What was open may have gone: a disk taken out, a mount undone.
        if let View::Disk(name) = &self.view
            && self.machine.disk(name).is_none()
        {
            self.view = View::Mounts;
            self.open = None;
            self.asking = None;
        }
        match &self.open {
            Some(Open::Volume(name)) if self.machine.volume(name).is_none() => self.open = None,
            Some(Open::Mount(id)) if self.machine.mount(*id).is_none() => self.open = None,
            _ => {}
        }
    }

    /// Starts `change`: what it says as it goes, and how it ends, land on
    /// the page.
    fn start(&mut self, change: Change) {
        if !self.may() {
            return;
        }
        self.jobs += 1;
        let number = self.jobs;
        self.job = Some(Job { number, change: change.clone(), progress: None, ended: None });
        self.asking = None;
        self.said = None;
        let window = self.window.clone();
        service::request(change, move |heard| {
            if let Some(window) = window.upgrade() {
                window.update(|manager, _| manager.heard_job(number, heard));
            }
        });
    }

    fn heard_job(&mut self, number: u64, heard: Heard) {
        let Some(job) = self.job.as_mut().filter(|j| j.number == number) else { return };
        match heard {
            Heard::Progress { done, of, text } => job.progress = Some((done, of, text)),
            Heard::Done(said) => {
                let said = if said.is_empty() { job.change.done() } else { said };
                job.ended = Some(Ok(said.clone()));
                self.said = Some(Ok(said));
            }
            Heard::Failed(why) => {
                job.ended = Some(Err(why.clone()));
                self.said = Some(Err(why));
            }
        }
        if self.job.as_ref().is_some_and(|j| !j.going()) {
            self.job = None;
            self.drafts = Drafts::default();
            self.reread();
            if self.close_when_done
                && let Some(window) = self.window.upgrade()
            {
                window.close();
            }
        }
    }

    fn named(value: &Value, key: &str) -> String {
        value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    fn number(value: &Value, key: &str) -> u64 {
        value.get(key).and_then(Value::as_str).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    /// Opens a form or a question, with its fields as they start.
    fn ask(&mut self, asking: Asking, fields: &mut Fields) {
        if !self.may() {
            return;
        }
        self.said = None;
        self.drafts = Drafts::default();
        match &asking {
            Asking::Add { size, .. } => {
                let (amount, unit) = page::size_in_field(*size);
                fields.set("add-size", &amount);
                fields.set("add-unit", unit);
                fields.set("add-type", PartitionType::Linux.id());
                fields.set("add-name", "");
                fields.set("add-format", Format::Ext4.id());
                fields.set("add-label", "");
            }
            Asking::Format(name) => {
                let label = self.machine.volume(name).and_then(|(_, v)| v.filesystem.as_ref()?.label.clone()).unwrap_or_default();
                let format = match self.machine.volume(name).and_then(|(_, v)| v.filesystem.as_ref()).map(|f| f.kind.as_str()) {
                    Some("vfat") => Format::Fat32,
                    _ => Format::Ext4,
                };
                fields.set("format-fs", format.id());
                fields.set("format-label", &label);
            }
            Asking::Mount(name) => {
                let Some((_, volume)) = self.machine.volume(name) else { return };
                fields.set("mount-where", &page::suggested_target(volume));
                fields.set("mount-ro", if volume.read_only { "on" } else { "" });
                fields.set("mount-policy", page::default_policy(volume.filesystem.as_ref()).id());
                fields.set("mount-startup", "");
            }
            Asking::AddStartup => {
                fields.set("startup-fs", "");
                fields.set("startup-where", "");
                fields.set("startup-ro", "");
                fields.set("startup-policy", PolicyKind::DenyMissing.id());
            }
            Asking::NewTable(_) | Asking::Delete(_) => {}
        }
        self.asking = Some(asking);
    }

    fn refuse(&mut self, why: &str) {
        self.said = Some(Err(why.to_string()));
    }

    fn add_partition(&mut self, fields: &mut Fields) {
        let Some(Asking::Add { disk, start, size: free }) = self.asking.clone() else { return };
        let amount: f64 = match fields.get("add-size").trim().replace(',', ".").parse() {
            Ok(n) if n > 0.0 => n,
            _ => return self.refuse("Give the partition a size."),
        };
        let unit = if fields.get("add-unit") == "mb" { disks::MIB } else { 1024 * disks::MIB };
        // Whole mebibytes, as the table lays partitions out; a size a hair
        // over what is free, as a rounded figure is, is all of it.
        let mut size = ((amount * unit as f64) as u64) / disks::MIB * disks::MIB;
        if size > free && size - free < unit / 10 {
            size = free;
        }
        if size > free {
            return self.refuse(&format!("Only {} is free there.", words::bytes(free)));
        }
        if size < disks::MIB {
            return self.refuse("A partition is at least 1 MB.");
        }
        let kind = PartitionType::by(fields.get("add-type")).unwrap_or(PartitionType::Linux);
        let name = fields.get("add-name").trim().to_string();
        if name.chars().count() > 36 {
            return self.refuse("A partition's name is at most 36 characters.");
        }
        let format = match Format::by(fields.get("add-format")) {
            Some(format) => {
                let label = fields.get("add-label").trim().to_string();
                if let Err(why) = page::check_label(format, &label) {
                    return self.refuse(&why);
                }
                Some((format, label))
            }
            None => None,
        };
        let root = format.as_ref().filter(|(f, _)| f.keeps_permissions()).map(|_| self.drafts.root.clone().unwrap_or_else(permissions::default_root));
        self.start(Change::AddPartition { disk, start, size, kind, name, format, root });
    }

    fn format(&mut self, fields: &mut Fields) {
        let Some(Asking::Format(volume)) = self.asking.clone() else { return };
        let format = Format::by(fields.get("format-fs")).unwrap_or(Format::Ext4);
        let label = fields.get("format-label").trim().to_string();
        if let Err(why) = page::check_label(format, &label) {
            return self.refuse(&why);
        }
        let root = format.keeps_permissions().then(|| self.drafts.root.clone().unwrap_or_else(permissions::default_root));
        self.start(Change::Format { volume, format, label, root });
    }

    fn mount(&mut self, fields: &mut Fields) {
        let Some(Asking::Mount(volume)) = self.asking.clone() else { return };
        let target = fields.get("mount-where").trim().trim_end_matches('/').to_string();
        if let Err(why) = page::check_target(&target, self.machine.mounts.as_deref().unwrap_or_default()) {
            return self.refuse(&why);
        }
        let kind = PolicyKind::by(fields.get("mount-policy")).unwrap_or(PolicyKind::DenyMissing);
        let template = kind.makes().then(|| self.drafts.template.clone().unwrap_or_else(permissions::default_root));
        let policy = MountPolicy { kind, template };
        let read_only = fields.get("mount-ro") == "on";
        let at_startup = fields.get("mount-startup") == "on";
        self.start(Change::Mount { volume, target, read_only, policy, at_startup });
    }

    fn add_startup(&mut self, fields: &mut Fields) {
        let uuid = fields.get("startup-fs").to_string();
        let Some(volume) = self.machine.disks.iter().flatten().flat_map(|d| d.volumes()).find(|v| v.filesystem.as_ref().and_then(|f| f.uuid.as_deref()) == Some(uuid.as_str())) else {
            return self.refuse("Choose a filesystem.");
        };
        let shown = page::volume_name(volume);
        let target = fields.get("startup-where").trim().trim_end_matches('/').to_string();
        if let Err(why) = page::check_target(&target, &[]) {
            return self.refuse(&why);
        }
        let kind = PolicyKind::by(fields.get("startup-policy")).unwrap_or(PolicyKind::DenyMissing);
        let template = kind.makes().then(|| self.drafts.template.clone().unwrap_or_else(permissions::default_root));
        let read_only = fields.get("startup-ro") == "on";
        let startup = Startup { id: String::new(), uuid, shown, target, read_only, policy: MountPolicy { kind, template } };
        self.start(Change::AddStartup { startup });
    }

    /// Opens the permissions editor on a descriptor for a form: `root` for
    /// a new filesystem's top folder, or else a mount's template.
    fn draft(&mut self, root: bool) {
        if self.editing {
            return;
        }
        let current = if root { self.drafts.root.clone() } else { self.drafts.template.clone() }.unwrap_or_else(permissions::default_root);
        let window = self.window.clone();
        let window_done = self.window.clone();
        let what = if root { "The new filesystem's top folder" } else { "Files without permissions of their own" };
        let opened = permissions::edit(what, &current, true, None, move |sd| {
            if let Some(window) = window.upgrade() {
                window.update(|manager, _| {
                    if root {
                        manager.drafts.root = Some(sd.to_vec());
                    } else {
                        manager.drafts.template = Some(sd.to_vec());
                    }
                });
            }
            Ok(())
        }, move || {
            if let Some(window) = window_done.upgrade() {
                window.update(|manager, _| manager.editing = false);
            }
        });
        match opened {
            Ok(()) => self.editing = true,
            Err(why) => self.said = Some(Err(why)),
        }
    }

    /// Opens the permissions editor on a mount's template, which applying
    /// changes the mount's policy.
    fn edit_template(&mut self, id: u32) {
        if self.editing {
            return;
        }
        let Some(mount) = self.machine.mount(id) else { return };
        let Ok(policy) = &mount.policy else { return };
        let current = policy.template_sd.clone().unwrap_or_else(permissions::default_root);
        let may = self.may() && mount.fixed_policy().is_none() && mount.needed().is_none();
        let why = if self.machine.may.is_err() {
            Some("You may not change this mount's policy.".to_string())
        } else {
            mount.fixed_policy().or(mount.needed()).map(|why| why.to_string())
        };
        let kind = policy.kind;
        let target = mount.target.clone();
        let window_done = self.window.clone();
        let what = format!("Files on {target} without permissions of their own");
        let opened = permissions::edit(&what, &current, may, why, move |sd| {
            service::make(Change::SetPolicy { target: target.clone(), policy: MountPolicy { kind, template: Some(sd.to_vec()) } }).map(|_| ())
        }, move || {
            if let Some(window) = window_done.upgrade() {
                window.update(|manager, _| {
                    manager.editing = false;
                    manager.reread();
                });
            }
        });
        match opened {
            Ok(()) => self.editing = true,
            Err(why) => self.said = Some(Err(why)),
        }
    }

    fn nav(&self) -> Vec<Nav> {
        let mut nav = vec![Nav::Heading("Disks")];
        match &self.machine.disks {
            Ok(disks) => {
                for disk in disks {
                    let glyph = match disk.kind {
                        Kind::Fixed => Glyph::Disk,
                        Kind::Removable => Glyph::Removable,
                        Kind::Optical => Glyph::Disc,
                        Kind::Image => Glyph::File,
                    };
                    let tile = match disk.kind {
                        Kind::Fixed => Tile::Blue,
                        Kind::Removable => Tile::Orange,
                        Kind::Optical => Tile::Violet,
                        Kind::Image => Tile::Slate,
                    };
                    let now = format!("{} · {}", disk.name, words::bytes(disk.size));
                    nav.push(Nav::Section(Section { id: View::Disk(disk.name.clone()).id(), title: interned(&words::disk_title(disk)), now, glyph, tile }));
                }
            }
            Err(_) => nav.push(Nav::Section(Section { id: "disk:", title: "Disks", now: "Unavailable".into(), glyph: Glyph::Disk, tile: Tile::Blue })),
        }
        let mounts = match &self.machine.mounts {
            Ok(list) => words::count(list.iter().filter(|m| !m.hidden && m.origin == disks::Origin::Storage).count(), "filesystem", "filesystems"),
            Err(_) => "Unavailable".into(),
        };
        let startup = match &self.machine.startup {
            Ok(list) if list.is_empty() => "Only the system".to_string(),
            Ok(list) => words::count(list.len(), "filesystem", "filesystems"),
            Err(_) => "Unavailable".into(),
        };
        nav.push(Nav::Heading("This Machine"));
        nav.push(Nav::Section(Section { id: "mounts", title: "Mounts", now: mounts, glyph: Glyph::Folder, tile: Tile::Teal }));
        nav.push(Nav::Section(Section { id: "startup", title: "At Startup", now: startup, glyph: Glyph::Power, tile: Tile::Green }));
        nav
    }
}

impl Live for Manager {
    fn render(&self, facts: &Facts) -> String {
        let fields = facts.fields;
        let page = if !self.read {
            settings::head(Glyph::Disk, Tile::Blue, "Disks", "") + &settings::group("", &settings::row("Reading…", "", ""), "")
        } else {
            let looking = page::Looking { manager: self, fields };
            match (&self.open, &self.view) {
                (Some(Open::Volume(name)), _) => page::volume(&looking, name),
                (Some(Open::Mount(id)), _) => crate::mounts::mount(&looking, *id),
                (None, View::Disk(name)) => page::disk(&looking, name),
                (None, View::Mounts) => crate::mounts::mounts(&looking),
                (None, View::Startup) => crate::mounts::startup(&looking),
            }
        };
        let page = match &self.job {
            Some(job) if job.going() => {
                let (done, of, text) = job.progress.clone().unwrap_or((0, 0, job.change.doing()));
                let mut top = settings::progress(done, of, &text);
                if self.close_when_done {
                    top = settings::caution("Disk Manager closes when this change is finished.") + &top;
                }
                top + &page
            }
            _ => page,
        };
        settings::window(&self.nav(), self.view.id(), &page, &settings::status(self.said.as_ref(), ""))
    }

    fn input(&mut self, name: &str, fields: &mut Fields) {
        // A mount's policy, chosen on its page, applies as it is chosen.
        let Some(id) = name.strip_prefix("policy:").and_then(|id| id.parse::<u32>().ok()) else { return };
        let Some(mount) = self.machine.mount(id) else { return };
        let Ok(current) = &mount.policy else { return };
        let chosen = PolicyKind::by(fields.get(name));
        let current_kind = current.kind;
        if !self.may() || chosen.is_none() || chosen == Some(current_kind) {
            fields.set(name, current_kind.id());
            return;
        }
        let kind = chosen.unwrap_or(current_kind);
        let template = kind.makes().then(|| current.template_sd.clone().unwrap_or_else(permissions::default_root));
        let target = mount.target.clone();
        // Until the policy is changed, the field shows what it is.
        fields.set(name, current_kind.id());
        self.start(Change::SetPolicy { target, policy: MountPolicy { kind, template } });
    }

    fn event(&mut self, name: &str, value: &Value, fields: &mut Fields) {
        match name {
            "section" => {
                if let Some(view) = value.get("section").and_then(Value::as_str).and_then(View::by) {
                    self.view = view;
                    self.open = None;
                    self.asking = None;
                    self.said = None;
                    self.reread();
                }
            }
            "open-volume" => {
                self.open = Some(Open::Volume(Self::named(value, "volume")));
                self.asking = None;
                self.said = None;
            }
            "open-mount" => {
                self.open = Some(Open::Mount(Self::number(value, "mount") as u32));
                self.asking = None;
                self.said = None;
            }
            "back" => {
                self.open = None;
                self.asking = None;
                self.said = None;
            }
            "ask-new-table" => self.ask(Asking::NewTable(Self::named(value, "disk")), fields),
            "ask-add" => {
                let asking = Asking::Add { disk: Self::named(value, "disk"), start: Self::number(value, "start"), size: Self::number(value, "size") };
                self.ask(asking, fields);
            }
            "ask-delete" => self.ask(Asking::Delete(Self::named(value, "volume")), fields),
            "ask-format" => self.ask(Asking::Format(Self::named(value, "volume")), fields),
            "ask-mount" => self.ask(Asking::Mount(Self::named(value, "volume")), fields),
            "ask-add-startup" => self.ask(Asking::AddStartup, fields),
            "cancel" => {
                self.asking = None;
                self.drafts = Drafts::default();
                self.said = None;
            }
            "new-table" => {
                if let Some(Asking::NewTable(disk)) = self.asking.clone() {
                    self.start(Change::NewTable { disk });
                }
            }
            "delete" => {
                if let Some(Asking::Delete(volume)) = self.asking.clone() {
                    self.start(Change::DeletePartition { volume });
                }
            }
            "add-partition" => self.add_partition(fields),
            "format" => self.format(fields),
            "mount" => self.mount(fields),
            "add-startup" => self.add_startup(fields),
            "unmount" => {
                if let Some(mount) = self.machine.mount(Self::number(value, "mount") as u32) {
                    let target = mount.target.clone();
                    self.start(Change::Unmount { target });
                }
            }
            "eject" => {
                let disk = Self::named(value, "disk");
                self.start(Change::Eject { disk });
            }
            "remove-startup" => {
                let id = Self::named(value, "id");
                self.start(Change::RemoveStartup { id });
            }
            "permissions-root" => self.draft(true),
            "permissions-template" => self.draft(false),
            "permissions-mount" => self.edit_template(Self::number(value, "mount") as u32),
            _ => {}
        }
    }

    fn closing(&mut self, _fields: &mut Fields) -> bool {
        // A change under way finishes whatever happens; the window stays to
        // show how it ended, then goes.
        if self.busy() {
            self.close_when_done = true;
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_section_names_a_view() {
        for view in [View::Mounts, View::Startup, View::Disk("vdb".into())] {
            assert_eq!(View::by(view.id()), Some(view.clone()));
        }
        assert_eq!(View::by("nonsense"), None);
    }

    #[test]
    fn a_name_is_kept_once() {
        assert!(std::ptr::eq(interned("disk:sda"), interned("disk:sda")));
    }
}
