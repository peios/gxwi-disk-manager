//! Disk Manager: this machine's disks, what is on them and where it is
//! mounted, and the changes to all of it.
//!
//! What it shows is read as the person, from lsblk, sysfs, the kernel's
//! mount table and the kernel's mount policies. What it changes is made by
//! the privileged disk service, which doesn't exist yet: until it does,
//! every change is designed in full and makes nothing (`service`).

use libgxwi::App;

mod disks;
mod manager;
mod mounts;
mod page;
mod permissions;
mod service;
mod words;

use manager::Manager;

// What this program looks like, to whatever lists it. The icon itself is
// `gxwi-disk-manager.svg` at the repo root, installed as the base theme's.
libgxwi::icon!(b"dev.peios.gxwi-disk-manager");

fn main() {
    if std::env::args().nth(1).is_some() {
        eprintln!("gxwi-disk-manager: usage: gxwi-disk-manager (given {:?})", std::env::args().skip(1).collect::<Vec<_>>());
        std::process::exit(64);
    }
    let mut app = match App::connect() {
        Ok(app) => app,
        Err(e) => {
            eprintln!("gxwi-disk-manager: no desktop to open on: {e}");
            std::process::exit(1);
        }
    };
    libgxwi::settings::stylesheet(&mut app);
    app.stylesheet("/gxwi-disk-manager.css", include_str!("gxwi-disk-manager.css"));
    let window = app.live("Disk Manager", Manager::new());
    let aside = std::sync::Arc::downgrade(&window);
    window.update(|manager, _| {
        manager.window = aside;
        manager.reread();
        manager.watch();
    });
    if let Err(e) = app.run() {
        eprintln!("gxwi-disk-manager: {e}");
        std::process::exit(1);
    }
}
