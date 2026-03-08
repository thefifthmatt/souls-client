use std::collections::HashMap;
use std::ptr::NonNull;
use std::{ffi::c_void};
use eldenring::cs::{ChrIns, MenuString};
use windows::core::PCWSTR;
use std::sync::{LazyLock, Mutex};
use std::cell::Cell;

use crate::client::SPAWN_NAME_ID;
use crate::game::FromGame;
use crate::rva::{DISPLAY_BOSS_HEALTHBAR, DISPLAY_BOSS_HEALTHBAR_DS1R, DISPLAY_BOSS_HEALTHBAR_DS3, DISPLAY_BOSS_HEALTHBAR_SDT, GET_CHR_INS_BY_ENTITY_ID, GET_CHR_NAME, GET_MESSAGE_DS3, GET_MESSAGE_SDT, GET_NPC_NAME_DS1R};
use crate::{
    hooks::install::hook,
    program::Program,
    name_templates::{ER_ENTITY_ID_TEMPLATES, ER_GROUP_ENTITY_IDS},
    rva::{
        GET_MESSAGE_RVA,
        SUMMON_BUDDY_CHRSET_ALLOC_SIZE,
        SUMMON_BUDDY_CHRSET_CAPACITY,
        SUMMON_BUDDY_CHRSET_MEMSET_SIZE,
    },
    spawn::CHR_SET_CAPACITY,
};

type GetMessage = unsafe extern "C" fn(*mut c_void, u32, u32, i32) -> PCWSTR;
type GetChrName = unsafe extern "C" fn(*mut c_void, Option<NonNull<ChrIns>>, bool) -> *mut MenuString;
type DisplayBossHealthbar = unsafe extern "C" fn(NonNull<u32>, u32, i32);

type GetNpcNameDS1R = unsafe extern "C" fn(i32) -> PCWSTR;
type DisplayBossHealthbarLua = unsafe extern "C" fn(*mut c_void, i32, i32, i32);

mod install;

fn er_npc_bnd(bnd_id: u32) -> bool { bnd_id == 18 || bnd_id == 328 || bnd_id == 428 }
fn ds3_npc_bnd(bnd_id: u32) -> bool { bnd_id == 18 || bnd_id == 215 || bnd_id == 255 }
fn sekiro_npc_bnd(bnd_id: u32) -> bool { bnd_id == 18 }

pub fn hook_messages(game: FromGame) {
    let program = Program::current();
    unsafe {
        match game {
            FromGame::ER => {
                let get_message = program.derva_ptr::<GetMessage>(GET_MESSAGE_RVA);
                hook(get_message, |original| {
                    move |param_1, param_2, param_3, param_4|
                        get_message_override(er_npc_bnd(param_3), param_4, &|| original(param_1, param_2, param_3, param_4))
                });

                let get_chr_name = program.derva_ptr::<GetChrName>(GET_CHR_NAME);
                hook(get_chr_name, |original| {
                    move |param_1, param_2, param_3|
                        get_chr_name_override(param_2, &|| original(param_1, param_2, param_3))
                });
                let display_boss_healthbar = program.derva_ptr::<DisplayBossHealthbar>(DISPLAY_BOSS_HEALTHBAR);
                hook(display_boss_healthbar, |original| {
                    move |param_1, param_2, param_3|
                        display_boss_healthbar_override_er(param_1, param_3, &|| original(param_1, param_2, param_3))
                });
            },
            FromGame::DS1R => {
                let get_message = program.derva_ptr::<GetNpcNameDS1R>(GET_NPC_NAME_DS1R);
                hook(get_message, |original| {
                    move |param_1|
                        get_message_override(true, param_1, &|| original(param_1))
                });
                let display_boss_healthbar = program.derva_ptr::<DisplayBossHealthbarLua>(DISPLAY_BOSS_HEALTHBAR_DS1R);
                hook(display_boss_healthbar, |original| {
                    move |param_1, param_2, param_3, param_4|
                        display_boss_healthbar_override_lua(param_2, param_4, &|| original(param_1, param_2, param_3, param_4))
                });
            },
            FromGame::SDT => {
                // This needs a delay, it's super encrypted (3s seems fine)
                let get_message = program.derva_ptr::<GetMessage>(GET_MESSAGE_SDT);
                hook(get_message, |original| {
                    move |param_1, param_2, param_3, param_4|
                        get_message_override(sekiro_npc_bnd(param_3), param_4, &|| original(param_1, param_2, param_3, param_4))
                });
                let display_boss_healthbar = program.derva_ptr::<DisplayBossHealthbarLua>(DISPLAY_BOSS_HEALTHBAR_SDT);
                hook(display_boss_healthbar, |original| {
                    move |param_1, param_2, param_3, param_4|
                        display_boss_healthbar_override_lua(param_2, param_4, &|| original(param_1, param_2, param_3, param_4))
                });
            },
            FromGame::DS3 => {
                let get_message = program.derva_ptr::<GetMessage>(GET_MESSAGE_DS3);
                hook(get_message, |original| {
                    move |param_1, param_2, param_3, param_4|
                        get_message_override(ds3_npc_bnd(param_3), param_4, &|| original(param_1, param_2, param_3, param_4))
                });
                let display_boss_healthbar = program.derva_ptr::<DisplayBossHealthbarLua>(DISPLAY_BOSS_HEALTHBAR_DS3);
                hook(display_boss_healthbar, |original| {
                    move |param_1, param_2, param_3, param_4|
                        display_boss_healthbar_override_lua(param_2, param_4, &|| original(param_1, param_2, param_3, param_4))
                });
            },
        }
    }
}

pub fn hook_spawn() {
    let program = Program::current();
    unsafe {
        const CAP: u32 = CHR_SET_CAPACITY;
        const SIZE: u32 = CAP * 0x10;
        // Double buddy set
        std::ptr::write_unaligned(
            program.derva_ptr::<*mut u32>(SUMMON_BUDDY_CHRSET_ALLOC_SIZE),
            SIZE,
        );

        std::ptr::write_unaligned(
            program.derva_ptr::<*mut u32>(SUMMON_BUDDY_CHRSET_MEMSET_SIZE),
            SIZE,
        );

        std::ptr::write_unaligned(
            program.derva_ptr::<*mut u32>(SUMMON_BUDDY_CHRSET_CAPACITY),
            CAP,
        );
    }
}

thread_local! {
    static CHR_ENTITY_ID: Cell<u32> = Cell::new(0);
}

fn get_replace_npc_name(msg_id: i32) -> Option<PCWSTR> {
    static OVERRIDE_STR: LazyLock<Mutex<HashMap::<String, Vec<u16>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

    let entity_id = CHR_ENTITY_ID.get();
    let client = crate::client::Client::get();
    let str = match client.claim_name(entity_id, msg_id) {
        Some(str) => str,
        None if msg_id == SPAWN_NAME_ID => "".to_string(),
        _ => return None,
    };
    // log::info!("{}, {}: {}", msg_id, entity_id, str);

    let mut char_map = OVERRIDE_STR.lock().unwrap();
    let chars = char_map.entry(str).or_insert_with_key(|str| {
        // We should be able to reuse this indefinitely, although it will keep getting more memory
        let strs: Vec<u16> = str.encode_utf16().collect();
        let mut chars =  Vec::with_capacity(strs.len() + 1);
        chars.extend_from_slice(&strs);
        chars.push(0);
        chars
    });
    Some(PCWSTR(chars.as_ptr()))
}

// No bnd_id information here, probably best to calculate this outside to share between games
unsafe fn get_message_override(is_npc_name: bool, msg_id: i32, original: &dyn Fn() -> PCWSTR) -> PCWSTR {
    let vanilla_ptr = original();
    if vanilla_ptr.is_null() || !is_npc_name {
        return vanilla_ptr;
    }
    let Result::Ok(_vanilla) = (unsafe { vanilla_ptr.to_string() }) else {
        return vanilla_ptr;
    };
    match get_replace_npc_name(msg_id) {
        Some(replace) => replace,
        None => vanilla_ptr,
    }
}

// To hot patch:
// #[cfg_attr(debug_assertions, libhotpatch::hotpatch)]

unsafe fn get_chr_name_override(chr_ins: Option<NonNull<ChrIns>>, original: &dyn Fn() -> *mut MenuString) -> *mut MenuString {
    let entity_id = chr_ins.map(|chr| unsafe { chr.as_ref() }.event_entity_id).unwrap_or(0);
    CHR_ENTITY_ID.set(entity_id);
    let result = original();
    CHR_ENTITY_ID.set(0);
    result
} 

// This sets the msg_id in CSFeMan, but the text itself is fetched separately, updated in UpdateBossNames
// Still, aside from looking at CEFeMan, this is probably the best place to associate with entity id
unsafe fn display_boss_healthbar_override_er(entity_id: NonNull<u32>, msg_id: i32, original: &dyn Fn()) {
    let program = Program::current();
    let get_chr_ins_by_entity_id = unsafe { program.derva_ptr::<extern "C" fn(
        NonNull<u32>, *mut i32, *mut i32
    ) -> Option<NonNull<ChrIns>>>(GET_CHR_INS_BY_ENTITY_ID) };
    // TODO: Check entity id directly, or else use buddy group to map to entity id
    // TODO: Set replacement name from template
    // State from server: pending name list (top 25), claimed names list (by entity id + fmg id)
    if let Some(chr_ins) = get_chr_ins_by_entity_id(entity_id, std::ptr::null_mut(), std::ptr::null_mut()) {
        let chr_ins = unsafe { chr_ins.as_ref() };
        let mut entity_id = chr_ins.event_entity_id;
        if !ER_ENTITY_ID_TEMPLATES.contains_key(&entity_id) {
            if let Some(part_data) = chr_ins.module_container.data.msb_parts.msb_part {
                let part_data = unsafe { part_data.as_ref() };
                let common_data = unsafe { part_data.common.as_ref() };
                for group_id in common_data.entity_group_ids {
                    if group_id == 0 { continue }
                    // These should all have name templates
                    if let Some(rep_entity_id) = ER_GROUP_ENTITY_IDS.get(&group_id) {
                        log::info!("Group entity id {} -> {}", group_id, rep_entity_id);
                        entity_id = *rep_entity_id;
                        break;
                    }
                }
            }
        }

        // Let the client figure out if the template is valid
        crate::Client::get().set_msg_boss(entity_id, msg_id);
    }
    
    original();
}

unsafe fn display_boss_healthbar_override_lua(entity_id: i32, msg_id: i32, original: &dyn Fn()) {
    if entity_id > 0 {
        let entity_id = entity_id as u32;
        crate::Client::get().set_msg_boss(entity_id, msg_id);
    }
    original();
}

#[cfg(test)]
mod test {
    #[test]
    fn expected_offsets() {
        assert_eq!(0x60, memoffset::offset_of!(eldenring::cs::MsbPart, common));
        assert_eq!(0x1c, memoffset::offset_of!(eldenring::cs::MsbPartCommon, entity_group_ids));
    }
}
