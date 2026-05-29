use std::{collections::HashMap, sync::{Arc, OnceLock, RwLock}, time::{Duration, Instant}};
use chrono::{DateTime, FixedOffset, Local};
use eldenring::{
    cs::{CSEventFlagMan, CSTaskGroupIndex, CSTaskImp, GameDataMan, WorldChrMan},
    fd4::FD4TaskData,
    util::system::wait_for_system_init
};
use fromsoftware_shared::{FromStatic, SharedTaskImpExt};

use hudhook::imgui::{Condition, TreeNodeFlags, Ui, WindowFlags};
use serde::{Serialize, Deserialize};

use crate::{
    client::{Client, ClientModule, StreamRequest, parse_network_time}, program::current_module_path, ui::UiData,
};

// --- Network layer

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")] 
pub struct BossRecord {
    player_id: String,
    flag_id: u32,
    real_ms: Option<u32>,
    igt_ms: Option<u32>,
}

// Received by server
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct UpdateBossRequest {
    // Flag ids of newly dead bosses, sent by the server (client version is in BossStreamRequest).
    // It is assumed that clients can never revive bosses. A server-side reset should be performed if so.
    add: Option<Vec<BossRecord>>,
    // Flag ids of all known dead bosses, sent by the server.
    // This will be an empty list in the case of a reset.
    set: Option<Vec<BossRecord>>,
    // The run timestamp if there's a run active, or empty string if not
    run: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct BossesApiRequest {
    // For in-game interactions, filter for local id
    bosses: Option<UpdateBossRequest>,
}

// Sent to server
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum BossStreamRequest {
    #[serde(rename = "update_bosses")]
    UpdateBosses { bosses: Vec<BossRecord> },
}
impl StreamRequest for BossStreamRequest {}

// --- Current state
#[derive(Default, Clone, Debug)]
pub struct BossState {
    local: Option<BossRecord>,
    network: HashMap<String, BossRecord>,
}

impl BossState {
    fn new() -> Self { Default::default() }
    fn local_dead(&self) -> bool { self.local.is_some() }
    fn network_dead(&self) -> bool { !self.network.is_empty() }
    fn any_dead(&self) -> bool { self.local.is_some() || !self.network.is_empty() }
    fn pending_dead(&self) -> Option<&BossRecord> {
        // Local player_id should always match the immutable one in Client
        self.local.as_ref().filter(|b| !self.network.contains_key(&b.player_id))
    }
    fn clear(&mut self) {
        self.local.take();
        self.network.clear();
    }
}

#[derive(Default, Clone, Debug)]
struct GameBossState {
    bosses: HashMap<u32, BossState>,
    current_run: Option<DateTime<FixedOffset>>,
    pending_update: Option<Instant>,
    last_igt: Option<u32>,
}

impl GameBossState {
    fn from_data(data: &GameBossData) -> Self {
        Self {
            bosses: data.bosses().map(|b| (b.flag_id, BossState::new())).collect(),
            ..Default::default()
        }
    }

    fn summary(&self) -> String {
        let local = self.bosses.values().filter(|b| b.local_dead()).count();
        let network = self.bosses.values().filter(|b| b.network_dead()).count();
        let any = self.bosses.values().filter(|b| b.any_dead()).count();
        let total = self.bosses.len();
        format!("{} local, {} network, {}/{} total", local, network, any, total)
    }
}

// --- Config
#[derive(Clone, Default, Debug)]
struct GameBossData(Vec<RegionBossData>);

impl GameBossData {
    fn read() -> Result<GameBossData, Box<dyn std::error::Error>> {
        let mut path = current_module_path();
        path.set_file_name("bosses.json");
        let path = path.as_path();
        let file = std::fs::File::open(path).map_err(|e| format!("Failed to open {}: {e}", path.to_str().unwrap_or("")))?;
        let data: Vec<RegionBossData> = serde_json::from_reader(file)?;
        Ok(GameBossData(data))
    }

    fn bosses(&self) -> impl Iterator<Item = &BossData> {
        self.0.iter().flat_map(|r| &r.bosses)
    }

    fn regions(&self) -> &Vec<RegionBossData> { &self.0 }
}

// Structure of EROverlay data, as of May 2026, for easy interop
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct RegionBossData {
    region_name: String,
    regions: Vec<u32>,
    bosses: Vec<BossData>,
    #[serde(default)]
    dlc: u8,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct BossData {
    boss: String,
    place: String,
    flag_id: u32,
}

#[derive(Clone, Default, Debug)]
struct UiState {
    is_open: bool,
}

pub struct BossClient {
    data: GameBossData,
    state: RwLock<GameBossState>,
    // Only read/write in ImGui thread
    ui_state: RwLock<UiState>,
}

static INSTANCE: OnceLock<Arc<BossClient>> = OnceLock::new();

impl BossClient {
    #[allow(unused)]
    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    pub fn initialize() {
        if INSTANCE.get().is_some() {
            panic!("Already initialized");
        }
        // Can panic, TODO expect error message
        let data = GameBossData::read().unwrap();
        let state = GameBossState::from_data(&data);
        let client = Arc::new(BossClient {
            data: data,
            state: RwLock::new(state),
            ui_state: RwLock::new(UiState::default()),
        });
        let other = client.clone();
        std::thread::spawn(move || other.run_task());
        Client::get().register_module(client.clone());
        INSTANCE.set(client).ok().expect("Already initialized");
    }

    fn handle_request(&self, req: &UpdateBossRequest) {
        // Change network status. This will be reconciled with local data in the task.
        let mut state = self.state.write().unwrap();
        if let Some(run) = &req.run {
            state.current_run = parse_network_time(run);
        }
        let records: &Vec<BossRecord>;
        if let Some(set) = &req.set {
            // Clear out all network_dead at least. local_dead will be set anyway before the next
            // message to the server, but will be cached if the task is not running, so prioritize
            // the reset case where all rendered counters will go to 0.
            state.bosses.values_mut().for_each(|boss| boss.clear());
            // For a reset or server restart, allow for immediate response from the task.
            state.pending_update.take();
            records = set;
        } else if let Some(add) = &req.add {
            records = add;
        } else {
            return;
        }
        for record in records {
            if let Some(boss) = state.bosses.get_mut(&record.flag_id) {
                boss.network.insert(record.player_id.clone(), record.clone());
            } else {
                log::warn!("Unknown boss record {record:?}");
            }
        }
        // If everything is resolved, allow future updates to prompt an immediate response from the task.
        if state.pending_update.is_some() && !state.bosses.values().any(|boss| boss.pending_dead().is_some()) {
            state.pending_update.take();
        }
        log::info!("Received boss state ({}): {req:?}", state.summary());
    }

    fn run_task(self: Arc<Self>) {
        wait_for_system_init(&fromsoftware_shared::Program::current(), Duration::MAX)
            .expect("Could not await system init.");
        // Needed without modloader
        std::thread::sleep(Duration::from_secs(3));

        let cs_task = unsafe { CSTaskImp::instance().expect("Task system not initialized") };
        cs_task.run_recurring(move
            |_: &FD4TaskData| {
                // Ideally use CSMenuMan here (apparently it's in LoadingScreenData?), but maybe this should work
                let Ok(_) = (unsafe { WorldChrMan::instance() }) else {
                    // Also update_ui here, if there is any indirection
                    return
                };
                let Ok(event_flag_man) = (unsafe { CSEventFlagMan::instance() }) else {
                    return
                };
                let mut pending: Vec<BossRecord> = Vec::new();
                let mut state = self.state.write().unwrap();
                let can_update = state.pending_update.is_none_or(|time| time.elapsed() > Duration::from_secs(10));
                let run_time = state.current_run.as_ref()
                    .map(|time| (Local::now().fixed_offset() - time).as_seconds_f64());
                let mut igt_ms = None;
                if let Ok(game_data_man) = unsafe { GameDataMan::instance() } {
                    let play_time = game_data_man.play_time;
                    // Require boss kill in recent observed memory (<1s IGT from last pass) to count for recording.
                    if let Some(last_igt) = state.last_igt && last_igt.abs_diff(play_time) < 1000 {
                        igt_ms = Some(play_time);
                    }
                    state.last_igt = Some(play_time);
                }
                let client = Client::get();
                for boss in self.data.bosses() {
                    let flag_id = boss.flag_id;
                    let dead = event_flag_man.virtual_memory_flag.get_flag(flag_id);
                    if let Some(boss_state) = state.bosses.get_mut(&flag_id) {
                        // This may be a state change in either direction
                        if dead && boss_state.local.is_none() {
                            // Only report real_ms during run and if igt_ms condition is met.
                            let real_ms = run_time.and_then(
                                |elapsed| if igt_ms.is_some() && elapsed > 0.0 { Some((elapsed * 1000.0) as u32) } else { None });
                            let record = BossRecord {
                                player_id: client.unique_id.clone(),
                                flag_id,
                                real_ms,
                                igt_ms,
                            };
                            boss_state.local = Some(record);
                        } else if !dead {
                            boss_state.local = None;
                        }
                        // Reconcile with server only for marking dead
                        if can_update && let Some(record) = boss_state.pending_dead() {
                            pending.push(record.clone());
                        }
                    }
                }
                // Report to the server only if there wasn't a recent report
                if !pending.is_empty() {
                    let req = BossStreamRequest::UpdateBosses { bosses: pending };
                    log::info!("Sending boss state ({}) -> {req:?}", state.summary());
                    client.stream_send(req);
                    state.pending_update = Some(Instant::now());
                }
                // self.update_ui(&state.bosses);
            },
            // Idk
            CSTaskGroupIndex::HavokWorldUpdate_Pre,
        );
    }
}

impl ClientModule for BossClient {
    fn handle_message(&self, json: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
        let api_req = BossesApiRequest::deserialize(json)?;
        if let Some(req) = &api_req.bosses {
            self.handle_request(req);
        }
        Ok(())
    }

    fn render(&self, ui: &Ui, ui_data: &UiData) {
        // EROverlay style. (also no saved settings?)
        let corner_flags =
            WindowFlags::NO_TITLE_BAR | WindowFlags::NO_RESIZE | WindowFlags::NO_MOVE | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_SCROLLBAR | WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_NAV;
        let sidebar_flags =
            WindowFlags::NO_TITLE_BAR | WindowFlags::NO_RESIZE | WindowFlags::NO_MOVE | WindowFlags::NO_COLLAPSE;

        // TODO: Maybe don't init when empty
        let corner_text;
        {
            let bosses = &self.state.read().unwrap().bosses;
            let dead_count = bosses.values().filter(|b| b.any_dead()).count();
            corner_text = format!("{}/{}", dead_count, bosses.len());
        }

        let mut ui_state = self.ui_state.write().unwrap();

        if ui.is_key_pressed_no_repeat(hudhook::imgui::Key::Equal) {
            ui_state.is_open = !ui_state.is_open;
        }

        let io = ui.io();
        let [dw, dh] = io.display_size;

        let text_size = ui.calc_text_size(&corner_text);
        let padding = ui.clone_style().window_padding;
        if ui_state.is_open {
            // Was 0.2, 0.8
            let size = [dw * 0.15, dh * 0.84];
            let pos = [dw * 0.99, dh * 0.01];
            ui.window("##corner")
                .flags(sidebar_flags)
                .position_pivot([1.0, 0.0])
                .position(pos, Condition::Always)
                .size(size, Condition::Always)
                .build(|| {
                    if !corner_text.is_empty() {
                        let bound_width = ui.content_region_avail()[0];
                        ui.set_cursor_pos([bound_width + padding[0] - text_size[0], ui.cursor_pos()[1]]);
                        if ui.selectable(&corner_text) {
                            ui_state.is_open = false;
                        }
                        ui.child_window("##regions")
                            .size(ui.content_region_avail())
                            .build(|| {
                                let small = ui.push_font(ui_data.fonts.small);
                                let state = &self.state.read().unwrap();
                                for region in self.data.regions() {
                                    let region_count = region.bosses.iter()
                                        .flat_map(|b| state.bosses.get(&b.flag_id))
                                        .filter(|b| b.any_dead())
                                        .count();
                                    let title = format!(
                                        "{}/{} {}###{}",
                                        region_count,
                                        region.bosses.len(),
                                        region.region_name,
                                        region.region_name);
                                    ui.tree_node_config(&title).flags(TreeNodeFlags::SPAN_AVAIL_WIDTH).build(|| {
                                        for data in &region.bosses {
                                            let boss = state.bosses.get(&data.flag_id);
                                            let local = boss.map_or(false, |b| b.local_dead());
                                            let network = boss.map_or(false, |b| b.network_dead());
                                            let mut any = local || network;
                                            // Can disable to prevent clicking, but whatever
                                            let disabled = ui.begin_disabled(true);
                                            ui.checkbox(format!("##{}", data.flag_id), &mut any);
                                            let check_hover = ui.is_item_hovered();
                                            disabled.end();
                                            ui.same_line();
                                            let wrap = ui.push_text_wrap_pos();
                                            ui.text_wrapped(&data.boss);
                                            wrap.end();
                                            if check_hover || ui.is_item_hovered() {
                                                ui.tooltip_text(format!("{}: {}", data.boss, data.place));
                                            }
                                        }
                                        // tree.pop();
                                    });
                                }
                                small.end();
                            });
                    }
                });
        } else {
            // Was 0.2, 0.8
            let size = [text_size[0] + padding[0] * 2.0, text_size[1] + padding[1] * 2.0];
            let pos = [dw * 0.99, dh * 0.01];
            ui.window("##corner")
                .flags(corner_flags)
                .position_pivot([1.0, 0.0])
                .position(pos, Condition::Always)
                .size(size, Condition::Always)
                .build(|| {
                    if !corner_text.is_empty() {
                        if ui.selectable(&corner_text) {
                            ui_state.is_open = true;
                        }
                    }
                });
        }
    }
}
