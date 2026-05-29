use std::{hash::{DefaultHasher, Hasher}, ops::DerefMut};

use eldenring::cs::{GameDataMan, PlayerGameData};
use fromsoftware_shared::FromStatic;

pub trait PlayerGameDataExt {
    unsafe fn main_instance() -> Option<&'static mut Self>;

    fn character_name(&self) -> String;
}

impl PlayerGameDataExt for PlayerGameData {
    unsafe fn main_instance() -> Option<&'static mut PlayerGameData> {
        unsafe { GameDataMan::instance_mut() }.ok().map(|game| game.main_player_game_data.deref_mut())
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

pub fn get_game_hash() -> String {
    // Select either Steam id or random number (resets every game restart) based on feature.
    // Using both would also be possible.
    let game_id = get_game_id();
    let mut hasher = DefaultHasher::new();
    hasher.write_u32(0x2B0708A2);
    hasher.write_u64(game_id);
    let hash = hasher.finish();
    format!("{:04x}", hash & 0xFFFF)
}

#[cfg(feature = "eldenring")]
pub fn get_game_id() -> u64 {
    // To be safe, steam dll with these functions must be loaded. True for Elden Ring 1.16.1 at least.
    // The failure will happen when loading the dll, long before here.
    unsafe {
        let user = steamworks_sys::SteamAPI_SteamUser_v021();
        steamworks_sys::SteamAPI_ISteamUser_GetSteamID(user)
    }
}

#[cfg(not(feature = "eldenring"))]
pub fn get_game_id() -> u64 {
    let mut rng = rand::rng();
    rand::RngExt::random::<u64>(&mut rng)
}
