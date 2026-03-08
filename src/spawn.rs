use eldenring::cs::{CSSessionManager, CSTaskGroupIndex, CSTaskImp, ChrIns, ChrSet, LobbyState, NetChrSync, P2PEntityHandle};
use eldenring::fd4::FD4TaskData;
use eldenring::position::BlockPosition;
use eldenring::util::system::wait_for_system_init;
use std::collections::HashMap;
use std::error::Error;
use std::f32::consts::PI;
use std::hash::{Hasher, DefaultHasher};
use std::ptr::NonNull;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, mpsc};
use std::time::Duration;
// use tokio::io::{AsyncWriteExt, Result};
use eldenring::{
    cs::{BlockId, ChrDebugSpawnRequest, FieldArea, FieldInsHandle, FieldInsSelector, WorldChrMan},
    position::HavokPosition,
};
use fromsoftware_shared::{F32Vector4, FromStatic, SharedTaskImpExt};
use serde::{Deserialize, Serialize};

use crate::arenas::MAIN_ARENAS;
use crate::client::{Client, NameClaim};
use crate::hooks::hook_spawn;
use crate::rva::{
    GLOBAL_FIELD_AREA, NET_CHR_SYNC_SETUP_ENTITY_1, NET_CHR_SYNC_SETUP_ENTITY_2,
    NET_CHR_SYNC_SETUP_ENTITY_3, REMOVE_CHR_INS, SPAWN_CHR,
};
use crate::ui::WidgetChannel;
use crate::{Program, event::emedf};

// Must be >80 for health sync to work, also avoid seamless 127 torrents and buddies
pub const CHR_SET_CAPACITY: u32 = 320; // 480 for bigger
pub const CHR_SET_START: u32 = 160;  // 128 + 32

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct SpawnRequest {
    pub name: Option<String>,
    pub spawn: Option<EnemySpawn>,
    pub enemies: Option<Vec<EnemySpawn>>,
    pub inits: Option<Vec<EnemyInit>>,
    pub claim: Option<NameClaim>,
    #[serde(default)]
    pub sequential: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct EnemyInit {
    pub id: u32,
    pub args: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct EnemySpawn {
    pub entity_id: Option<u32>,
    pub model: String,
    pub npc_param: i32,
    pub npc_think_param: i32,
    pub chara_init_param: Option<i32>,
    #[serde(default)]
    pub mount: bool,
    pub index: Option<u32>,
}

pub struct EnemySpawner {
    spawn_send: Option<mpsc::Sender<SpawnRequest>>,
    // TODO also track spawned enemies
}

static INSTANCE: OnceLock<Arc<EnemySpawner>> = OnceLock::new();

impl EnemySpawner {
    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    pub fn initialize(enable: bool) {
        if INSTANCE.get().is_some() {
            panic!("Already initialized");
        }
        let spawner;
        if enable {
            let (spawn_send, spawn_recv) = mpsc::channel();
            spawner = Arc::new(EnemySpawner {
                spawn_send: Some(spawn_send),
            });
            let other = Arc::clone(&spawner);
            std::thread::spawn(move || other.run_task(spawn_recv));
            hook_spawn();
        } else {
            spawner = Arc::new(EnemySpawner {
                spawn_send: None,
            });
        }
        INSTANCE.set(spawner).ok().expect("Already initialized");
    }

    pub fn spawn_req(&self, item: &SpawnRequest) {
        // Convert everything to main request format
        if let Some(enemy) = &item.spawn {
            self.spawn_enemy(enemy.clone());
        } else if item.enemies.is_some() {
            // Send the entire thing
            self.send_spawn(item);
        } else if let Some(str) = &item.name {
            // Not very useful at this point
            self.spawn_name(str);
        }
    }

    fn send_spawn(&self, req: &SpawnRequest) {
        if let Some(spawn_send) = &self.spawn_send {
            if let Err(e) = spawn_send.send(req.clone()) {
                log::error!("Can't send spawn: {e}");
            }
        }
    }

    // TODO: Return error, or possibly move this elsewhere
    pub fn spawn_name(&self, name: &str) {
        let enemy;
        if name == "Crab" {
            enemy = EnemySpawn {
                model: "c2275".to_string(),
                npc_param: 22750020,
                npc_think_param: 22750000,
                ..Default::default()
            };
        } else if name == "Messmer" {
            enemy = EnemySpawn {
                model: "c5130".to_string(),
                npc_param: 51300099,
                npc_think_param: 51300000,
                ..Default::default()
            };
        } else {
            return;
        }
        self.spawn_enemy(enemy);
    }

    pub fn spawn_enemy(&self, spawn: EnemySpawn) {
        self.send_spawn(&SpawnRequest {
            enemies: Some(vec![spawn]),
            ..Default::default()
        });
    }

    // Run in thread
    pub fn run_task(self: Arc<Self>, spawn_recv: mpsc::Receiver<SpawnRequest>) {
        // Kick off new thread.
        wait_for_system_init(&fromsoftware_shared::Program::current(), Duration::MAX)
            .expect("Could not await system init.");
        // Needed without modloader
        std::thread::sleep(Duration::from_secs(3));

        let slot_count: Mutex<HashMap<u32, i32>> = Mutex::new(HashMap::new());
        let pos_index: Mutex<u32> = Mutex::new(0);

        let cs_task = unsafe { CSTaskImp::instance().expect("Task system not initialized") };
        cs_task.run_recurring(
            move |_: &FD4TaskData| {
                let (block_id, arena) = get_current_arena();
                let client = Client::get();

                'main: while let Ok(req) = spawn_recv.try_recv() {
                    // Check if loaded after consuming the request, to avoid mass-spawning when loading into such a map
                    // Could also check player position but this is fine
                    let Some(arena) = &arena else {
                        log::info!("Can't spawn {:?}: player in {:?}, not an arena map", req.name, block_id);
                        continue;
                    };
                    let mut first = None;
                    let Some(enemies) = &req.enemies else { continue };
                    let mut mapping: HashMap<u32, u32> = HashMap::new();
                    let start_index = enemies.first().map(|e| e.index).flatten().unwrap_or_else(|| {
                        let mut pos_index = pos_index.lock().unwrap();
                        *pos_index += 1;
                        *pos_index
                    });
                    for enemy in enemies {
                        let Some(chr_ins) = self.spawn_from_task(&enemy, arena, &req, start_index) else {
                            // Likewise give up here
                            log::error!("Can't init {:?}", &enemy);
                            continue 'main;
                        };
                        let chr_ins = unsafe { chr_ins.as_ref() };
                        let temp_entity_id = chr_ins.event_entity_id;
                        log::info!("Spawned {:?} -> {} #{}: {:p}", &enemy, temp_entity_id, chr_ins.field_ins_handle.selector.index(), chr_ins);
                        if let Some(entity_id) = enemy.entity_id {
                            mapping.insert(entity_id, temp_entity_id);
                        }
                        let claim = if first.is_none() { req.claim.clone() } else { None };
                        let claim = claim.unwrap_or_else(|| NameClaim::new("".to_string(), 0));
                        log::info!("Assigning {} -> {:?}", temp_entity_id, claim);
                        client.set_spawn_name(temp_entity_id, claim);
                        match first {
                            None => {
                                let _ = first.insert(chr_ins);
                            }
                            Some(rider) if enemy.mount => {
                                mount_chr(rider, chr_ins);
                            }
                            _ => (),
                        }
                        // Unfortunately this doesn't work
                        // emedf::change_character_patrol_behavior(temp_entity_id, 835310000);
                        // TODO: Multiplayer buff
                        emedf::activate_multiplayer_dependant_buffs(temp_entity_id);
                        emedf::character_collision(temp_entity_id, 1);
                    }
                    if let Some(inits) = req.inits {
                        let mut count_map = slot_count.lock().unwrap();
                        for init in inits {
                            let event_args: Vec<i32> = init
                                .args
                                .into_iter()
                                .map(|id| *mapping.get(&id).unwrap_or(&id) as i32)
                                .collect();
                            let slot_id = count_map.entry(init.id).or_insert(0);
                            log::info!("Init {} {} - {:?}", slot_id, init.id, &event_args);
                            emedf::initialize_common_event(*slot_id, init.id, &event_args);
                            *slot_id += 1;
                        }
                    }
                    // Some emevd testing
                    // emedf::display_banner(33);
                    // emedf::character_immortality(10000, 1);
                    // emedf::character_invincibility(10000, 1);
                    // emedf::character_collision(10000, 0);
                    // unsafe { crate::event::emk_system::CSEmevdRepository::instance() }.ok().unwrap().print_rescaps();
                    // emedf::initialize_common_event(1096990000, &vec![32]);  // Display banner
                    let enemy_name = req.name.as_deref().unwrap_or("an enemy");
                    let enemy_name = enemy_name.replace(" (Regular)", "");
                    let text = match &req.claim {
                        Some(claim) => {
                            let mut amount = "".to_string();
                            if claim.amount > 0 {
                                let dollars = claim.amount / 100;
                                let cents = claim.amount % 100;
                                if cents == 0 {
                                    amount = format!(" with ${}", dollars);
                                } else {
                                    amount = format!(" with ${}.{:02}", dollars, cents);
                                }
                            }
                            format!("{} spawned {}{}", claim.name, enemy_name, amount)
                        },
                        None => format!("Spawned {}", enemy_name),
                    };
                    WidgetChannel::get().show_toast(&text);
                }
            },
            // CSTaskGroupIndex::ChrIns_PostPhysics,
            CSTaskGroupIndex::WorldChrMan_Respawn,
        );
    }

    fn spawn_from_task(&self, enemy: &EnemySpawn, arena: &Arena, req: &SpawnRequest, start_index: u32) -> Option<NonNull<ChrIns>> {
        // Overall position is based on first index
        let pos;
        let orientation;
        if let Some(radius) = arena.radius {
            // In this mod, facing towards center
            let angle_rad = if req.sequential {
                // 0 to radius*2PI, 0 to 2PI, increase by x means increase by x/radius
                let spacing: f32 = 2.0;
                (start_index as f32) * spacing / radius
            } else {
                // Otherwise, make it somewhat random, a deterministic nonlinear function of start_index
                let mut hasher = DefaultHasher::new();
                hasher.write_u32(start_index);
                let hash = hasher.finish() as u16;
                let frac = (hash as f32) / (u16::MAX as f32);
                frac * 2.0 * PI
            };
            let (z, x) = angle_rad.sin_cos();
            pos = BlockPosition::from_xyz(arena.pos.x + x * radius, arena.pos.y, arena.pos.z + z * radius);
            orientation = PI / 2.0 - angle_rad;
        } else {
            pos = arena.pos;
            orientation = arena.orientation.unwrap_or(0.0);
        }
        spawn_mob(
            enemy.index,
            &arena.map,
            &pos,
            orientation,
            enemy.npc_param,
            enemy.npc_think_param,
            enemy.chara_init_param.unwrap_or(-1),
            &enemy.model,
        )
        .unwrap()
    }
}

fn get_current_arena() -> (Option<BlockId>, Option<Arena>) {
    let Ok(world_chr_man) = (unsafe { WorldChrMan::instance() }) else {
        return (None, None);
    };
    let Some(player) = world_chr_man.main_player.as_ref() else {
        return (None, None);
    };
    let block_id = player.block_origin_override;
    let arena = MAIN_ARENAS.iter().find(|a| a.map == block_id).cloned().or_else(|| {
        // Experimental mode to spawn on the fly, should ideally be specified in request
        // Without this the spawn is ignored
        let pos = player.block_position;
        let orientation = player.module_container.physics.orientation_euler.1;
        Some(Arena::new_for_test("Player", block_id, pos, orientation))
    });
    (Some(block_id), arena)
}

// Definitions are just kept locally in code
#[derive(Debug, Clone)]
pub struct Arena {
    #[allow(unused)]
    pub name: String,
    pub map: BlockId,
    pub pos: BlockPosition,
    pub radius: Option<f32>,
    pub orientation: Option<f32>,
}

impl Arena {
    pub fn new(name: &str, map: BlockId, pos: BlockPosition, radius: f32) -> Arena {
        Arena { name: name.to_string(), map, pos, radius: Some(radius), orientation: None }
    }

    pub fn new_for_test(name: &str, map: BlockId, pos: BlockPosition, orientation: f32) -> Arena {
        Arena { name: name.to_string(), map, pos, radius: None, orientation: Some(orientation) }
    }
}

// Don't spawn these unless testing, to avoid spawns stuck in the queue
#[allow(unused)]
static MANUAL_ARENAS: LazyLock<Vec<Arena>> = LazyLock::new(||
     vec![
        Arena::new_for_test("First Step", BlockId::from_parts(60, 51, 57, 00), BlockPosition::from_xyz(25.754, 1620.250, 113.799), -44.685),
        Arena::new_for_test("Castle Sol", BlockId::from_parts(60, 42, 36, 00), BlockPosition::from_xyz(-11.155, 91.650, -76.040), -178.738),
    ]
);

pub fn mount_chr(rider: &ChrIns, mount: &ChrIns) {
    let program = Program::current();
    let mount_fn = unsafe {
        program.derva_ptr::<extern "C" fn(&ChrIns, &ChrIns, bool)>(
            crate::rva::MOUNT_RIDE,
        )
    };
    mount_fn(rider, mount, true);
}

pub fn generate_field_ins_handle(index: u32) -> FieldInsHandle {
    let field_ins_handle = FieldInsHandle {
        selector: FieldInsSelector::from_parts(eldenring::cs::FieldInsType::Chr, 113, index),
        block_id: BlockId::none(),
    };
    field_ins_handle
}

pub fn spawn_mob(
    index: Option<u32>,
    map: &BlockId,
    pos: &BlockPosition,
    orientation: f32,
    npc_param: i32,
    think_param: i32,
    chara_init_param: i32,
    model: &str,
) -> Result<Option<NonNull<ChrIns>>, Box<dyn Error>> {
    let Ok(world_chr_man) = (unsafe { WorldChrMan::instance() }) else {
        return Ok(None);
    };

    let chr_set = &world_chr_man.summon_buddy_chr_set;
    let chr_set_index = |index| CHR_SET_START + (index % (CHR_SET_CAPACITY - CHR_SET_START));

    let index = match index {
        // If given an index, always use it (0-indexed)
        Some(index) => index,
        None => {
            // If not given an index, find last one, for more compatibility with set indices
            let mut empty_index = CHR_SET_START;
            for index in (CHR_SET_START..CHR_SET_CAPACITY).rev() {
                let field_ins_handle = generate_field_ins_handle(chr_set_index(index));
                if chr_set.chr_ins_by_handle(&field_ins_handle).is_none() {
                    empty_index = index;
                    break;
                }
            }
            empty_index
        },
    };

    let entity_id: u32 = 460280000 + index;
    let field_ins_handle = generate_field_ins_handle(chr_set_index(index));

    let program = Program::current();
    if let Some(exist_chr_ins) = chr_set.chr_ins_by_handle(&field_ins_handle) {
        let remove_chr_ins =
            unsafe { program.derva_ptr::<extern "C" fn(&WorldChrMan, &ChrIns)>(REMOVE_CHR_INS) };

        // Don't do in PostPhysics or else crash
        remove_chr_ins(world_chr_man, exist_chr_ins);
    }

    let field_area = unsafe { *program.derva::<*mut FieldArea>(GLOBAL_FIELD_AREA) };
    let field_area = unsafe { field_area.as_ref().unwrap_unchecked() };
    // log::debug!("overworld: {} for {:p}", map.is_overworld(), field_area);

    let Some(center) = field_area
        .world_info_owner
        .world_res
        .world_info
        .world_block_info_by_map(map)
        .map(|b| b.physics_center)
    else {
        log::error!("Could not find WorldBlockInfo for map ID {map}");
        return Ok(None);
    };

    let spawn_physics_pos =
        HavokPosition::from_xyz(pos.x + center.0, pos.y + center.1, pos.z + center.2);

    // Farum Azula: patrol 9
    let request = Box::leak(Box::new(ChrSpawnRequest {
        position: spawn_physics_pos,
        orientation: F32Vector4(0.0, orientation, 0.0, 0.0),
        scale: F32Vector4(1.0, 1.0, 1.0, 1.0),
        unk30: F32Vector4(1.0, 1.0, 1.0, 1.0),
        npc_param: npc_param,
        npc_think_param: think_param,
        chara_init_param: chara_init_param,
        event_entity_id: entity_id,
        talk_id: 0,
        unk54: -1.828282595,

        unk58: 0x142A425A0,
        asset_name_str_ptr: 0, // Filled in after the fact
        unk68: 5,
        unk6c: 0,
        unk70: 0,
        unk74: 0x00010002,
        asset_name: Default::default(),
        unk98: 0x140BDE74D,
        unka0: 16,
        unka4: 0,
        // World pos?
        unka8: 0x141EBB015,
        unkb0: 0x143DCC270,
        unkb8: 0x1423F25F5,
        unkc0: 0,
        unkc4: 0,
    }));

    // Set string pointers as god intended
    let model_bytes = model.encode_utf16().collect::<Vec<u16>>();
    request.asset_name[0..5].clone_from_slice(model_bytes.as_slice());
    request.asset_name_str_ptr = request.asset_name.as_ptr() as usize;

    let spawn_chr = unsafe {
        program.derva_ptr::<extern "C" fn(
            &ChrSet<ChrIns>,
            &ChrSpawnRequest,
            FieldInsHandle,
        ) -> Option<NonNull<ChrIns>>>(SPAWN_CHR)
    };

    let setup_chrsync_1 = unsafe {
        program
            .derva_ptr::<extern "C" fn(&NetChrSync, &P2PEntityHandle) -> Option<NonNull<ChrIns>>>(
                NET_CHR_SYNC_SETUP_ENTITY_1,
            )
    };

    let setup_chrsync_2 = unsafe {
        program.derva_ptr::<extern "C" fn(
        &NetChrSync,
        &P2PEntityHandle,
        bool,
    ) -> Option<NonNull<ChrIns>>>(NET_CHR_SYNC_SETUP_ENTITY_2)
    };

    let setup_chrsync_3 = unsafe {
        program.derva_ptr::<extern "C" fn(
        &NetChrSync,
        &P2PEntityHandle,
        u32,
    ) -> Option<NonNull<ChrIns>>>(NET_CHR_SYNC_SETUP_ENTITY_3)
    };

    let mut chr_ins = spawn_chr(
        &world_chr_man.summon_buddy_chr_set,
        request,
        field_ins_handle.clone(),
    )
    .expect("Could not spawn chr");

    // set team type for all the enemies - or don't do this, do it in dll
    // unsafe { chr_ins.as_mut() }.team_type = 6;

    let p2phandle = &unsafe { chr_ins.as_ref() }.p2p_entity_handle;
    setup_chrsync_1(world_chr_man.net_chr_sync.as_ref(), p2phandle);
    unsafe { chr_ins.as_mut() }
        .net_chr_sync_flags
        .set_unk2(true);

    if let Ok(session_manager) = unsafe { CSSessionManager::instance() } {
        log::info!("Setting host flags: {:?}", session_manager.lobby_state);
        if session_manager.lobby_state == LobbyState::Host {
            setup_chrsync_2(world_chr_man.net_chr_sync.as_ref(), p2phandle, true);
            setup_chrsync_3(world_chr_man.net_chr_sync.as_ref(), p2phandle, 0xfff);
        }
    }

    // chr_flags_unk1(unsafe { chr_ins.as_ref() }, true);
    // chr_flags_unk2(unsafe { chr_ins.as_ref() });

    Ok(Some(chr_ins))
}

#[repr(C)]
pub struct ChrSpawnRequest {
    pub position: HavokPosition,
    pub orientation: F32Vector4,
    pub scale: F32Vector4,
    pub unk30: F32Vector4,
    pub npc_param: i32,        // 31000000
    pub npc_think_param: i32,  // 31000000
    pub chara_init_param: i32, // -1
    pub event_entity_id: u32,  // 0
    pub talk_id: u32,          // 0
    unk54: f32,                // -1.828282595

    // Cursed ass dlinplace str meme
    unk58: usize,              // 142A425A0
    asset_name_str_ptr: usize, // 13FFF0278
    unk68: u32,                // 5
    unk6c: u32,                // 0
    unk70: u32,                // 0
    unk74: u32,                // 0x00010002
    asset_name: [u16; 0x10],   // c3100
    unk98: usize,              // 140BDE74D
    unka0: u32,                // 16
    unka4: u32,                // 0
    unka8: u64,                // 0000000141EBB015
    unkb0: u64,                // 0000000143DCC270
    unkb8: u64,                // 00000001423F25F5
    unkc0: u32,                // 0
    unkc4: u32,                // 0
}

#[allow(unused)]
pub fn buddy_spawn() {
    // Knight at First Step
    let map = BlockId::from_parts(60, 42, 36, 00);
    let pos = BlockPosition::from_xyz(-11.155, 91.650, -76.040);
    spawn_mob(
        Some(20),
        &map,
        &pos,
        -178.738,
        43510010,
        43510000,
        -1,
        "c4351",
    )
    .unwrap();
}

#[allow(unused)]
pub fn debug_spawn() {
    static ENTITY_ID: Mutex<i32> = Mutex::new(460280000);

    let Ok(world_chr_man) = (unsafe { WorldChrMan::instance() }) else {
        return;
    };
    let Some(player) = world_chr_man.main_player.as_ref() else {
        return;
    };

    let id;
    {
        let mut current_id = ENTITY_ID.lock().unwrap();
        id = *current_id;
        *current_id += 1;
    }

    let pos = player.module_container.physics.position;
    world_chr_man.spawn_debug_character(&ChrDebugSpawnRequest {
        chr_id: 2275,
        chara_init_param_id: -1,
        npc_param_id: 22750020,
        npc_think_param_id: 22750000,
        event_entity_id: id,
        talk_id: -1,
        pos_x: pos.0 + 0.5,
        pos_y: pos.1,
        pos_z: pos.2 + 0.5,
    });
}
