use std::collections::{HashMap, HashSet};
use bimap::BiMap;

use eldenring::{cs::{ItemCategory, ItemId}, fd4::FD4ParamRepository, param::{EQUIP_PARAM_ACCESSORY_ST, EQUIP_PARAM_GEM_ST, EQUIP_PARAM_GOODS_ST, EQUIP_PARAM_PROTECTOR_ST, EQUIP_PARAM_WEAPON_ST, MAGIC_PARAM_ST, ParamDef}};
use fromsoftware_shared::{FromStatic, singleton};
use windows::core::PCWSTR;
use serde::{Deserialize, Serialize};

use crate::{program::Program, rva::GET_MESSAGE_RVA};


#[derive(Debug, PartialEq, Eq, Hash)]
pub enum UpgradeType {
    Regular,
    Somber,
    SpiritAsh,
}

// All static item data for giving/auto-equip/auto-upgrade purposes
#[derive(Debug, Default)]
pub struct ItemData {
    pub item_names: BiMap<ItemId, String>,
    pub item_cats: HashMap<ItemId, ClientCat>,
    pub upgrade_types: HashMap<ItemId, UpgradeType>,
    pub crystal_tears: HashSet<ItemId>,
    pub spell_slots: HashMap<ItemId, u32>,
    pub _multi_items: HashSet<ItemId>,
    pub accessory_groups: HashMap<ItemId, i32>,
    // For auto building, also add reqs and weights
}

type ClientItemData = Vec<ClientItem>;

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct ClientItem {
    #[serde(skip)]
    pub item_id: Option<ItemId>,
    pub name: String,
    pub cat: ClientCat,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub enum ClientCat {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "weapon")]
    Weapon,
    #[serde(rename = "arrow")]
    Arrow,
    #[serde(rename = "armor")]
    Armor,
    #[serde(rename = "talisman")]
    Talisman,
    #[serde(rename = "spell")]
    Spell,
    #[serde(rename = "runes")]
    Runes,
    #[serde(rename = "key_item")]
    KeyItem,
    #[serde(rename = "crystal_tear")]
    CrystalTear,
    #[serde(rename = "upgrade_material")]
    UpgradeMaterial,
    #[serde(rename = "self_buff")]
    SelfBuff,
    #[serde(rename = "weapon_buff")]
    WeaponBuff,
    #[serde(rename = "reusable_tool")]
    ReusableTool,
    #[serde(rename = "throwable")]
    Throwable,
    #[serde(rename = "spirit_ash")]
    SpiritAsh,
    #[serde(rename = "crafting_material")]
    CraftingMaterial,
    #[serde(rename = "notes")]
    Notes,
}

impl ItemData {
    pub fn try_new() -> Option<Self> {
        let Ok(repo) = (unsafe { FD4ParamRepository::instance() }) else {
            return None;
        };
        let Some(_) = unsafe { repo.res_cap_holder() }.entries().find(|e| e.struct_name() == "THROW_PARAM_ST") else {
            return None;
        };
        let mut data = ItemData::default();
        data.init(repo);
        Some(data)
    }

    pub fn get_base_item(item_id: ItemId) -> ItemId {
        if item_id.category() != ItemCategory::Weapon {
            return item_id;
        }
        let id = item_id.param_id();
        ItemId::new(ItemCategory::Weapon, id - (id % 100)).unwrap()
    }

    pub fn regular_to_somber_level(level: u8) -> u8 {
        const LEVELS: [u8; 26] = [
            0, 0, 1, 1, 1,
            2, 2, 3, 3, 3,
            4, 4, 5, 5, 5,
            6, 6, 7, 7, 7,
            8, 8, 9, 9, 9, 10
        ];
        LEVELS[level as usize]
    }

    pub fn for_client(&self) -> ClientItemData {
        let mut items: Vec<ClientItem> = self.item_names.iter().filter(|(item_id, _)| {
            // Filter out infusions
            item_id.category() != ItemCategory::Weapon || item_id.param_id() % 10000 == 0
        }).map(|(item_id, item_name)| {
            let cat = self.item_cats.get(item_id).cloned().unwrap_or(ClientCat::None);
            ClientItem { item_id: Some(item_id.clone()), name: item_name.clone(), cat }
        }).collect();
        // Probably doesn't need to be BiBTreeMap, just sort here
        items.sort_by_key(|e| e.item_id.map(|i| i.into_inner()));
        items
    }

    fn init(&mut self, repo: &FD4ParamRepository) {
        self.iterate_items::<EQUIP_PARAM_WEAPON_ST>(repo, ItemCategory::Weapon, 11, 310, &|data, id, row| {
            // Unarmed
            if id.param_id() == 110000 { return false; }
            if row.origin_equip_wep25() > 0 {
                data.upgrade_types.insert(id, UpgradeType::Regular);
            } else if row.origin_equip_wep10() > 0 {
                data.upgrade_types.insert(id, UpgradeType::Somber);
            }
            data.item_cats.insert(id, ClientCat::Weapon);
            true
        });
        self.iterate_items::<EQUIP_PARAM_PROTECTOR_ST>(repo, ItemCategory::Protector, 12, 313, &|data, id, _| {
            // Hairstyles and none armor
            if id.param_id() < 40000 { return false; }
            data.item_cats.insert(id, ClientCat::Armor);            
            true
        });
        self.iterate_items::<EQUIP_PARAM_ACCESSORY_ST>(repo, ItemCategory::Accessory, 13, 316, &|data, id, row| {
            if row.accessory_group() > 0 {
                data.accessory_groups.insert(id, row.accessory_group().into());
            }
            data.item_cats.insert(id, ClientCat::Talisman);
            true
        });
        self.iterate_items::<EQUIP_PARAM_GOODS_ST>(repo, ItemCategory::Goods, 10, 319, &|data, id, row| {
            if row.use_limit_summon_buddy() == 1 {
                if id.param_id() % 100 == 0 {
                    data.upgrade_types.insert(id, UpgradeType::SpiritAsh);
                } else {
                    return false;
                }
            }
            if row.goods_type() == 10 {
                data.crystal_tears.insert(id);
            }
            true
        });
        self.iterate_items::<EQUIP_PARAM_GEM_ST>(repo, ItemCategory::Gem, 35, 322, &|_, _, _| {
            true
        });
        // Magic param
        let Some(param) = unsafe { repo.res_cap_holder() }.entries().find(|e| e.struct_name() == MAGIC_PARAM_ST::NAME) else {
            panic!("{} not loaded", MAGIC_PARAM_ST::NAME);
        };
        for index in 0..param.data.row_count() {
            let Some((row_id, row)) = (unsafe { param.get_index::<MAGIC_PARAM_ST>(index) }) else {
                continue;
            };
            // Stonesword Key, I don't remember what prevents it from getting equipped normally
            if row_id == 8000 {
                continue;
            }
            // GoodsType can be used for spells, but having a Magic is the main important thing
            let Ok(item_id) = ItemId::new(ItemCategory::Goods, row_id) else {
                continue;
            };
            if row.slot_length() > 0 {
                self.spell_slots.insert(item_id, row.slot_length().into());
            }
        }
        log::info!("Loaded item data with {} named items", self.item_names.len());
    }

    fn iterate_items<P: ParamDef>(
            &mut self, repo: &FD4ParamRepository, cat: ItemCategory, fmg_id: u32, dlc_fmg_id: u32, process: &dyn Fn(&mut Self, ItemId, &P) -> bool) {
        // TODO: Use SoloParamRepository new iteration/index API
        let Some(param) = unsafe { repo.res_cap_holder() }.entries().find(|e| e.struct_name() == P::NAME) else {
            panic!("{} not loaded", P::NAME);
        };
        let msg_repo = unsafe { MsgRepository::instance().unwrap() };
        // Should msg_id be u32?
        let get_message = unsafe { Program::current()
            .derva_ptr::<unsafe extern "C" fn(&MsgRepository, u32, u32, i32) -> PCWSTR>(GET_MESSAGE_RVA) };
        let get_valid_name = |wstr: PCWSTR| -> Result<String, String> {
            if wstr.is_null() {
                return Err("Null".to_string());
            }
            let Result::Ok(str) = (unsafe { wstr.to_string() }) else {
                return Err("Invalid encoding".to_string());
            };
            if str.is_empty() || str.contains("ERROR") {
                Err("Error item".to_string())
            } else {
                Ok(str)
            }
        };
        let lookup_name = |row_id| {
            match get_valid_name(unsafe { get_message(msg_repo, 0, fmg_id, row_id) }) {
                Err(err) => match get_valid_name(unsafe { get_message(msg_repo, 0, dlc_fmg_id, row_id) }) {
                    Err(err2) => Err(format!("{}, {}", err, err2)),
                    val => val,
                }
                val => val,
            }
        };
        log::info!("{} with {} rows", P::NAME, param.data.row_count());
        for index in 0..param.data.row_count() {
            let Some((row_id, row)) = (unsafe { param.get_index::<P>(index) }) else {
                log::info!("{} {} not found", P::NAME, index);
                continue;
            };
            let name = match lookup_name(row_id as i32) {
                Ok(name) => name,
                Err(_) => {
                    // log::error!("{} name not found: {}", row_id, err);
                    continue;
                },
            };
            let Ok(item_id) = ItemId::new(cat, row_id) else {
                continue;
            };
            // log::info!("{} {} found {} -> {}. {} {}", P::NAME, index, row_id, name, self.item_names.contains_left(&item_id), self.item_names.contains_right(&name));
            if self.item_names.contains_left(&item_id) || self.item_names.contains_right(&name) {
                continue;
            }
            if process(self, item_id, row) {
                self.item_names.insert(item_id, name);
            }
        }
    }
}

#[repr(C)]
#[singleton("MsgRepository")]
pub struct MsgRepository {}
