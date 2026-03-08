use std::hash::{Hasher, DefaultHasher};

use eldenring::cs::{GameDataMan, PlayerGameData};
use fromsoftware_shared::FromStatic;
use rand::RngExt;

pub trait PlayerGameDataExt {
    unsafe fn main_instance() -> Option<&'static mut Self>;

    fn character_name(&self) -> String;
}

impl PlayerGameDataExt for PlayerGameData {
    unsafe fn main_instance() -> Option<&'static mut PlayerGameData> {
        unsafe { GameDataMan::instance() }.ok().map(|game| game.main_player_game_data.as_mut())
    }

    fn character_name(&self) -> String {
        let length = self.character_name.iter().position(|c| *c == 0).unwrap_or(self.character_name.len());
        String::from_utf16(&self.character_name[..length]).unwrap()
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum FromGame {
    DS1R,
    DS3,
    SDT,
    ER,
}

pub fn get_game_hash(random: bool) -> String {
    let steam_id = get_steam_id();
    let mut hasher = DefaultHasher::new();
    if random {
        let mut rng = rand::rng();
        hasher.write_u32(rng.random::<u32>())
    }
    hasher.write_u64(steam_id);
    let hash = hasher.finish();
    format!("{:04x}", hash & 0xFFFF)
}

#[cfg(false)]
pub fn get_steam_id() -> u64 {
    // To be safe, steam dll with these functions must be loaded. True for Elden Ring 1.16.1 at least.
    // The failure will happen when loading the dll, long before here.
    unsafe {
        let user = steamworks_sys::SteamAPI_SteamUser_v021();
        steamworks_sys::SteamAPI_ISteamUser_GetSteamID(user)
    }
}

#[cfg(true)]
pub fn get_steam_id() -> u64 {
    0
}
