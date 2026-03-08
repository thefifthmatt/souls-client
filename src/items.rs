use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;
use eldenring::cs::{CSTaskGroupIndex, CSTaskImp, ItemCategory, ItemId, PlayerGameData, WorldChrManDbgFlags};
use eldenring::fd4::FD4TaskData;
use eldenring::util::system::wait_for_system_init;
use fromsoftware_shared::{FromStatic, SharedTaskImpExt};
use serde::{Deserialize, Serialize};

pub mod data;
pub mod equip;

use crate::client::Client;
use crate::game::PlayerGameDataExt;
use crate::items::data::UpgradeType;
use crate::items::equip::{EquipHandler, EquipStatus};
use crate::{
    items::data::ItemData,
};

pub struct ItemUpdater {
    data: OnceLock<ItemData>,
    // Not used at present
    _item_send: mpsc::Sender<ItemRequest>,
    // Keyed by base id
    pending: Mutex<HashMap<ItemId, PendingEquip>>,
    equip_handler: Mutex<EquipHandler>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct ItemRequest {
    items: Vec<ItemDesc>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct ItemDesc {
    name: String,
    // -1 or missing to autoupgrade
    level: Option<i32>,
    quantity: Option<i32>,
    gem: Option<String>,
    #[serde(default)]
    equip: bool,
    #[serde(default)]
    ignore_level: bool,
}

#[derive(Clone, Debug)]
struct PendingEquip {
    desc: ItemDesc,
    real_id: Option<ItemId>,
    gem_id: Option<ItemId>,
    tries: u32,
}

static INSTANCE: OnceLock<Arc<ItemUpdater>> = OnceLock::new();

impl ItemUpdater {
    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    pub fn initialize() {
        if INSTANCE.get().is_some() {
            panic!("Already initialized");
        }
        let (item_send, item_recv) = mpsc::channel();
        let updater = Arc::new(ItemUpdater {
            data: OnceLock::new(),
            _item_send: item_send,
            pending: Mutex::new(HashMap::new()),
            equip_handler: Mutex::new(EquipHandler::default()),
        });
        let other = Arc::clone(&updater);
        std::thread::spawn(move || other.load_data());
        let other = Arc::clone(&updater);
        std::thread::spawn(move || other.run_task(item_recv));
        INSTANCE.set(updater).ok().expect("Already initialized");
    }

    pub fn give(&self, req: &ItemRequest) -> Result<(), String> {
        let errs: Vec<String> = req.items.iter()
            .map(&|item| self.give_item(item))
            .filter_map(|result| result.err())
            .collect();
        if errs.len() == 0 { Ok(()) } else { Err(errs.join("; ")) }
    }

    pub fn give_item(&self, item: &ItemDesc) -> Result<(), String> {
        let Some(data) = self.data.get() else {
            return Err("Item data not initialized".to_string());
        };
        let Some(item_id) = data.item_names.get_by_right(&item.name) else {
            return Err(format!("No item found for {}", item.name));
        };
        let mut gem_id = None;
        if let Some(gem_name) = &item.gem {
            gem_id = data.item_names.get_by_right(gem_name).copied();
            if gem_id.is_none() {
                return Err(format!("No gem found for {}", gem_name));
            }
        }
        let mut pending = self.pending.lock().unwrap();
        if pending.contains_key(&item_id) {
            // In progress
            return Ok(());
        }
        let equip = PendingEquip {
            desc: item.clone(),
            real_id: None,
            gem_id: gem_id,
            tries: 0,
        };
        log::info!("Item {:?}", equip);
        pending.insert(item_id.clone(), equip);

        Ok(())
    }

    pub fn set_infinite_arrows(&self, enabled: bool) {
        if let Ok(debug_flags) = unsafe { WorldChrManDbgFlags::instance() } {
            debug_flags.all_no_arrow_consume = enabled;
        }
    }

    pub fn dump_items(&self) -> std::io::Result<()> {
        let Some(data) = self.data.get() else {
            return Ok(());
        };
        let client_data = data.for_client();
        let file = File::create("items.ts")?;
        let mut writer = BufWriter::new(file);
        writeln!(&mut writer, "export const ITEMS = (")?;
        serde_json::to_writer_pretty(&mut writer, &client_data)?;
        writeln!(&mut writer, ");")?;
        writer.flush()
    }

    fn load_data(&self) {
        loop {
            if let Some(data) = ItemData::try_new() {
                // log::info!("{:?}", data);
                self.data.set(data).ok().expect("Param data already initialized");
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    fn get_auto_upgraded_weapon(data: &ItemData, item_id: ItemId) -> Option<ItemId> {
        if item_id.category() != ItemCategory::Weapon {
            return None;
        }
        let Some(player_game_data) = (unsafe { PlayerGameData::main_instance() }) else {
            return None;
        };
        let player_max = player_game_data.matching_weapon_level.clamp(0, 25);
        let reg_level = player_max;
        let new_level = match data.upgrade_types.get(&item_id) {
            Some(UpgradeType::Regular) => reg_level,
            Some(UpgradeType::Somber) => ItemData::regular_to_somber_level(reg_level),
            _ => return None,
        };
        ItemId::new(ItemCategory::Weapon, item_id.param_id() + new_level as u32).ok()
    }

    fn get_upgraded_weapon(data: &ItemData, item_id: ItemId, level: i32) -> Option<ItemId> {
        if item_id.category() != ItemCategory::Weapon {
            return None;
        }
        let new_level = match data.upgrade_types.get(&item_id) {
            Some(UpgradeType::Regular) => level.clamp(0, 25),
            Some(UpgradeType::Somber) => level.clamp(0, 10),
            _ => return None,
        };
        ItemId::new(ItemCategory::Weapon, item_id.param_id() + new_level as u32).ok()
    }

    // Run in thread
    fn run_task(self: Arc<Self>, _item_recv: mpsc::Receiver<ItemRequest>) {
        wait_for_system_init(&fromsoftware_shared::Program::current(), Duration::MAX)
            .expect("Could not await system init.");
        // Needed without modloader
        std::thread::sleep(Duration::from_secs(3));

        let cs_task = unsafe { CSTaskImp::instance().expect("Task system not initialized") };
        cs_task.run_recurring(move
            |_: &FD4TaskData| {
                // Probably shouldn't go here but it should be safe enough
                let client = Client::get();
                if let Some(player_game_data) = unsafe { PlayerGameData::main_instance() } {
                    client.set_player(&player_game_data.character_name());
                }

                let Some(data) = self.data.get() else {
                    return;
                };
                let mut equip_handler = self.equip_handler.lock().unwrap();
                {
                    let mut pending = self.pending.lock().unwrap();
                    let mut finished = Vec::new();
                    for (&item_id, state) in pending.iter_mut() {
                        let mut complete = false;
                        if let Some(real_id) = state.real_id {
                            let status = equip_handler.equip_item(data, real_id, state.tries == 0, state.desc.ignore_level);
                            if status == EquipStatus::Missing {
                                log::warn!("Can't find {:?} in inventory, so not equipping", real_id);
                            }
                            // If it's missing, don't try giving again
                            if status == EquipStatus::Failed && state.tries < 60 * 60 {
                                state.tries += 1;
                            } else {
                                complete = true;
                            }
                        } else {
                            let real_id = match state.desc.level {
                                Some(level) if level >= 0 => Self::get_upgraded_weapon(data, item_id, level),
                                _ => Self::get_auto_upgraded_weapon(data, item_id),
                            };
                            let real_id = real_id.unwrap_or(item_id);
                            let quantity = state.desc.quantity.unwrap_or(1).clamp(1, 100) as u8;
                            log::info!("Giving {:?} {}x as {:?}", item_id, quantity, real_id);
                            equip_handler.give_item_as_lot(&real_id, quantity, &state.gem_id);
                            if state.desc.equip {
                                state.real_id = Some(real_id.clone());
                            } else {
                                complete = true;
                            }
                        }
                        if complete {
                            finished.push(item_id.clone());
                        }
                    }
                    for item_id in finished {
                        pending.remove(&item_id);
                    }
                }
                equip_handler.update();
            },
            // Idk
            CSTaskGroupIndex::HavokWorldUpdate_Pre,
        );
    }

}
