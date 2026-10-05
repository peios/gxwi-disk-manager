//! A disk's page, with its partitions and the space between them, and a
//! partition's (or a whole disk's filesystem's) page, with the forms and
//! questions that change them.

use libgxwi::settings::{self, Glyph, Kind as Button, More, Tile, Tone, Width};
use libgxwi::{Fields, escape};

use crate::disks::{Disk, Filesystem, Kind, Mount, PolicyKind, Stretch, Volume, MIB};
use crate::manager::{Asking, Manager};
use crate::service::{Format, PartitionType};
use crate::words;

/// What a page is drawn from.
pub struct Looking<'a> {
    pub manager: &'a Manager,
    pub fields: &'a Fields,
}

impl Looking<'_> {
    /// Whether changes are offered at all: the person may make them.
    pub fn offered(&self) -> bool {
        self.manager.machine.may.is_ok()
    }

    /// Whether a change may be started now.
    pub fn may(&self) -> bool {
        self.manager.may()
    }

    fn asking(&self) -> Option<&Asking> {
        self.manager.asking.as_ref()
    }

    /// The banner for someone who may change nothing, said once a page.
    pub fn banner(&self) -> String {
        match &self.manager.machine.may {
            Err(why) if !why.is_empty() => settings::banner(why),
            _ => String::new(),
        }
    }
}

/// A size as a form's field starts with it: what is free, as the page says
/// it, in gigabytes to a tenth or in megabytes below one. Rounded up, it is
/// still all of it: the form takes a hair over what is free as everything.
pub fn size_in_field(size: u64) -> (String, &'static str) {
    let gib = 1024 * MIB;
    if size >= gib {
        let tenths = (size * 10 + gib / 2) / gib;
        let text = if tenths.is_multiple_of(10) { format!("{}", tenths / 10) } else { format!("{}.{}", tenths / 10, tenths % 10) };
        (text, "gb")
    } else {
        ((size / MIB).to_string(), "mb")
    }
}

/// Where a filesystem is offered to be mounted: under /mnt, by its label.
pub fn suggested_target(volume: &Volume) -> String {
    let label = volume.filesystem.as_ref().and_then(|f| f.label.as_deref()).unwrap_or(&volume.name);
    let clean: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    let clean = clean.trim_matches('-');
    format!("/mnt/{}", if clean.is_empty() { volume.name.as_str() } else { clean })
}

/// Whether files on a filesystem can carry permissions of their own: the
/// policy offered first follows from it.
pub fn keeps_permissions(fs: Option<&Filesystem>) -> bool {
    fs.is_some_and(|f| ["ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs"].contains(&f.kind.as_str()))
}

/// The policy a mount is offered with: as the kernel would choose for that
/// filesystem.
pub fn default_policy(fs: Option<&Filesystem>) -> PolicyKind {
    if keeps_permissions(fs) { PolicyKind::DenyMissing } else { PolicyKind::Ephemeral }
}

/// The policies a filesystem can be mounted with. Saving a made-up
/// descriptor needs somewhere to save it.
pub fn policies(fs: Option<&Filesystem>) -> Vec<PolicyKind> {
    if keeps_permissions(fs) { PolicyKind::CHOSEN.to_vec() } else { vec![PolicyKind::Ephemeral, PolicyKind::DenyMissing] }
}

pub fn check_label(format: Format, label: &str) -> Result<(), String> {
    if label.chars().count() > format.label_limit() {
        return Err(format!("A label on {} is at most {} characters.", words::format(format), format.label_limit()));
    }
    if format == Format::Fat32 && label.chars().any(|c| !c.is_ascii() || "\"*/:<>?\\|.+,;=[]".contains(c)) {
        return Err("A FAT32 label is letters, digits, spaces and simple punctuation.".into());
    }
    Ok(())
}

/// Whether a filesystem can be mounted at `target`: a folder of its own,
/// away from the system's.
pub fn check_target(target: &str, mounts: &[Mount]) -> Result<(), String> {
    if !target.starts_with('/') {
        return Err("Mount it at a full path, such as /mnt/data.".into());
    }
    if target == "/" || ["/proc", "/sys", "/dev", "/run", "/usr", "/lcl", "/system", "/boot"].iter().any(|p| target == *p || target.starts_with(&format!("{p}/"))) {
        return Err(format!("{target} is the system's: choose a folder of your own, such as one under /mnt."));
    }
    if mounts.iter().any(|m| !m.hidden && m.target == target) {
        return Err(format!("Something is mounted at {target} already."));
    }
    Ok(())
}

/// A filesystem, as a list of them names it: its label, or where it is.
pub fn volume_name(volume: &Volume) -> String {
    let label = volume.filesystem.as_ref().and_then(|f| f.label.clone());
    match label {
        Some(label) => format!("{label} ({})", volume.name),
        None => format!("{} ({})", words::volume_title(volume), volume.name),
    }
}

fn disk_glyph(disk: &Disk) -> Glyph {
    match disk.kind {
        Kind::Fixed => Glyph::Disk,
        Kind::Removable => Glyph::Removable,
        Kind::Optical => Glyph::Disc,
        Kind::Image => Glyph::File,
    }
}

/// The colour a filesystem is drawn in, on the bar and beside its row.
fn colour(volume: &Volume) -> &'static str {
    match volume.filesystem.as_ref().map(|f| f.kind.as_str()) {
        Some("ext2" | "ext3" | "ext4" | "xfs" | "btrfs" | "f2fs") => "c-linux",
        Some("vfat" | "exfat" | "ntfs" | "ntfs3") => "c-fat",
        Some("swap") => "c-swap",
        Some(_) => "c-other",
        None => "c-none",
    }
}

fn swatch(volume: &Volume) -> String {
    format!(r#"<span class="dm-swatch {}" aria-hidden="true"></span>"#, colour(volume))
}

/// The disk drawn as a bar: each partition and each free stretch as wide as
/// its share, with a narrowest width so the smallest can still be seen
/// and pressed. Widths are classes: a page may carry no style attribute.
fn bar(disk: &Disk, l: &Looking) -> String {
    let stretches = disk.stretches();
    if stretches.is_empty() {
        return String::new();
    }
    let share = |size: u64| ((size as f64 / disk.size.max(1) as f64) * 100.0).round().clamp(1.0, 100.0) as u32;
    let mut out = String::new();
    for stretch in &stretches {
        match stretch {
            Stretch::Used(v) => out.push_str(&format!(
                r#"<button type="button" class="dm-seg g{} {}" fx-click="open-volume" fx-value-volume="{}" title="{}"><b>{}</b><small>{}</small></button>"#,
                share(v.size),
                colour(v),
                escape(&v.name),
                escape(&format!("{} · {}", words::volume_title(v), words::volume_line(v))),
                escape(&words::volume_title(v)),
                escape(&words::bytes(v.size)),
            )),
            Stretch::Free { start, size } => {
                let label = format!("Free · {}", words::bytes(*size));
                if l.offered() && !disk.read_only {
                    out.push_str(&format!(
                        r#"<button type="button" class="dm-seg g{} free" fx-click="ask-add" fx-value-disk="{}" fx-value-start="{start}" fx-value-size="{size}" title="{} — add a partition here"{}><b>Free</b><small>{}</small></button>"#,
                        share(*size),
                        escape(&disk.name),
                        escape(&label),
                        if l.may() { "" } else { " disabled" },
                        escape(&words::bytes(*size)),
                    ));
                } else {
                    out.push_str(&format!(r#"<span class="dm-seg g{} free" title="{}"><b>Free</b><small>{}</small></span>"#, share(*size), escape(&label), escape(&words::bytes(*size))));
                }
            }
        }
    }
    format!(r#"<div class="dm-bar" role="group" aria-label="{}">{out}</div>"#, escape(&format!("What is on {}", disk.name)))
}

/// A line on where a volume is mounted, or that it isn't.
fn mounted_line(volume: &Volume, mounts: &[&Mount]) -> String {
    let places: Vec<&str> = mounts.iter().filter(|m| !m.hidden).map(|m| m.target.as_str()).collect();
    let places: Vec<&str> = if places.is_empty() { volume.mounted.iter().map(String::as_str).collect() } else { places };
    match places.as_slice() {
        [] if volume.filesystem.is_none() => "Not formatted".into(),
        [] => "Not mounted".into(),
        [one] => format!("Mounted at {one}"),
        many => format!("Mounted at {}", many.join(", ")),
    }
}

fn volume_row(volume: &Volume, l: &Looking) -> String {
    let mounts = l.manager.machine.mounts_of(volume);
    let mut title = words::volume_title(volume);
    if let Some(kind) = volume.partition.as_ref().and_then(|p| p.kind.as_deref()) {
        title.push_str(&format!(" — {}", words::partition_type(kind)));
    }
    let what = words::volume_line(volume);
    let place = mounted_line(volume, &mounts);
    // Without a filesystem, the first line has said all there is.
    let lines: Vec<(&str, bool)> = if volume.filesystem.is_some() { vec![(&what, false), (&place, false)] } else { vec![(&what, false)] };
    settings::link(&swatch(volume), &title, &lines, "", "open-volume", &[("volume", &volume.name)])
}

pub fn disk(l: &Looking, name: &str) -> String {
    let machine = &l.manager.machine;
    let Some(disk) = machine.disk(name) else {
        return settings::head(Glyph::Disk, Tile::Blue, "Disk", "") + &settings::group("", &settings::row("This disk isn't attached any more", "", ""), "");
    };
    let title = words::disk_title(disk);
    let pill = if disk.kind == Kind::Optical && disk.size == 0 {
        settings::pill("No Disc", Tone::Plain)
    } else if !disk.readable {
        settings::pill("Not Readable", Tone::Plain)
    } else if machine.holds_system(disk) {
        settings::pill("System", Tone::Good)
    } else if disk.read_only {
        settings::pill("Read-Only", Tone::Plain)
    } else {
        match &disk.table {
            Some(t) => settings::pill(&words::table(t), Tone::Plain),
            None if disk.whole.is_some() => settings::pill("No Partition Table", Tone::Plain),
            None => settings::pill("Empty", Tone::Warn),
        }
    };
    let about = format!("{} · {}", words::disk_line(disk), disk.path);
    let mut page = settings::hero(&settings::hero_title(disk_glyph(disk), Tile::Blue, &title, &about), &pill);
    page.push_str(&l.banner());
    if !disk.readable {
        page.push_str(&settings::caution(
            "Only SYSTEM and Administrators may open a disk, so its partition table and filesystems can't be read. Its partitions are shown as the kernel lists them, and what is mounted as the mount table says.",
        ));
    }
    if machine.holds_system(disk) && l.offered() {
        page.push_str(&settings::caution("Peios runs from this disk: what it runs from can't be changed while it does."));
    }
    page.push_str(&bar(disk, l));

    // What is on it: partitions and the space between them in order, and
    // a filesystem on the whole disk where there is one.
    let mut rows = String::new();
    for stretch in disk.stretches() {
        match stretch {
            Stretch::Used(volume) => rows.push_str(&volume_row(volume, l)),
            Stretch::Free { start, size } => {
                let control = if l.offered() && !disk.read_only {
                    settings::button("Add Partition…", "ask-add", &[("disk", &disk.name), ("start", &start.to_string()), ("size", &size.to_string())], Button::Plain, l.may())
                } else {
                    String::new()
                };
                let at = format!("Starts {} in", words::bytes(start));
                rows.push_str(&settings::item(r#"<span class="dm-swatch free" aria-hidden="true"></span>"#, &format!("Free Space — {}", words::bytes(size)), &[(&at, false)], &control));
                if let Some(Asking::Add { disk: d, start: s, size: free }) = l.asking()
                    && *d == disk.name
                    && *s == start
                {
                    rows.push_str(&add_form(*free, l));
                }
            }
        }
    }
    if let Some(whole) = &disk.whole {
        rows.push_str(&volume_row(whole, l));
    }
    if rows.is_empty() {
        rows = if disk.kind == Kind::Optical && disk.size == 0 {
            settings::row("No disc", "Put a disc in the drive to see what is on it.", "")
        } else if !disk.readable {
            settings::row("Not known", "It has no partitions the kernel lists, and whether it holds a filesystem couldn't be read.", "")
        } else {
            settings::row("Nothing on it", "It has no partition table and no filesystem. Erase it to give it an empty partition table, then add partitions.", "")
        };
    }
    let foot = if disk.table.as_deref() == Some("dos") && l.offered() {
        settings::note("This disk has an MBR partition table, which Peios reads but doesn't change. Erase it to give it a GPT table.")
    } else {
        String::new()
    };
    let title = if disk.table.is_some() || !disk.partitions.is_empty() { "Partitions" } else { "Contents" };
    page.push_str(&settings::group(title, &rows, &foot));

    page.push_str(&disk_facts(disk, l));
    page
}

fn disk_facts(disk: &Disk, l: &Looking) -> String {
    let machine = &l.manager.machine;
    let mut rows = settings::fact("Device", &disk.path, true);
    if let Some(model) = &disk.model {
        rows.push_str(&settings::fact("Model", model, false));
    }
    if let Some(serial) = &disk.serial {
        rows.push_str(&settings::fact("Serial Number", serial, true));
    }
    if let Some(t) = &disk.transport {
        rows.push_str(&settings::fact("Connection", &words::transport(t), false));
    }
    if let Some(file) = &disk.backing {
        rows.push_str(&settings::fact("Image File", file, true));
    }
    if disk.size > 0 {
        rows.push_str(&settings::fact("Size", &format!("{} ({})", words::bytes(disk.size), words::exact(disk.size)), false));
        rows.push_str(&settings::fact("Sector Size", &format!("{} bytes", disk.sector), false));
    }
    match (&disk.table, &disk.table_uuid) {
        (Some(t), Some(id)) => rows.push_str(&settings::fact("Partition Table", &format!("{} · {id}", words::table(t)), false)),
        (Some(t), None) => rows.push_str(&settings::fact("Partition Table", &words::table(t), false)),
        (None, _) if !disk.readable => rows.push_str(&settings::fact("Partition Table", "Not readable", false)),
        (None, _) => rows.push_str(&settings::fact("Partition Table", "None", false)),
    }
    if l.offered() {
        let busy = machine.disk_busy(disk);
        if disk.kind == Kind::Removable || disk.kind == Kind::Optical {
            let about = if machine.holds_system(disk) { "Peios runs from it, so it stays in." } else { "Unmount everything on it, so it can be taken out safely." };
            rows.push_str(&settings::row("Eject", about, &settings::button("Eject", "eject", &[("disk", &disk.name)], Button::Plain, l.may() && !machine.holds_system(disk))));
        }
        if disk.kind != Kind::Optical {
            let why = if disk.read_only { Some("The disk is read-only.".to_string()) } else { busy };
            let about = why.clone().unwrap_or_else(|| "Give it a new, empty GPT partition table. Everything on it is lost.".into());
            let asking = matches!(l.asking(), Some(Asking::NewTable(d)) if *d == disk.name);
            rows.push_str(&settings::row("Erase Disk", &about, &settings::button("Erase Disk…", "ask-new-table", &[("disk", &disk.name)], Button::Danger, l.may() && why.is_none() && !asking)));
            if asking {
                let question = format!(
                    "<p>Erase {}? Every partition on it, and everything on them, is lost. It is left with an empty GPT partition table.</p>{}",
                    escape(&disk.name),
                    settings::actions(&format!("{}{}", settings::focused_button("Cancel", "cancel", Button::Plain), settings::button("Erase Disk", "new-table", &[], Button::Danger, true)))
                );
                rows.push_str(&settings::more(More::Asking, &question));
            }
        }
    }
    settings::group("This Disk", &rows, "")
}

/// The form to add a partition in free space of `free` bytes.
fn add_form(free: u64, l: &Looking) -> String {
    let f = l.fields;
    let units = [("gb".to_string(), "GB".to_string()), ("mb".to_string(), "MB".to_string())];
    let types: Vec<(String, String)> = PartitionType::ALL.iter().map(|t| (t.id().to_string(), words::new_partition_type(*t).to_string())).collect();
    let mut formats: Vec<(String, String)> = Format::ALL.iter().map(|x| (x.id().to_string(), words::format(*x).to_string())).collect();
    formats.push(("none".into(), "Leave Unformatted".into()));
    let mut rows = settings::row(
        "Size",
        &format!("Up to {}.", words::bytes(free)),
        &format!(r#"<span class="dm-size">{}{}</span>"#, settings::text("add-size", "Size", "text", Width::Short, true, r#"inputmode="decimal" autocomplete="off""#), settings::select("add-unit", "Unit", &units, true)),
    );
    rows.push_str(&settings::row("Type", "What it is for, as the partition table records it.", &settings::select("add-type", "Type", &types, true)));
    rows.push_str(&settings::row("Name", "Optional. The partition table's name for it.", &settings::text("add-name", "Name", "text", Width::Normal, true, r#"autocomplete="off" spellcheck="false""#)));
    rows.push_str(&settings::row("Format As", "", &settings::select("add-format", "Format as", &formats, true)));
    if let Some(format) = Format::by(f.get("add-format")) {
        rows.push_str(&settings::row("Label", &format!("Optional. Up to {} characters.", format.label_limit()), &settings::text("add-label", "Label", "text", Width::Normal, true, r#"autocomplete="off" spellcheck="false""#)));
        if format.keeps_permissions() {
            rows.push_str(&root_row(l));
        }
    }
    let buttons = settings::actions(&format!("{}{}", settings::button("Cancel", "cancel", &[], Button::Plain, true), settings::submit("Add Partition", Button::Primary, l.may())));
    settings::more(More::Form, &format!(r#"<form fx-submit="add-partition">{rows}{buttons}</form>"#))
}

/// The row for a new filesystem's top-folder permissions.
fn root_row(l: &Looking) -> String {
    let said = if l.manager.drafts.root.is_some() { "Set here." } else { "SYSTEM and Administrators have full control; everyone may read." };
    settings::row("Permissions of Its Top Folder", said, &settings::button("Permissions…", "permissions-root", &[], Button::Plain, !l.manager.editing))
}

/// Why a volume can't be erased (formatted or deleted) now, or nothing.
fn erase_blocked(disk: &Disk, volume: &Volume, l: &Looking) -> Option<String> {
    let mounts = l.manager.machine.mounts_of(volume);
    if mounts.iter().any(|m| m.needed().is_some()) {
        return Some("Peios runs from it.".into());
    }
    if disk.read_only || volume.read_only {
        return Some("The disk is read-only.".into());
    }
    if !mounts.is_empty() || !volume.mounted.is_empty() {
        return Some("It is mounted: unmount it first.".into());
    }
    None
}

pub fn volume(l: &Looking, name: &str) -> String {
    let machine = &l.manager.machine;
    let Some((disk, volume)) = machine.volume(name) else {
        return settings::back("Back", "back") + &settings::group("", &settings::row("It isn't there any more", "", ""), "");
    };
    let mounts = machine.mounts_of(volume);
    let mut page = settings::back(&words::disk_title(disk), "back");
    let pill = match (&volume.filesystem, mounts.iter().any(|m| !m.hidden) || !volume.mounted.is_empty()) {
        (None, _) if !volume.readable => settings::pill("Not Readable", Tone::Plain),
        (None, _) => settings::pill("Not Formatted", Tone::Warn),
        (Some(_), true) => settings::pill("Mounted", Tone::Good),
        (Some(_), false) => settings::pill("Not Mounted", Tone::Plain),
    };
    let title = match &volume.partition {
        Some(_) => format!("{} of {}", words::volume_title(volume), disk.name),
        None => format!("{} ({})", words::disk_title(disk), disk.name),
    };
    page.push_str(&settings::hero(&settings::hero_title(disk_glyph(disk), Tile::Blue, &title, &format!("{} · {}", words::volume_line(volume), volume.path)), &pill));
    page.push_str(&l.banner());
    let blocked = erase_blocked(disk, volume, l);

    // The filesystem: what it is, how full, and formatting it again.
    let mut rows = String::new();
    if let Some(fs) = &volume.filesystem {
        let kind = match &fs.version {
            Some(v) if fs.kind != "vfat" => format!("{} ({v})", words::filesystem(fs)),
            _ => words::filesystem(fs),
        };
        rows.push_str(&settings::fact("Type", &kind, false));
        let unknown = if volume.readable { "None" } else { "Not readable" };
        rows.push_str(&settings::fact("Label", fs.label.as_deref().unwrap_or(unknown), false));
        if let Some(uuid) = &fs.uuid {
            rows.push_str(&settings::fact("UUID", uuid, true));
        }
        if let Some((total, free)) = mounts.iter().find_map(|m| m.usage) {
            rows.push_str(&usage_row(total, free));
        }
    }
    let formattable = l.offered() && disk.kind != Kind::Optical;
    let asking = matches!(l.asking(), Some(Asking::Format(v)) if *v == volume.name);
    let format = settings::button("Format…", "ask-format", &[("volume", &volume.name)], Button::Plain, l.may() && blocked.is_none() && !asking);
    if volume.filesystem.is_none() && !volume.readable {
        rows.push_str(&settings::row("Contents Unknown", "Whether it holds a filesystem couldn't be read: only SYSTEM and Administrators may open a disk.", ""));
    } else if volume.filesystem.is_none() {
        // One row: that it has none, and what gives it one.
        let about = if formattable { blocked.clone().unwrap_or_else(|| "Format it to use it.".into()) } else { "Nothing on it can be used until it has a filesystem.".into() };
        rows.push_str(&settings::row("Not Formatted", &about, if formattable { &format } else { "" }));
    } else if formattable {
        let about = blocked.clone().unwrap_or_else(|| "Erase it and make a new, empty filesystem.".into());
        rows.push_str(&settings::row("Format", &about, &format));
    }
    if asking {
        rows.push_str(&format_form(volume, l));
    }
    page.push_str(&settings::group("Filesystem", &rows, ""));

    // Where it is mounted, and mounting it.
    if volume.filesystem.as_ref().is_some_and(|f| !["swap", "crypto_LUKS", "LVM2_member", "linux_raid_member"].contains(&f.kind.as_str())) {
        let mut rows = String::new();
        for mount in mounts.iter().filter(|m| !m.hidden) {
            rows.push_str(&mount_item(mount, l));
        }
        let asking = matches!(l.asking(), Some(Asking::Mount(v)) if *v == volume.name);
        let mount = if l.offered() { settings::button("Mount…", "ask-mount", &[("volume", &volume.name)], Button::Plain, l.may() && !asking) } else { String::new() };
        if rows.is_empty() {
            // One row: that it isn't, and what mounts it.
            let about = if l.offered() { "Mount it at a folder to use what is on it." } else { "What is on it can't be used until it is mounted." };
            rows.push_str(&settings::row("Not Mounted", about, &mount));
        } else if l.offered() {
            rows.push_str(&settings::row("Mount Elsewhere", "Mount it at another folder as well.", &mount));
        }
        if asking {
            rows.push_str(&mount_form(volume, l));
        }
        page.push_str(&settings::group("Mounts", &rows, ""));
    }

    // The partition itself, and deleting it.
    if let Some(p) = &volume.partition {
        let mut rows = settings::fact("Number", &p.number.to_string(), false);
        if let Some(kind) = &p.kind {
            rows.push_str(&settings::fact("Type", &words::partition_type(kind), false));
        }
        rows.push_str(&settings::fact("Name", p.name.as_deref().unwrap_or(if volume.readable { "None" } else { "Not readable" }), false));
        if let Some(uuid) = &p.uuid {
            rows.push_str(&settings::fact("Unique ID", uuid, true));
        }
        rows.push_str(&settings::fact("Starts At", &format!("{} in ({})", words::bytes(p.start), words::exact(p.start)), false));
        rows.push_str(&settings::fact("Size", &words::exact(volume.size), false));
        if l.offered() && disk.table.as_deref() == Some("gpt") {
            let asking = matches!(l.asking(), Some(Asking::Delete(v)) if *v == volume.name);
            let about = blocked.clone().unwrap_or_else(|| "Remove it from the partition table. What is on it is lost.".into());
            rows.push_str(&settings::row("Delete Partition", &about, &settings::button("Delete…", "ask-delete", &[("volume", &volume.name)], Button::Danger, l.may() && blocked.is_none() && !asking)));
            if asking {
                let question = format!(
                    "<p>Delete {} of {}? Everything on it is lost, and its space becomes free.</p>{}",
                    escape(&words::volume_title(volume)),
                    escape(&disk.name),
                    settings::actions(&format!("{}{}", settings::focused_button("Cancel", "cancel", Button::Plain), settings::button("Delete Partition", "delete", &[], Button::Danger, true)))
                );
                rows.push_str(&settings::more(More::Asking, &question));
            }
        }
        page.push_str(&settings::group("Partition", &rows, ""));
    }
    page
}

/// How full a filesystem is: a bar, and the figures.
pub fn usage_row(total: u64, free: u64) -> String {
    let used = total.saturating_sub(free);
    let control = format!(
        r#"<span class="dm-usage"><meter min="0" max="{total}" value="{used}" low="{}" high="{}" optimum="0"></meter><span>{} free of {}</span></span>"#,
        total / 100 * 75,
        total / 100 * 90,
        escape(&words::bytes(free)),
        escape(&words::bytes(total))
    );
    settings::row("Space", "", &control)
}

/// One mount of a filesystem, in its list, with unmounting it.
pub fn mount_item(mount: &Mount, l: &Looking) -> String {
    let how = if mount.read_only { "Read-only" } else { "Read and write" };
    let policy = match &mount.policy {
        Ok(p) => words::policy(p.kind).to_string(),
        Err(_) => "Policy unknown".into(),
    };
    let line = format!("{how} · Files without permissions: {policy}");
    let mut control = settings::button("Details", "open-mount", &[("mount", &mount.id.to_string())], Button::Quiet, true);
    if l.offered() && mount.needed().is_none() {
        control.push_str(&settings::button("Unmount", "unmount", &[("mount", &mount.id.to_string())], Button::Plain, l.may()));
    }
    settings::item(&settings::icon(Glyph::Folder, Tile::Teal), &mount.target, &[(&line, false)], &control)
}

fn format_form(volume: &Volume, l: &Looking) -> String {
    let formats: Vec<(String, String)> = Format::ALL.iter().map(|x| (x.id().to_string(), words::format(*x).to_string())).collect();
    let format = Format::by(l.fields.get("format-fs")).unwrap_or(Format::Ext4);
    let mut rows = settings::row("Filesystem", "ext4 keeps permissions on every file. FAT32 is read by every system, and keeps none.", &settings::select("format-fs", "Filesystem", &formats, true));
    rows.push_str(&settings::row("Label", &format!("Optional. Up to {} characters.", format.label_limit()), &settings::text("format-label", "Label", "text", Width::Normal, true, r#"autocomplete="off" spellcheck="false""#)));
    if format.keeps_permissions() {
        rows.push_str(&root_row(l));
    }
    let warning = format!(r#"<p class="dm-warn">Everything on {} is erased.</p>"#, escape(&volume.path));
    let buttons = settings::actions(&format!("{}{}", settings::button("Cancel", "cancel", &[], Button::Plain, true), settings::submit("Erase and Format", Button::Danger, l.may())));
    settings::more(More::Form, &format!(r#"<form fx-submit="format">{rows}{warning}{buttons}</form>"#))
}

/// The policy chooser and its template, as a mount form has them.
pub fn policy_rows(field: &str, choices: &[PolicyKind], l: &Looking) -> String {
    let options: Vec<(String, String)> = choices.iter().map(|k| (k.id().to_string(), words::policy(*k).to_string())).collect();
    let chosen = PolicyKind::by(l.fields.get(field)).unwrap_or(choices[0]);
    let mut rows = settings::row("Files Without Permissions", words::policy_about(chosen), &settings::select(field, "Files without permissions", &options, true));
    if chosen.makes() {
        let said = if l.manager.drafts.template.is_some() { "Set here." } else { "Where a file's folder has none to pass on: SYSTEM and Administrators have full control; everyone may read." };
        rows.push_str(&settings::row("Permissions They Are Given", said, &settings::button("Permissions…", "permissions-template", &[], Button::Plain, !l.manager.editing)));
    }
    rows
}

fn mount_form(volume: &Volume, l: &Looking) -> String {
    let mut rows = settings::row("Mount At", "A folder, made if it isn't there.", &settings::text("mount-where", "Mount at", "text", Width::Wide, true, r#"autocomplete="off" spellcheck="false""#).replacen("st-input", "st-input mono", 1));
    rows.push_str(&settings::row("Read-Only", if volume.read_only { "The disk is read-only." } else { "Nothing on it can be changed while it is mounted." }, &settings::switch("mount-ro", "Read-only", !volume.read_only)));
    rows.push_str(&policy_rows("mount-policy", &policies(volume.filesystem.as_ref()), l));
    rows.push_str(&settings::row("Mount at Every Startup", "Mounted here again each time this machine starts.", &settings::switch("mount-startup", "Mount at every startup", true)));
    let buttons = settings::actions(&format!("{}{}", settings::button("Cancel", "cancel", &[], Button::Plain, true), settings::submit("Mount", Button::Primary, l.may())));
    settings::more(More::Form, &format!(r#"<form fx-submit="mount">{rows}{buttons}</form>"#))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_starts_with_what_fits() {
        assert_eq!(size_in_field(2_684_354_560), ("2.5".to_string(), "gb"));
        assert_eq!(size_in_field(2_549 * MIB), ("2.5".to_string(), "gb"));
        assert_eq!(size_in_field(4 * 1024 * MIB), ("4".to_string(), "gb"));
        assert_eq!(size_in_field(87 * MIB), ("87".to_string(), "mb"));
    }

    #[test]
    fn a_mount_goes_in_a_folder_of_its_own() {
        assert!(check_target("/mnt/data", &[]).is_ok());
        assert!(check_target("mnt/data", &[]).is_err());
        assert!(check_target("/", &[]).is_err());
        assert!(check_target("/proc/x", &[]).is_err());
        assert!(check_target("/usr", &[]).is_err());
    }

    #[test]
    fn labels_fit_their_filesystem() {
        assert!(check_label(Format::Ext4, "a-sixteen-chars!").is_ok());
        assert!(check_label(Format::Ext4, "seventeen-chars!!").is_err());
        assert!(check_label(Format::Fat32, "STICK").is_ok());
        assert!(check_label(Format::Fat32, "TWELVE CHARS").is_err());
        assert!(check_label(Format::Fat32, "A/B").is_err());
    }

    #[test]
    fn a_mount_is_offered_at_its_label() {
        let v = Volume {
            name: "vdb2".into(),
            path: "/dev/vdb2".into(),
            size: 0,
            read_only: false,
            readable: true,
            filesystem: Some(Filesystem { kind: "ext4".into(), version: None, label: Some("My Data".into()), uuid: None }),
            mounted: Vec::new(),
            partition: None,
        };
        assert_eq!(suggested_target(&v), "/mnt/my-data");
        assert_eq!(default_policy(v.filesystem.as_ref()), PolicyKind::DenyMissing);
        assert_eq!(policies(None), vec![PolicyKind::Ephemeral, PolicyKind::DenyMissing]);
    }
}
