//! Permissions in the permissions editor (gxwi-sd-editor): a new
//! filesystem's top folder, and the template a mount gives files without
//! permissions of their own. Both are a file's descriptor, with a file's
//! rights.

use gxwi_sd_editor::{Can, Children, Generic, Object, Request, Right};
use peios::file::File;
use peios::security::AccessMask;

/// What a filesystem's top folder is given unless the person says
/// otherwise, and what a template is: the installer's own, for the root
/// it formats. SYSTEM and Administrators have full control, everyone may
/// read and run, and whoever makes something inside owns it.
pub const DEFAULT_SDDL: &str = "O:SYG:SYD:(A;OICI;GA;;;SY)(A;OICI;GA;;;BA)(A;OICI;GRGX;;;WD)(A;OICIIO;GA;;;S-1-3-0)";

pub fn default_root() -> Vec<u8> {
    peios::security::sddl::parse(DEFAULT_SDDL).map(|sd| sd.as_bytes().to_vec()).unwrap_or_default()
}

fn rights() -> Vec<Right> {
    let generic = generic();
    let right = |name: &str, mask: u32| Right { name: name.into(), mask, general: true };
    vec![
        right("Full control", generic.all),
        right("Modify", generic.read | generic.write | generic.execute | AccessMask::DELETE.bits()),
        right("Read & execute", generic.read | generic.execute),
        right("Read", generic.read),
        right("Write", generic.write),
    ]
}

/// What the generic rights stand for on a file, as the kernel has it.
fn generic() -> Generic {
    let mapping = File::generic_mapping();
    let means = |generic: AccessMask| generic.resolve_generic(&mapping).bits();
    Generic {
        read: means(AccessMask::GENERIC_READ),
        write: means(AccessMask::GENERIC_WRITE),
        execute: means(AccessMask::GENERIC_EXECUTE),
        all: means(AccessMask::GENERIC_ALL),
    }
}

/// Opens the editor on `sd`, named `what`. `may` says whether it can be
/// changed, and `why` why not. `apply` is given each descriptor applied;
/// `done` is called once the editor has gone.
pub fn edit(
    what: &str,
    sd: &[u8],
    may: bool,
    why: Option<String>,
    apply: impl FnMut(&[u8]) -> Result<(), String> + Send + 'static,
    done: impl FnOnce() + Send + 'static,
) -> Result<(), String> {
    let mut apply = apply;
    let request = Request {
        object: Object { name: what.to_string(), kind: "Folder".into(), container: true, children: Children::All, ..Object::default() },
        sd: sd.to_vec(),
        rights: rights(),
        generic: generic(),
        can: Can { dacl: may, owner: may, why: if may { None } else { why }, ..Can::default() },
        ..Request::default()
    };
    gxwi_sd_editor::edit(&request, move |sd, _parts| apply(sd), done).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "The permissions editor isn't installed.".to_string()
        } else {
            format!("The permissions editor couldn't be opened: {e}.")
        }
    })
}
