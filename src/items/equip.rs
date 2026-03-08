use std::collections::{HashMap, HashSet};
use eldenring::cs::{ChrAsmSlot, EquipGameData, EquipInventoryData, EquipInventoryDataListEntry, EquipMagicData, ItemCategory, ItemId, OptionalItemId, PlayerGameData, PlayerIns, WorldChrMan};
use eldenring::fd4::{FD4ParamRepository};
use eldenring::param::{EQUIP_PARAM_CUSTOM_WEAPON_ST, EQUIP_PARAM_PROTECTOR_ST, EQUIP_PARAM_WEAPON_ST, ITEMLOT_PARAM_ST, ParamDef};
use fromsoftware_shared::{FromStatic};

use crate::event::emedf;
use crate::program::Program;
use crate::rva::{EQUIP_INVENTORY, EQUIP_MAGIC, UNEQUIP_INVENTORY, UNEQUIP_MAGIC};
use crate::{
    items::data::ItemData,
    game::PlayerGameDataExt,
};

#[derive(PartialEq, Eq)]
pub enum EquipStatus {
    Done,
    NotNeeded,
    // Item missing, should probably not retry
    Missing,
    // Tried to equip but failed due to player state, can retry
    Failed,
}

#[derive(Debug, Default)]
pub struct EquipHandler {
    // From oldest to newest
    accessory_order: Vec<ItemId>,
    spell_order: Vec<ItemId>,
    tear_order: Vec<ItemId>,
}

// --- Inventory functions
fn get_latest_inventory_entry(inventory: &EquipInventoryData, item_id: ItemId, ignore_level: bool) -> Option<(i32, &EquipInventoryDataListEntry)> {
    let data = &inventory.items_data;
    let matches = &|entry_id| {
        if ignore_level {
            return ItemData::get_base_item(entry_id) == ItemData::get_base_item(item_id);
        } else {
            return entry_id == item_id;
        }
    };
    let mut sort_id = 0;
    // This could also return Option<u32>, and/or the entry. The game does use -1 for missing indices at least
    let mut index = None;
    // This is a bit manual
    for (i, entry) in data.normal_entries().iter().enumerate() {
        if let Some(entry) = entry.as_option() {
            if entry.sort_id >= sort_id && matches(entry.item_id) {
                sort_id = entry.sort_id;
                index = Some((data.key_items_capacity as i32 + i as i32, entry));
            }
        }
    }
    for (i, entry) in data.key_entries().iter().enumerate() {
        if let Some(entry) = entry.as_option() {
            if entry.sort_id >= sort_id && matches(entry.item_id) {
                sort_id = entry.sort_id;
                index = Some((i as i32, entry));
            }
        }
    }
    index
}

fn get_item_inventory_indices(inventory: &EquipInventoryData, item_ids: Vec<OptionalItemId>) -> Vec<i32> {
    let data = &inventory.items_data;
    // This doesn't preserve order but it's fine, probably
    let item_ids: HashSet<ItemId> = item_ids.iter().filter_map(|id| id.as_valid()).collect();
    let mut indices = Vec::new();
    for (i, entry) in data.normal_entries().iter().enumerate() {
        if let Some(entry) = entry.as_option() {
            // This is weird (allowing duplicates) but it's what C++ version does, TODO revisit
            if item_ids.contains(&entry.item_id) {
                indices.push(data.key_items_capacity as i32 + i as i32);
            }
        }
    }
    for (i, entry) in data.key_entries().iter().enumerate() {
        if let Some(entry) = entry.as_option() {
            if item_ids.contains(&entry.item_id) {
                indices.push(i as i32);
            }
        }
    }
    indices
}

fn get_regular_inventory_index(inventory: &EquipInventoryData, index: i32) -> i32 {
    let regular_index = index - inventory.items_data.key_items_capacity as i32;
    if index < 0 || index as u32 >= inventory.items_data.normal_items_capacity {
        return -1;
    }
    regular_index
}

fn get_inventory_entry(inventory: &EquipInventoryData, index: i32) -> Option<&EquipInventoryDataListEntry> {
    if index >= 0 && index < inventory.items_data.key_items_capacity as i32 {
        return inventory.items_data.key_entries()[index as usize].as_option();
    }
    let regular_index = get_regular_inventory_index(inventory, index);
    if regular_index < 0 {
        return None;
    }
    inventory.items_data.normal_entries()[regular_index as usize].as_option()
}

fn get_equipment_inventory_entry(equipment: &EquipGameData, equip_slot: ChrAsmSlot) -> Option<(i32, &EquipInventoryDataListEntry)> {
    let index = equipment.equipped_indices[equip_slot];
    get_inventory_entry(&equipment.equip_inventory_data, index).map(|e| (index, e))
}

const ACCESSORY_SLOTS: &[ChrAsmSlot] = &[ChrAsmSlot::Accessory1, ChrAsmSlot::Accessory2, ChrAsmSlot::Accessory3, ChrAsmSlot::Accessory4];

fn get_equipped_accessory_indices(player_game_data: &PlayerGameData) -> Vec<i32> {
    // From 0 to 3
    let unlocked_slots = player_game_data.unlocked_talisman_slots;
    let mut ids = Vec::new();
    for (i, &equip_slot) in ACCESSORY_SLOTS.iter().enumerate() {
        if i as u8 > unlocked_slots {
            break;
        }
        if let Some((index, _)) = get_equipment_inventory_entry(&player_game_data.equipment, equip_slot) {
            ids.push(index);
        } else {
            ids.push(-1);
        }
    }
    ids
}

fn get_equipped_accessory_ids(player_game_data: &PlayerGameData) -> Vec<OptionalItemId> {
    get_equipped_accessory_indices(player_game_data).iter().map(|&index| {
        if let Some(entry) = get_inventory_entry(&player_game_data.equipment.equip_inventory_data, index) {
            OptionalItemId::from(entry.item_id)
        } else {
            OptionalItemId::NONE
        }
    }).collect()
}

fn get_equipped_spell_ids(player_game_data: &PlayerGameData) -> Vec<OptionalItemId> {
    player_game_data.equipment.equip_magic_data.entries.iter().map(|e| {
        if e.param_id < 0 { OptionalItemId::NONE } else { ItemId::new(ItemCategory::Goods, e.param_id as u32).unwrap().into() }
    }).collect()
}

fn get_used_spell_slots(data: &ItemData, spells: &Vec<OptionalItemId>) -> u32 {
    spells.iter().filter_map(|equipped| {
        let equipped = equipped.as_valid()?;
        data.spell_slots.get(&equipped)
    }).sum()
}

fn get_equipped_physick_ids(player_game_data: &PlayerGameData) -> Vec<OptionalItemId> {
    player_game_data.equipment.equipment_entries.physick_tears.to_vec()
}

// --- Equip functions

fn clear_action_flag(player: &mut PlayerIns) {
    // At 0x10, set 1
    let anim_flag = &mut player.module_container.action_flag.animation_action_flags;
    anim_flag.set_stay_state(true);
    // At ctrl 0x18 + 0x8, set 1 and clear 0x10
    let menu_ctrl = unsafe { player.player_menu_ctrl.as_mut() };
    let menu_flag = &mut menu_ctrl.chr_menu_flags.flags;
    menu_flag.set_lock_equip_0(true);
    menu_flag.set_lock_equip_4(false);
}

#[repr(C)]
struct EquipInventoryItem {
    unk0: [u8; 0x8],
    equip_slot: ChrAsmSlot,
    unkc: [u8; 0x4C],
    inventory_index: i32,
    // TODO: What is the width of this struct? Only the above fields are used by equip_inventory
}

impl EquipInventoryItem {
    pub const fn new(equip_slot: ChrAsmSlot, inventory_index: i32) -> Self {
        EquipInventoryItem { unk0: [0; 0x8], equip_slot: equip_slot, unkc: [0; 0x4C], inventory_index: inventory_index }
    }
}

fn unequip_inventory(program: &Program, equipment: &EquipGameData, equip_slot: ChrAsmSlot, unk_arrow_cond: bool) {
    unsafe {
        let fun = program.derva_ptr::<unsafe extern "C" fn(&EquipGameData, ChrAsmSlot, bool)>(UNEQUIP_INVENTORY);
        fun(equipment, equip_slot, unk_arrow_cond);
    }
}

fn equip_inventory(program: &Program, equip_slot: ChrAsmSlot, inventory_index: i32) {
    unsafe {
        let fun = program.derva_ptr::<unsafe extern "C" fn(&EquipInventoryItem)>(EQUIP_INVENTORY);
        fun(&EquipInventoryItem::new(equip_slot, inventory_index));
    }
}

fn unequip_magic(program: &Program, equip_magic_data: &EquipMagicData, unequip_slot: u32) {
    unsafe {
        let fun = program.derva_ptr::<unsafe extern "C" fn(&EquipMagicData, u32)>(UNEQUIP_MAGIC);
        fun(equip_magic_data, unequip_slot);
    }
}

fn equip_magic(program: &Program, equip_magic_data: &EquipMagicData, equip_slot: u32, id: i32) {
    unsafe {
        let fun = program.derva_ptr::<unsafe extern "C" fn(&EquipMagicData, u32, i32)>(EQUIP_MAGIC);
        fun(equip_magic_data, equip_slot, id);
    }
}

impl EquipHandler {
    pub fn equip_item(&mut self, data: &ItemData, item_id: ItemId, log_equip: bool, ignore_level: bool) -> EquipStatus {
        let Some(mut player_game_data) = (unsafe { PlayerGameData::main_instance() }) else {
            // Note, this is usually not retry
            return EquipStatus::Missing;
        };
        // Require item in inventory
        let equipment = &player_game_data.equipment;
        let inventory = &equipment.equip_inventory_data;
        let Some((index, entry)) = get_latest_inventory_entry(inventory, item_id, ignore_level) else {
            return EquipStatus::Missing;
        };
        let item_id = entry.item_id;
        let Ok(repo) = (unsafe { FD4ParamRepository::instance() }) else {
            return EquipStatus::Missing;
        };
        let program = Program::current();
        let mut target_slot = None;
        let base_item = ItemData::get_base_item(item_id);
        match item_id.category() {
            ItemCategory::Weapon => {
                let Some(weapon) = (unsafe { repo.get::<EQUIP_PARAM_WEAPON_ST>(base_item.param_id()) }) else {
                    log::warn!("Can't equip {:?} from {}: param entry {:?} not found", item_id, index, base_item);
                    return EquipStatus::Missing;
                };
                let wep_type = weapon.wep_type();
                let mut left = false;
                let mut arrows = false;
                let bow_left = false;
                let cast_left = true;
                if wep_type == 65 || wep_type == 67 || wep_type == 69 || wep_type == 90 {
                    // Shields. Don't use enableGuard, it's true for perfume bottles
                    left = true;
                }
                else if wep_type >= 81 && wep_type <= 86 {
                    arrows = true;
                }
                else if wep_type >= 50 && wep_type <= 56 {
                    // Prefer right-hand for bows. There's also arrowSlotEquippable/boltSlotEquippable
                    // isDragonSlayer is used for Greatbows/Ballistas
                    // category 10 and 11
                    left = bow_left;
                }
                // Sorcery, incantations, pyromancy: weapon->enableMagic || weapon->enableMiracle || weapon->enableSorcery
                // Includes Carian Sorcery Sword
                else if wep_type == 57 || wep_type == 61 {
                    left = cast_left;
                }
                if arrows {
                    target_slot = if wep_type == 85 || wep_type == 86 { Some(ChrAsmSlot::Bolt1) } else { Some(ChrAsmSlot::Arrow1) };
                } else {
                    let slots = &equipment.chr_asm.equipment.selected_slots;
                    // Slots 0, 1, 2
                    let selected_index = if left { slots.left_weapon_slot } else { slots.right_weapon_slot };
                    for weapon_index in 0..3 {
                        let equip_slot = match (weapon_index, left) {
                            (1, true) => ChrAsmSlot::WeaponLeft2,
                            (1, false) => ChrAsmSlot::WeaponRight2,
                            (2, true) => ChrAsmSlot::WeaponLeft3,
                            (2, false) => ChrAsmSlot::WeaponRight3,
                            (_, true) => ChrAsmSlot::WeaponLeft1,
                            (_, false) => ChrAsmSlot::WeaponRight1,
                        };
                        if weapon_index == selected_index {
                            // Will be replaced later
                            target_slot = Some(equip_slot)
                        } else {
                            // Unequip
                            let Some((_, weapon_entry)) = get_equipment_inventory_entry(equipment, equip_slot) else {
                                continue;
                            };
                            // Avoid calling into binary for unarmed
                            if weapon_entry.item_id == ItemId::new(ItemCategory::Weapon, 110000).unwrap() {
                                continue;
                            }
                            unequip_inventory(&program, equipment, equip_slot, true);
                        }
                    }
                }
            },
            ItemCategory::Protector => {
                let Some(protector) = (unsafe { repo.get::<EQUIP_PARAM_PROTECTOR_ST>(item_id.param_id()) }) else {
                    log::warn!("Can't equip {:?} from {}: param entry not found", item_id, index);
                    return EquipStatus::Missing;
                };
                const ARMOR_SLOTS: &[ChrAsmSlot] = &[ChrAsmSlot::ProtectorHead, ChrAsmSlot::ProtectorChest, ChrAsmSlot::ProtectorHands, ChrAsmSlot::ProtectorLegs];
                target_slot = ARMOR_SLOTS.get(protector.protector_category() as usize).copied();
            }
            ItemCategory::Accessory => {
                let equipped_items = get_equipped_accessory_ids(player_game_data);
                let acc_index = self.get_oldest_equip_index(data, item_id, &self.accessory_order, &equipped_items);
                target_slot = ACCESSORY_SLOTS.get(acc_index).copied();
            }
            ItemCategory::Goods => {
                // Having slots qualifies something as a spell
                if data.spell_slots.contains_key(&item_id) {
                    // Equip spell
                    log::info!("Equipping {:?} from inventory {} as spell", item_id, index);
                    return self.equip_spell(&program, data, &mut player_game_data, item_id);
                } else if data.crystal_tears.contains(&item_id) {
                    // Equip physick
                    log::info!("Equipping {:?} from inventory {} as physick tear", item_id, index);
                }
            }
            _ => (),
        }
        let Some(target_slot) = target_slot else {
            return EquipStatus::NotNeeded;
        };
        // Can only equip to slots from regular inventory
        if log_equip {
            log::info!("Equipping {:?} from inventory {} in slot {:?}, at {:p} {:p}", item_id, index, target_slot, player_game_data, inventory);
        }
        // Don't double-equip something, since the game unequips it
        if let Some((exist_index, _)) = get_equipment_inventory_entry(equipment, target_slot) && exist_index == index {
            log::warn!("Won't double-equip at slot {}", index);
            return EquipStatus::Done;
        };
        // Actual equip
        if let Ok(world_chr_man) = unsafe { WorldChrMan::instance() }
            && let Some(main_player) = &mut world_chr_man.main_player {
            clear_action_flag(main_player);
        }
        equip_inventory(&program, target_slot, index);
        // Check/update after equip
        if let Some((exist_index, _)) = get_equipment_inventory_entry(equipment, target_slot) && exist_index == index {
            return EquipStatus::Done;
        };
        EquipStatus::Failed
    }

    pub fn equip_spell(&mut self, program: &Program, data: &ItemData, player_game_data: &mut PlayerGameData, item_id: ItemId) -> EquipStatus {
        let Some(slots) = data.spell_slots.get(&item_id).copied() else {
            return EquipStatus::NotNeeded;
        };
        let max_slots = player_game_data.effective_unlocked_magic_slots;
        if slots > max_slots {
            return EquipStatus::NotNeeded;
        }
        let mut spells = get_equipped_spell_ids(player_game_data);
        if let Some(exist_index) = spells.iter().position(|equipped| equipped.as_valid() == Some(item_id)) {
            player_game_data.equipment.equip_magic_data.selected_slot = exist_index as i32;
            return EquipStatus::Done;
        }
        let mut used_slots = get_used_spell_slots(data, &spells);
        log::info!("Spell {:?} has {} slots, {}/{} used, {} in order. Should remove: {}\n", item_id, slots, used_slots, max_slots, self.spell_order.len(), max_slots - used_slots < slots);
        loop {
            if max_slots - used_slots < slots {
                // Not enough space, remove spell
                let Some(&remove_id) = self.spell_order.first() else {
                    break;
                };
                let Some(remove_index) = spells.iter().position(|equipped| equipped.as_valid() == Some(remove_id)) else {
                    break;
                };
                unequip_magic(program, &player_game_data.equipment.equip_magic_data, remove_index as u32);
                let spell_indices = get_item_inventory_indices(&player_game_data.equipment.equip_inventory_data, get_equipped_spell_ids(player_game_data));
                Self::update_equip_order(player_game_data, &mut self.spell_order, &spell_indices);
                spells = get_equipped_spell_ids(player_game_data);
                let new_used_slots = get_used_spell_slots(data, &spells);
                let new_free_slots = used_slots - new_used_slots;
                used_slots = new_used_slots;
                if new_free_slots <= 0 {
                    break;
                }
            } else {
                // Space to equip spell
                let Some(insert_index) = spells.iter().position(|equipped| !equipped.is_valid()) else {
                    break;
                };
                equip_magic(program, &player_game_data.equipment.equip_magic_data, insert_index as u32, item_id.param_id() as i32);
                player_game_data.equipment.equip_magic_data.selected_slot = insert_index as i32;
                return EquipStatus::Done;
            }
        }
        EquipStatus::Failed
    }

    pub fn update(&mut self) {
        let Some(player_game_data) = (unsafe { PlayerGameData::main_instance() }) else {
            return;
        };
        let inventory = &player_game_data.equipment.equip_inventory_data;
        let accessory_indices = get_equipped_accessory_indices(player_game_data);
        Self::update_equip_order(player_game_data, &mut self.accessory_order, &accessory_indices);
        let spell_indices = get_item_inventory_indices(inventory, get_equipped_spell_ids(player_game_data));
        Self::update_equip_order(player_game_data, &mut self.spell_order, &spell_indices);
        let tear_indices = get_item_inventory_indices(inventory, get_equipped_physick_ids(player_game_data));
        Self::update_equip_order(player_game_data, &mut self.tear_order, &tear_indices);
    }

    fn get_oldest_equip_index(&self, data: &ItemData, equip_item: ItemId, equip_order: &Vec<ItemId>, equipped_items: &Vec<OptionalItemId>) -> usize {
        // If item already exists, use existing index (same group if talisman)
        log::info!("Equipped {:?} vs order {:?} for {:?}", equipped_items, equip_order, equip_item);    
        for (i, item_id) in equipped_items.iter().enumerate() {
            if let Some(item_id) = item_id.as_valid() {
                if equip_item == item_id {
                    return i;
                }
                if let Some(equip_group) = data.accessory_groups.get(&equip_item)
                    && let Some(item_group) = data.accessory_groups.get(&item_id)
                    && equip_group == item_group {
                    return i;
                }
            }
        }
        // Otherwise return empty slot
        if let Some(empty_index) = equipped_items.iter().position(|item_id| !item_id.is_valid()) {
            return empty_index;
        }
        // Validate if equip_order is still valid (matches equipped items) since it was last updated
        let equipped_items: Vec<ItemId> = equipped_items.iter().filter_map(|item_id| item_id.as_valid()).collect();
        if equipped_items.len() != equip_order.len() || equipped_items.iter().any(|item_id| !equip_order.contains(item_id)) {
            return 0;
        }
        return if let Some(replace_id) = equip_order.first()
            && let Some(replace_index) = equipped_items.iter().position(|item_id| item_id == replace_id) {
            replace_index
        } else {
            0
        };
    }

    fn update_equip_order(player_game_data: &PlayerGameData, equip_order: &mut Vec<ItemId>, equipped_indices: &Vec<i32>) {
        let mut equipped_items = HashMap::new();
        let inventory = &player_game_data.equipment.equip_inventory_data;
        for &index in equipped_indices {
            if let Some(entry) = get_inventory_entry(inventory, index) {
                equipped_items.insert(entry.item_id, entry.sort_id);
            }
        }
        // Remove unequipped
        let original_len = equip_order.len();
        equip_order.retain(|item_id| equipped_items.contains_key(item_id));
        // Add equipped but untracked
        let mut untracked_entries: Vec<ItemId> = equipped_items.keys().filter(|item_id| !equip_order.contains(item_id)).cloned().collect();
        untracked_entries.sort_by_key(|item_id| equipped_items.get(item_id).unwrap_or(&u32::MAX));
        let changed = equip_order.len() != original_len || untracked_entries.len() > 0;
        equip_order.append(&mut untracked_entries);
        if changed {
            // TODO: Add item names maybe
            log::info!("Item order: {:?}", equip_order);
        }
    }

    pub fn give_item_as_lot(&self, item_id: &ItemId, quantity: u8, gem: &Option<ItemId>) {
        // Done after item data is already loaded from params
        let repo = unsafe { FD4ParamRepository::instance() }.unwrap();
        let lot_id = 998990;
        let custom_id = 333;
        let mut using_gem = false;
        if let Some(gem) = gem && gem.category() == ItemCategory::Gem && item_id.category() == ItemCategory::Weapon {
            // TODO: Try to reuse repo if possible, but getting mutable row is an unsafe borrow
            let repo = unsafe { FD4ParamRepository::instance() }.unwrap();
            if let Some(wep) = unsafe { repo.get_mut::<EQUIP_PARAM_CUSTOM_WEAPON_ST>(custom_id) } {
                let level = item_id.param_id() % 100;
                wep.set_base_wep_id((item_id.param_id() - level) as i32);
                wep.set_reinforce_lv(level as u8);
                wep.set_gem_id(gem.param_id() as i32);
                using_gem = true;
            };
        }
        // ItemLotParam_enemy is 0x14, ItemLotParam_map is 0x15
        // TODO: Use SoloParamRepository which seems to do this correctly now
        let Some(lot_param) = unsafe { repo.res_cap_holder_mut() }.entries_mut().filter(|p| p.struct_name() == ITEMLOT_PARAM_ST::NAME).nth(1) else {
            panic!("ItemLotParam_map not loaded");
        };
        let Some(lot) = (unsafe { lot_param.get_mut::<ITEMLOT_PARAM_ST>(lot_id) }) else {
            log::info!("Row {} not found", lot_id);
            return;
        };
        // Non-default fields: lotItemId01=17000, lotItemCategory01=1, u16 lotItemBasePoint01=1000, u8 lotItemNum01=1, canExecByFriendlyGhost=1
        lot.set_can_exec_by_friendly_ghost(1);
        lot.set_can_exec_by_hostile_ghost(1);
        lot.set_lot_item_base_point01(1000);
        lot.set_lot_item_rarity(-1);
        lot.set_game_clear_offset(-1);
        if using_gem {
            lot.set_lot_item_id01(custom_id as i32);
            lot.set_lot_item_category01(6);
            lot.set_lot_item_num01(1);
        } else {
            lot.set_lot_item_id01(item_id.param_id() as i32);
            lot.set_lot_item_category01(match item_id.category() {
                ItemCategory::Weapon => 2,
                ItemCategory::Protector => 3,
                ItemCategory::Accessory => 4,
                ItemCategory::Goods => 1,
                ItemCategory::Gem => 5,
            });
            lot.set_lot_item_num01(quantity);
        }
        emedf::award_item_lot(lot_id as i32);
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn expected_offsets() {
        assert_eq!(
            0x1C,
            memoffset::offset_of!(eldenring::cs::EquipInventoryData, items_data)
                + memoffset::offset_of!(eldenring::cs::InventoryItemsData, key_items_capacity));
    }
}
