//! Everything mounted, a mount's own page with its policy, and what is
//! mounted at every startup.

use libgxwi::escape;
use libgxwi::settings::{self, Glyph, Kind as Button, More, Tile, Tone, Width};

use crate::disks::{Mount, Origin, PolicyKind};
use crate::manager::Asking;
use crate::page::{self, Looking};
use crate::words;

fn origin_title(origin: Origin) -> &'static str {
    match origin {
        Origin::Storage => "Filesystems on Disks",
        Origin::System => "The System",
        Origin::Shared => "Shared Folders",
        Origin::Kernel => "Kernel and Memory",
    }
}

fn origin_glyph(origin: Origin) -> (Glyph, Tile) {
    match origin {
        Origin::Storage => (Glyph::Disk, Tile::Blue),
        Origin::System => (Glyph::Layers, Tile::Violet),
        Origin::Shared => (Glyph::Server, Tile::Teal),
        Origin::Kernel => (Glyph::Terminal, Tile::Slate),
    }
}

/// What a mount is from, in a line.
fn from_line(mount: &Mount) -> String {
    match mount.origin {
        Origin::Storage => format!("{} on {}", words::mount_kind(&mount.kind), mount.source),
        Origin::System if mount.kind == "stratafs" => "StrataFS view".into(),
        _ if mount.source.is_empty() || mount.source == "none" || mount.source == mount.kind => words::mount_kind(&mount.kind),
        _ => format!("{} from {}", words::mount_kind(&mount.kind), mount.source),
    }
}

fn mount_link(mount: &Mount) -> String {
    let (glyph, tile) = origin_glyph(mount.origin);
    let mut second = if mount.read_only { "Read-only".to_string() } else { "Read and write".to_string() };
    if let Ok(p) = &mount.policy {
        second.push_str(&format!(" · {}", words::policy(p.kind)));
    }
    if mount.hidden {
        second = "Covered by a later mount".into();
    }
    // What is free on something read-only says nothing: what it holds does.
    let aside = match mount.usage {
        Some((total, _)) if mount.read_only => format!(r#"<span class="dm-aside">{}</span>"#, escape(&words::bytes(total))),
        Some((total, free)) => format!(r#"<span class="dm-aside">{} free</span>"#, escape(&words::bytes(free.min(total)))),
        None => String::new(),
    };
    let first = from_line(mount);
    settings::link(&settings::icon(glyph, tile), &mount.target, &[(&first, false), (&second, false)], &aside, "open-mount", &[("mount", &mount.id.to_string())])
}

pub fn mounts(l: &Looking) -> String {
    let mut page = settings::head(Glyph::Folder, Tile::Teal, "Mounts", "Every filesystem attached to this machine's folders, and how files without permissions of their own are treated on each.");
    page.push_str(&l.banner());
    let list = match &l.manager.machine.mounts {
        Ok(list) => list,
        Err(why) => return page + &settings::group("", &settings::row("The mounts couldn't be read", why, ""), ""),
    };
    for origin in [Origin::Storage, Origin::Shared, Origin::System, Origin::Kernel] {
        let rows: String = list.iter().filter(|m| m.origin == origin).map(mount_link).collect();
        let rows = if rows.is_empty() && origin == Origin::Storage {
            settings::row("None", "No filesystem on a disk is mounted. Mount one from its disk's page.", "")
        } else {
            rows
        };
        if rows.is_empty() {
            continue;
        }
        let foot = if origin == Origin::Storage && l.offered() { settings::hint("To mount a filesystem, open it from its disk.") } else { String::new() };
        page.push_str(&settings::group(origin_title(origin), &rows, &foot));
    }
    page
}

pub fn mount(l: &Looking, id: u32) -> String {
    let machine = &l.manager.machine;
    let Some(m) = machine.mount(id) else {
        return settings::back("Mounts", "back") + &settings::group("", &settings::row("It isn't mounted any more", "", ""), "");
    };
    let mut page = settings::back("Back", "back");
    let (glyph, tile) = origin_glyph(m.origin);
    let pill = if m.hidden {
        settings::pill("Covered", Tone::Warn)
    } else if m.read_only {
        settings::pill("Read-Only", Tone::Plain)
    } else {
        settings::pill("Read and Write", Tone::Good)
    };
    page.push_str(&settings::hero(&settings::hero_title(glyph, tile, &m.target, &from_line(m)), &pill));
    page.push_str(&l.banner());
    if m.hidden {
        page.push_str(&settings::caution("A later mount at this place, or above it, covers this one: what is here can't be reached until that is unmounted."));
    }

    let mut rows = String::new();
    if let Some((total, free)) = m.usage {
        rows.push_str(&page::usage_row(total, free));
    }
    // The volume it is mounted from, where it is one Disk Manager shows.
    let volume = machine.disks.iter().flatten().flat_map(|d| d.volumes().map(move |v| (d, v))).find(|(_, v)| machine.mounts_of(v).iter().any(|x| x.id == m.id));
    if let Some((d, v)) = volume {
        let from = match &v.partition {
            Some(_) => format!("{} of {} · {}", words::volume_title(v), d.name, words::volume_line(v)),
            None => format!("{} · {}", d.name, words::volume_line(v)),
        };
        rows.push_str(&settings::row("From", &from, &settings::button("Open", "open-volume", &[("volume", &v.name)], Button::Quiet, true)));
    } else {
        rows.push_str(&settings::fact("From", if m.source.is_empty() { "—" } else { &m.source }, true));
    }
    rows.push_str(&settings::fact("Filesystem", &words::mount_kind(&m.kind), false));
    rows.push_str(&settings::fact("Options", &m.options.join(","), true));
    if l.offered() {
        let about = m.needed().unwrap_or("Detach it from this folder. Programs using files on it must close them first.");
        let control = if m.needed().is_some() { String::new() } else { settings::button("Unmount", "unmount", &[("mount", &id.to_string())], Button::Plain, l.may()) };
        rows.push_str(&settings::row("Unmount", about, &control));
    }
    page.push_str(&settings::group("Mount", &rows, ""));

    page.push_str(&policy_group(m, l));

    if !m.strata.is_empty() {
        let rows: String = m
            .strata
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut said = Vec::new();
                if s.flags.iter().any(|f| f == "create") {
                    said.push("New files go here");
                }
                if s.flags.iter().any(|f| f == "ro") {
                    said.push("Read-only");
                }
                if s.state != "present" {
                    said.push("Missing");
                }
                let line = if said.is_empty() { "Read and write".to_string() } else { said.join(" · ") };
                let order = if i == 0 { "Top".to_string() } else { format!("Under {}", i) };
                settings::item(&settings::icon(Glyph::Layers, Tile::Violet), &s.path, &[(&line, false), (&order, false)], "")
            })
            .collect();
        page.push_str(&settings::group("Strata", &rows, &settings::hint("A file is found in the first stratum that has it. The system's views are set up as Peios starts.")));
    }
    page
}

fn policy_group(m: &Mount, l: &Looking) -> String {
    let policy = match &m.policy {
        Ok(p) => p,
        Err(why) => return settings::group("Files Without Permissions", &settings::row("Unknown", why, ""), ""),
    };
    let fixed = m.fixed_policy();
    let changeable = l.offered() && fixed.is_none() && !m.hidden;
    let mut rows = if changeable {
        // Offered as the filesystem allows: a saving policy needs somewhere
        // to save, and an unmanaged one can't be chosen.
        let fs = machine_fs(m, l);
        let mut choices = page::policies(fs.as_ref());
        if !choices.contains(&policy.kind) {
            choices.push(policy.kind);
        }
        let options: Vec<(String, String)> = choices.iter().map(|k| (k.id().to_string(), words::policy(*k).to_string())).collect();
        settings::row("Policy", words::policy_about(policy.kind), &settings::select(&format!("policy:{}", m.id), "Files without permissions", &options, l.may()))
    } else {
        settings::row("Policy", words::policy_about(policy.kind), &settings::value(words::policy(policy.kind), false))
    };
    if policy.kind.makes() {
        let said = match &policy.template {
            Some(sddl) => format!("Where a file's folder has none to pass on. Now: {sddl}"),
            None => "Where a file's folder has none to pass on. None is set, so the kernel's own is used.".into(),
        };
        let label = if changeable && l.may() { "Permissions…" } else { "View…" };
        rows.push_str(&settings::row("Permissions They Are Given", &said, &settings::button(label, "permissions-mount", &[("mount", &m.id.to_string())], Button::Plain, !l.manager.editing)));
    }
    let foot = match fixed {
        Some(why) => settings::note(why),
        None => settings::hint(&format!("Kept in the kernel as {}.", policy_name(policy.kind))),
    };
    settings::group("Files Without Permissions", &rows, &foot)
}

/// The policy's own name, as `mount -o policy=` and the docs have it.
fn policy_name(kind: PolicyKind) -> &'static str {
    match kind {
        PolicyKind::Unmanaged => "unmanaged",
        PolicyKind::DenyMissing => "deny-missing",
        PolicyKind::Ephemeral => "synth-ephemeral",
        PolicyKind::Persistent => "synth-persist",
    }
}

fn machine_fs(m: &Mount, l: &Looking) -> Option<crate::disks::Filesystem> {
    let machine = &l.manager.machine;
    machine.disks.iter().flatten().flat_map(|d| d.volumes()).find(|v| machine.mounts_of(v).iter().any(|x| x.id == m.id)).and_then(|v| v.filesystem.clone())
}

pub fn startup(l: &Looking) -> String {
    let machine = &l.manager.machine;
    let mut page = settings::head(Glyph::Power, Tile::Green, "At Startup", "Filesystems mounted each time this machine starts.");
    page.push_str(&l.banner());

    // What someone added first: it is what this page changes.
    let asking = matches!(l.manager.asking, Some(Asking::AddStartup));
    let add = if l.offered() { settings::button("Add…", "ask-add-startup", &[], Button::Plain, l.may() && !asking) } else { String::new() };
    let about = "A filesystem mounted at a folder each time this machine starts, found by its UUID wherever it is plugged in.";
    let mut rows = String::new();
    match &machine.startup {
        Ok(list) => {
            for s in list {
                let line = format!("{} · {}", if s.read_only { "Read-only" } else { "Read and write" }, words::policy(s.policy.kind));
                let attached = machine.disks.iter().flatten().flat_map(|d| d.volumes()).any(|v| v.filesystem.as_ref().and_then(|f| f.uuid.as_deref()) == Some(s.uuid.as_str()));
                let shown = if attached { s.shown.clone() } else { format!("{} — not attached now", s.shown) };
                let control = if l.offered() { settings::button("Remove", "remove-startup", &[("id", &s.id)], Button::Plain, l.may()) } else { String::new() };
                rows.push_str(&settings::item(&settings::icon(Glyph::Folder, Tile::Teal), &s.target, &[(&shown, false), (&line, false)], &control));
            }
            if list.is_empty() {
                // One row: that there are none, and what adds one.
                rows.push_str(&settings::row("None Added", about, &add));
            } else if l.offered() {
                rows.push_str(&settings::row("Add Another", about, &add));
            }
        }
        Err(why) => rows.push_str(&settings::row("The list couldn't be read", why, "")),
    }
    if asking {
        rows.push_str(&startup_form(l));
    }
    page.push_str(&settings::group("Added Here", &rows, ""));

    // What Peios mounts itself, which this window can't change.
    let own: String = machine
        .mounts
        .iter()
        .flatten()
        .filter(|m| m.origin == Origin::System || (m.origin == Origin::Storage && m.needed().is_some()))
        .filter(|m| !m.hidden)
        .map(|m| {
            let (glyph, tile) = origin_glyph(m.origin);
            settings::item(&settings::icon(glyph, tile), &m.target, &[(&from_line(m), false)], "")
        })
        .collect();
    if !own.is_empty() {
        page.push_str(&settings::group("Mounted by Peios", &own, &settings::locked("Peios mounts these itself as it starts. They aren't changed here.")));
    }
    page
}

fn startup_form(l: &Looking) -> String {
    let machine = &l.manager.machine;
    let mut options = vec![(String::new(), "Choose…".to_string())];
    let mut chosen_fs = None;
    // Not what the system runs from: Peios mounts that itself.
    let disks = machine.disks.iter().flatten().filter(|d| !machine.holds_system(d));
    for v in disks.flat_map(|d| d.volumes()) {
        let Some(fs) = &v.filesystem else { continue };
        let Some(uuid) = &fs.uuid else { continue };
        if ["swap", "crypto_LUKS", "LVM2_member", "linux_raid_member", "iso9660", "squashfs"].contains(&fs.kind.as_str()) {
            continue;
        }
        if l.fields.get("startup-fs") == uuid {
            chosen_fs = Some(fs.clone());
        }
        options.push((uuid.clone(), format!("{} · {}", page::volume_name(v), words::filesystem(fs))));
    }
    let mut rows = settings::row("Filesystem", "", &settings::select("startup-fs", "Filesystem", &options, true));
    rows.push_str(&settings::row("Mount At", "A folder, made if it isn't there.", &settings::text("startup-where", "Mount at", "text", Width::Wide, true, r#"autocomplete="off" spellcheck="false" placeholder="/mnt/data""#).replacen("st-input", "st-input mono", 1)));
    rows.push_str(&settings::row("Read-Only", "Nothing on it can be changed while it is mounted.", &settings::switch("startup-ro", "Read-only", true)));
    rows.push_str(&page::policy_rows("startup-policy", &page::policies(chosen_fs.as_ref()), l));
    let buttons = settings::actions(&format!("{}{}", settings::button("Cancel", "cancel", &[], Button::Plain, true), settings::submit("Add", Button::Primary, l.may())));
    settings::more(More::Form, &format!(r#"<form fx-submit="add-startup">{rows}{buttons}</form>"#))
}
