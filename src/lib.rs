use std::ffi::c_void;
use windows::{
    Win32::{Foundation::HINSTANCE, System::SystemServices::DLL_PROCESS_ATTACH},
    core::BOOL,
};

use crate::{
    client::Client,
    game::FromGame,
};

mod client;
mod event;
mod game;
mod hooks;
mod logger;
mod program;
mod rva;

#[cfg(feature = "bosses")]
mod bosses;
#[cfg(feature = "deathlink")]
mod deathlink;
#[cfg(feature = "items")]
mod items;
#[cfg(feature = "names")]
mod names;
#[cfg(feature = "spawn")]
mod spawn;
#[cfg(feature = "ui")]
mod ui;

pub const VERSION: &str = "0.4";

fn main() -> eyre::Result<()> {
    let (product, version) = program::get_product_info().expect("No version info found in exe");
    log::info!("-------------------------------------------------------------------------------");
    log::info!("Client {}: {} {}", VERSION, product, version);
    let game = match (product.as_str(), version.as_str()) {
        ("DARK SOULS: REMASTERED", "1.0.0.0") => FromGame::DS1R,
        ("DARK SOULS™ III", "1.15.2.0") => FromGame::DS3,
        ("Sekiro™: Shadows Die Twice", "1.6.0.0") => FromGame::SDT,
        ("ELDEN RING™", "2.6.2.0") => FromGame::ER,
        _ => panic!("Unknown game {} version {} found with dll mod", product, version),
    };

    let unique_id = crate::game::get_game_hash();

    let local = false;
    let host = if local { "localhost:3000" } else { include_str!("serverhost.txt").trim() };
    Client::initialize(host, &unique_id, game);

    #[cfg(feature = "names")]
    crate::names::NameClient::initialize();

    if game == FromGame::ER {
        #[cfg(feature = "ui")]
        crate::ui::WidgetChannel::initialize();
        #[cfg(feature = "items")]
        crate::items::ItemUpdater::initialize();
        #[cfg(feature = "spawn")]
        crate::spawn::EnemySpawner::initialize();
        #[cfg(feature = "bosses")]
        crate::bosses::BossClient::initialize();
        #[cfg(feature = "deathlink")]
        crate::deathlink::DeathlinkClient::initialize();
    }
    if game == FromGame::DS3 && cfg!(feature = "ds3") {
        // Rely on patch fix for DS1R instead since neuter_arxan seems to be too slow at startup
        #[cfg(feature = "ds3")]
        unsafe {
            dearxan::disabler::neuter_arxan(move |result| {
                log::info!("Dearxan result: {:?}", result);
                Client::get().hook();
            });
        }
    } else {
        std::thread::spawn(move || {
            Client::get().hook();
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

        std::thread::spawn(|| main().unwrap());
    }

    true.into()
}

// Awkward dependencies for unused_crate_dependencies
use rand as _;

#[cfg(test)]
use memoffset as _;
