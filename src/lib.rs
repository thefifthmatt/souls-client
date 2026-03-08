use std::{ffi::c_void, path::Path};

use windows::{
    Win32::{Foundation::HINSTANCE, System::SystemServices::DLL_PROCESS_ATTACH},
    core::BOOL,
};

use crate::{client::Client, game::FromGame, hooks::hook_messages, items::ItemUpdater, program::{Program, current_module_path}, spawn::EnemySpawner, ui::Widget};

mod arenas;
mod client;
mod event;
mod game;
mod hooks;
mod items;
mod logger;
mod name_templates;
mod program;
mod rva;
mod spawn;
mod ui;

fn main() -> eyre::Result<()> {
    let (product, version) = program::get_product_info().expect("No version info found in exe");

    log::info!("Mod start: game {} version {}", product, version);
    let game = match (product.as_str(), version.as_str()) {
        ("DARK SOULS: REMASTERED", "1.0.0.0") => FromGame::DS1R,
        ("DARK SOULS™ III", "1.15.2.0") => FromGame::DS3,
        ("Sekiro™: Shadows Die Twice", "1.6.0.0") => FromGame::SDT,
        ("ELDEN RING™", "2.6.1.0") => FromGame::ER,
        _ => panic!("Unknown game {} version {} found with dll mod", product, version),
    };

    // For cross-game compatibility this is just a random number
    let unique_id = crate::game::get_game_hash(true);

    let local = true;
    let host = if local { "localhost:3000" } else { panic!("Remote host not configured") };
    Client::initialize(host, &unique_id, game);

    if game == FromGame::ER {
        // For now, use presence of spawn file for a bunch of features
        let mut spawn_file = current_module_path();
        spawn_file.set_file_name("spawn.txt");
        let advanced = Path::exists(&spawn_file);
        if !advanced {
            log::info!("{:?} not found, so disabling spawn and UI", spawn_file);
        }
        EnemySpawner::initialize(advanced);
        ItemUpdater::initialize();
        if advanced {
            Widget::initialize();
        }
    }
    if game == FromGame::DS3 {
        // Rely on patch fix for DS1R since neuter_arxan seems to be too slow at startup
        unsafe {
            dearxan::disabler::neuter_arxan(move |result| {
                log::info!("Dearxan result: {:?}", result);
                hook_messages(game);
            });
        }
    } else {
        std::thread::spawn(move || {
            hook_messages(game);
        });
    }

    Client::get().start();

    Ok(())
}

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(_inst: HINSTANCE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        logger::init();
        logger::set_panic_hook();

        #[cfg(false)]
        if libhotpatch::is_hotpatched() {
            return true.into();
        }

        std::thread::spawn(|| main().unwrap());
    }

    true.into()
}