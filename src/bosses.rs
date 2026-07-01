use std::{collections::HashMap, ops::Deref, sync::{Arc, OnceLock, RwLock}, time::{Duration, Instant}};
use chrono::{DateTime, FixedOffset, Local};
use eldenring::{
    cs::{BlockId, CSEventFlagMan, CSTaskGroupIndex, CSTaskImp, GameDataMan, WorldChrMan},
    fd4::FD4TaskData,
    util::system::wait_for_system_init
};
use fromsoftware_shared::{FromStatic, SharedTaskImpExt};

use hex_color::HexColor;
use hudhook::imgui::{Condition, StyleColor, StyleVar, TreeNodeFlags, Ui, WindowFlags};
use serde::{Serialize, Deserialize};

use crate::{
    client::{Client, ClientModule, StreamRequest, parse_network_time}, program::current_module_path, ui::UiData
};

// --- Network layer

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")] 
pub struct BossRecord {
    player_id: String,
    flag_id: u32,
    defeated_at: Option<String>,
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
#[serde(rename_all = "camelCase")] 
pub struct Runner {
    id: String,
    name: Option<String>,
    character_name: Option<String>,
    color: Option<String>,
    #[serde(skip)]
    hex_color: Option<HexColor>,
}

impl Runner {
    fn name(&self) -> &str {
        self.name.as_deref().or(self.character_name.as_deref()).unwrap_or("Unknown")
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct BossesApiRequest {
    // For in-game interactions, filter for local id
    bosses: Option<UpdateBossRequest>,
    runners: Option<HashMap<String, Runner>>,
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
        // Returns local player which is not in networked players
        // Local player_id should always match the immutable one in Client
        self.local.as_ref().filter(|b| !self.network.contains_key(&b.player_id))
    }
    fn unique_records(&self) -> Vec<&BossRecord> {
        let mut records = Vec::new();
        if let Some(local) = &self.local && !self.network.contains_key(&local.player_id) {
            records.push(local);
        }
        records.extend(self.network.values());
        records.sort_by_key(|r| r.igt_ms);
        records
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
    runners: HashMap<String, Runner>,
    pending_update: Option<Instant>,
    // Last seen player location
    play_region: u32,
    block_id: Option<BlockId>,
    // Last observed IGT. Only set when WorldChrMan exists since it's normally 0 on the main menu.
    last_igt: Option<u32>,
    // Whether any messages have been observed.
    received_messages: bool,
    // Condition to show UI: Run ongoing, or in-game, or received bosses and no local bosses (empty state)
    show_ui: bool,
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
struct GameBossData {
    regions: Vec<RegionBossData>,
    block_id_indices: HashMap<BlockId, usize>,
    play_region_indices: HashMap<u32, usize>,
}

fn map_to_block_id(map: u32) -> BlockId {
    BlockId::from_parts((map / 100_00_00) as u8, ((map / 100_00) % 100) as u8, ((map / 100) % 100) as u8, (map % 100) as u8)
}

impl GameBossData {
    fn new(regions: Vec<RegionBossData>) -> Self {
        let block_id_indices = regions.iter().enumerate()
            .flat_map(|(i, region)| region.bosses.iter()
                .flat_map(|b| b.maps.iter().flatten())
                .map(move |&map| (map_to_block_id(map), i)))
            .collect();
        let play_region_indices = regions.iter().enumerate()
            .flat_map(|(i, region)| region.bosses.iter()
                .flat_map(|b| b.regions.iter().flatten())
                .chain(region.regions.iter().flatten())
                .map(move |&r| (r, i)))
            .collect();
        GameBossData { regions, block_id_indices, play_region_indices }
    }

    #[allow(unused)]
    fn read_file() -> Result<GameBossData, Box<dyn std::error::Error>> {
        let mut path = current_module_path();
        path.set_file_name("bosses.json");
        let path = path.as_path();
        let file = std::fs::File::open(path).map_err(|e| format!("Failed to open {}: {e}", path.to_str().unwrap_or("")))?;
        let data: Vec<RegionBossData> = serde_json::from_reader(file)?;
        Ok(GameBossData::new(data))
    }

    fn read_embedded() -> Result<GameBossData, Box<dyn std::error::Error>> {
        let data: Vec<RegionBossData> = serde_json::from_str(include_str!("bosses/bosses.json"))?;
        Ok(GameBossData::new(data))
    }

    fn bosses(&self) -> impl Iterator<Item = &BossData> {
        self.regions.iter().flat_map(|r| &r.bosses)
    }
}

// Structure of EROverlay data, as of May 2026, for easy interop
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct RegionBossData {
    region_name: String,
    regions: Option<Vec<u32>>,
    bosses: Vec<BossData>,
    #[serde(default)]
    dlc: u8,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct BossData {
    boss: String,
    place: String,
    flag_id: u32,
    regions: Option<Vec<u32>>,
    maps: Option<Vec<u32>>,
}

#[derive(Clone, Default, Debug)]
struct UiState {
    // Whether to open on next frame
    toggle_open: bool,
    // Current state, for handling changes
    is_open: bool,
    is_hidden: bool,
    is_big: bool,
    hide_completed: bool,
    // The index of the current region
    region_index: Option<usize>,
    // Whether to set y position on the next frame
    next_visible: Option<usize>,
}

impl UiState {
    fn new() -> Self {
        Self { is_big: true, ..Default::default() }
    }
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
        // Can panic, TODO should expect error message
        let data = GameBossData::read_embedded().unwrap();
        let state = GameBossState::from_data(&data);
        let client = Arc::new(BossClient {
            data: data,
            state: RwLock::new(state),
            ui_state: RwLock::new(UiState::new()),
        });
        let other = client.clone();
        std::thread::spawn(move || other.run_task());
        Client::get().register_module(client.clone());
        INSTANCE.set(client).ok().expect("Already initialized");
    }

    fn handle_request(&self, req: &UpdateBossRequest) {
        // Change network status. This will be reconciled with local data in the task.
        let mut state = self.state.write().unwrap();
        state.received_messages = true;
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
        log::info!("Received boss state ({})", state.summary());
    }

    fn handle_runners(&self, req: &HashMap<String, Runner>) {
        let mut state = self.state.write().unwrap();
        let mut runners = req.clone();
        runners.values_mut().for_each(|r| {
            r.hex_color = r.color.as_ref().and_then(|c| HexColor::parse_rgb(c).ok());
        });
        state.runners = runners;
    }

    fn run_task(self: Arc<Self>) {
        wait_for_system_init(&fromsoftware_shared::Program::current(), Duration::MAX)
            .expect("Could not await system init.");
        // Needed without modloader
        std::thread::sleep(Duration::from_secs(3));

        let cs_task = unsafe { CSTaskImp::instance().expect("Task system not initialized") };
        cs_task.run_recurring(move
            |_: &FD4TaskData| {
                let Ok(event_flag_man) = (unsafe { CSEventFlagMan::instance() }) else {
                    return
                };
                let mut state = self.state.write().unwrap();
                // CSMenuMan can be used to some extent, but it does start out with in-game-like state
                let is_in_game;
                if let Ok(world_chr_man) = unsafe { WorldChrMan::instance() } {
                    if let Some(player) = &world_chr_man.main_player {
                        let play_region = player.play_region_id;
                        let block_id = player.block_origin_override;
                        // Require both to exist as they load in at different times
                        if block_id.0 != -1 && play_region > 0 {
                            state.block_id = Some(block_id);
                            state.play_region = play_region / 1000;
                        }
                    }
                    is_in_game = true;
                } else {
                    is_in_game = false;
                }
                let can_update = is_in_game
                    && state.current_run.is_some()
                    && state.pending_update.is_none_or(|time| time.elapsed() > Duration::from_secs(10));
                // Eligibility for showing UI, to avoid only showing local or network state
                // Ideally this could trigger on create character
                // For now, just always allow it
                if !state.show_ui {
                    state.show_ui = state.current_run.is_some()
                        || state.last_igt.is_some()
                        || (state.received_messages && state.bosses.values().all(|b| !b.any_dead()))
                        || true;
                }
                // If not in game, flags may be unreliable, so don't set local or network state based on it
                if !is_in_game {
                    return;
                }
                let mut igt_ms = None;
                if is_in_game && let Ok(game_data_man) = unsafe { GameDataMan::instance() } {
                    let play_time = game_data_man.play_time;
                    // Require boss kill in recent observed memory (<1s IGT from last pass) to count for recording.
                    if let Some(last_igt) = state.last_igt && last_igt.abs_diff(play_time) < 1000 {
                        igt_ms = Some(play_time);
                    }
                    state.last_igt = Some(play_time);
                }
                let now = Local::now().fixed_offset();
                let mut pending: Vec<BossRecord> = Vec::new();
                let client = Client::get();
                for boss in self.data.bosses() {
                    // TODO: This can probably iterate state.bosses as data isn't used for anything
                    let flag_id = boss.flag_id;
                    let dead = event_flag_man.virtual_memory_flag.get_flag(flag_id);
                    if let Some(boss_state) = state.bosses.get_mut(&flag_id) {
                        // This may be a state change in either direction
                        if dead && boss_state.local.is_none() {
                            // real_ms could also be calculated as (now - current_run).as_seconds_f64() * 1000, but this
                            // can be unreliable. Just use real clock time for the case where a run is restarted partway through.
                            let defeated_at = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Millis, false));
                            let record = BossRecord {
                                player_id: client.unique_id.clone(),
                                flag_id,
                                defeated_at,
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

fn color_vec(c: HexColor) -> [f32; 4] {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, c.a as f32 / 255.0]
}

impl ClientModule for BossClient {
    fn handle_message(&self, json: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
        let api_req = BossesApiRequest::deserialize(json)?;
        if let Some(req) = &api_req.bosses {
            self.handle_request(req);
        }
        if let Some(req) = &api_req.runners {
            self.handle_runners(req);
        }
        Ok(())
    }

    fn render(&self, ui: &Ui, ui_data: &UiData) {
        // Match EROverlay
        let corner_flags =
            WindowFlags::NO_TITLE_BAR | WindowFlags::NO_RESIZE | WindowFlags::NO_MOVE | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_SCROLLBAR | WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_NAV;
        let sidebar_flags =
            WindowFlags::NO_TITLE_BAR | WindowFlags::NO_RESIZE | WindowFlags::NO_MOVE | WindowFlags::NO_COLLAPSE;

        // TODO: Could have mode for showing only local state
        let corner_text;
        let own_color;
        let play_region;
        let block_id;
        {
            let state = self.state.read().unwrap();
            if !state.show_ui {
                return;
            }
            let dead_count = state.bosses.values().filter(|b| b.any_dead()).count();
            corner_text = format!("{}/{}", dead_count, state.bosses.len());
            own_color = state.runners.get(&Client::get().unique_id).and_then(|r| r.hex_color).unwrap_or(HexColor::WHITE);
            play_region = state.play_region;
            block_id = state.block_id;
        }

        let mut ui_state = self.ui_state.write().unwrap();

        // Equal and Backslash both toggle, but between opposite states
        let toggle_open = ui.is_key_pressed_no_repeat(hudhook::imgui::Key::Equal) || ui_state.toggle_open;
        if toggle_open {
            if ui_state.is_hidden {
                ui_state.is_hidden = false;
                ui_state.is_open = false;
            } else {
                ui_state.is_open = !ui_state.is_open;
            }
            ui_state.toggle_open = false;
        }
        if ui.is_key_pressed_no_repeat(hudhook::imgui::Key::Backslash) {
            ui_state.is_open = false;
            ui_state.is_hidden = !ui_state.is_hidden;
        }
        let hide_completed = ui.is_key_down(hudhook::imgui::Key::Minus);

        let current_index = block_id.and_then(|block| self.data.block_id_indices.get(&block))
            .or_else(|| self.data.play_region_indices.get(&play_region))
            .copied();
        let index_changed = current_index.is_some() &&
            (ui_state.region_index != current_index || toggle_open || (ui_state.hide_completed && !hide_completed));
        let start_hide_completed = !ui_state.hide_completed && hide_completed;
        let next_visible = ui_state.next_visible.take();
        let debug_region = false;

        let io = ui.io();
        let [dw, dh] = io.display_size;

        let mut color_tokens = vec![];
        let mut set_colors = |color: HexColor, styles: &[StyleColor]| {
            let mut color = color_vec(color);
            for &style in styles {
                let exist_color = ui.style_color(style);
                // Copy alpha
                color[3] = exist_color[3];
                color_tokens.push(ui.push_style_color(style, color));
            }
        };
        // Header is selectables, Frame is checkbox background
        // Colors are here for some reason: https://github.com/TheGreatRambler/toost/blob/main/src/imgui/imgui_draw.cpp
        // Default light blue #4296fa. Hue 213 -> 270 is rgb(158, 66, 250)
        set_colors(HexColor::rgb(211, 185, 136), &vec![
            StyleColor::CheckMark,  StyleColor::FrameBgHovered, StyleColor::FrameBgActive,
        ]);
        set_colors(HexColor::rgb(100, 100, 100), &vec![
            StyleColor::Header, StyleColor::HeaderHovered, StyleColor::HeaderActive,
        ]);
        // Darker blue #294a7a, main checkbox color. 216 -> 270 is rgb(82, 42, 122)
        set_colors(HexColor::rgb(138, 109, 74), &vec![StyleColor::FrameBg]);
        // Simple #6e6e80 with transparency
        // set_colors(HexColor::rgb(82, 42, 122), &vec![StyleColor::Border]);
        // Default #0f0f0f at 94% opacity
        // set_colors(HexColor::rgb(82, 42, 122), &vec![StyleColor::WindowBg]);

        let check_styles = vec![StyleVar::ItemInnerSpacing([0.0; 2]), StyleVar::FramePadding([0.0; 2])];

        let big = ui.push_font(if ui_state.is_big { ui_data.fonts.very_big } else { ui_data.fonts.big });
        let text_size = ui.calc_text_size(&corner_text);
        let padding = ui.clone_style().window_padding;
        let top_size = [text_size[0] + padding[0] * 2.0, text_size[1] + padding[1] * 2.0];
        if ui_state.is_open && !ui_state.is_hidden {
            // Was 0.2, 0.8 (then 0.84)
            let size = [dw * 0.15, dh * 0.774 + top_size[1]];
            let pos = [dw * 0.99, dh * 0.01];
            ui.window("##corner")
                .flags(sidebar_flags)
                .bg_alpha(0.7)
                .position_pivot([1.0, 0.0])
                .position(pos, Condition::Always)
                .size(size, Condition::Always)
                .build(|| {
                    let bound_width = ui.content_region_avail()[0];
                    ui.set_cursor_pos([bound_width + padding[0] - text_size[0], ui.cursor_pos()[1]]);
                    let color = ui.push_style_color(StyleColor::Text, color_vec(own_color));
                    if ui.selectable(&corner_text) {
                        ui_state.toggle_open = true;
                    }
                    color.end();
                    if debug_region {
                        let current_region = current_index.and_then(|i| self.data.regions.get(i));
                        ui.text(format!("{} {}", play_region, current_region.map(|r| r.region_name.deref()).unwrap_or("")));
                    }
                    ui.child_window("##regions").size(ui.content_region_avail()).build(|| {
                        if start_hide_completed {
                            ui.set_scroll_here_y_with_ratio(0.0);
                        }
                        let small = ui.push_font(ui_data.fonts.small);
                        let state = &self.state.read().unwrap();
                        for (region_index, region) in self.data.regions.iter().enumerate() {
                            let region_count = region.bosses.iter()
                                .flat_map(|b| state.bosses.get(&b.flag_id))
                                .filter(|b| b.any_dead())
                                .count();
                            if hide_completed && region_count == region.bosses.len() {
                                continue;
                            }
                            let title = if hide_completed {
                                format!("{} {}###{}", region.bosses.len() - region_count, region.region_name, region.region_name)
                            } else {
                                format!("{}/{} {}###{}", region_count, region.bosses.len(), region.region_name, region.region_name)
                            };
                            let open_index = current_index == Some(region_index);
                            if next_visible == Some(region_index) {
                                ui.set_scroll_here_y_with_ratio(0.0);
                            }
                            let top_visible = ui.is_item_visible();
                            ui.tree_node_config(&title)
                                .flags(TreeNodeFlags::SPAN_AVAIL_WIDTH)
                                .opened(
                                    open_index || hide_completed,
                                    if index_changed || start_hide_completed { Condition::Always } else { Condition::Never })
                                .build(|| {
                                for data in &region.bosses {
                                    let boss = state.bosses.get(&data.flag_id);
                                    let local = boss.map_or(false, |b| b.local_dead());
                                    let network = boss.map_or(false, |b| b.network_dead());
                                    let mut any = local || network;
                                    if hide_completed && any {
                                        // Don't render a node if completed
                                        continue;
                                    }
                                    // These entries are unique by runner. We can ignore defeat time here as that's baked into order.
                                    let runners: Vec<&Runner> = boss.into_iter()
                                        .flat_map(|b| b.unique_records())
                                        .flat_map(|r| state.runners.get(&r.player_id))
                                        .collect();
                                    // Disable to prevent clicking I suppose
                                    // let disabled = ui.begin_disabled(true);
                                    let style_tokens: Vec<_> = check_styles.iter().map(|&v| ui.push_style_var(v)).collect();
                                    ui.checkbox(format!("##{}", data.flag_id), &mut any);
                                    let check_hover = ui.is_item_hovered();
                                    style_tokens.into_iter().rev().for_each(|v| v.pop());
                                    // disabled.end();
                                    // Formatted text
                                    let wrap = ui.push_text_wrap_pos();
                                    let bold = local.then(|| ui.push_font(ui_data.fonts.small_bold));
                                    if true || runners.len() <= 1 {
                                        let color = runners.iter().flat_map(|r| r.hex_color)
                                            .map(|c| ui.push_style_color(StyleColor::Text, color_vec(c))).nth(0);
                                        ui.same_line();
                                        ui.text_wrapped(&data.boss);
                                        color.map(|c| c.end());
                                    } else {
                                        // This should ideally use a cache to avoid constant DP
                                        // Unfortunately, SameLine and TextWrapped are incompatible (https://github.com/ocornut/imgui/issues/2313)
                                        // and I'd have to word wrap myself.
                                        let parts = split_evenly(&data.boss, 3);
                                        for (i, part) in parts.iter().enumerate() {
                                            let test_color = vec![HexColor::RED, HexColor::YELLOW, HexColor::BLUE];
                                            // let color = runners.get(i).and_then(|r| r.hex_color)
                                            let color = test_color.get(i).copied()
                                                .map(|c| ui.push_style_color(StyleColor::Text, color_vec(c)));
                                            ui.same_line_with_spacing(0.0, 0.0);
                                            ui.text_wrapped(part);
                                            color.map(|c| c.end());
                                        }
                                    }
                                    bold.map(|t| t.end());
                                    wrap.end();
                                    if check_hover || ui.is_item_hovered() {
                                        let place = if data.place.is_empty() { &region.region_name } else { &data.place };
                                        let names: Vec<&str> = runners.iter().map(|r| r.name()).collect();
                                        let defeat = if names.is_empty() { "".to_owned() } else { format!("\nDefeated by {}", names.join(", ")) };
                                        ui.tooltip_text(format!("In {}{}", place, defeat));
                                    }
                                }
                            });
                            let bottom_visible = ui.is_item_visible();
                            if index_changed && open_index && (!top_visible || !bottom_visible) && !hide_completed {
                                ui_state.next_visible = Some(region_index);
                            }
                        }
                        small.end();
                    });
                });
        } else if !ui_state.is_hidden {
            // Was 0.2, 0.8
            let transparent = true;
            let pos = [dw * 0.99, dh * 0.01];
            ui.window("##corner")
                .flags(corner_flags)
                .bg_alpha(if transparent { 0.0 } else { 0.1 })
                .position_pivot([1.0, 0.0])
                .position(pos, Condition::Always)
                .size(top_size, Condition::Always)
                .build(|| {
                    let color = ui.push_style_color(StyleColor::Text, color_vec(own_color));
                    if ui.selectable(&corner_text) {
                        ui_state.toggle_open = true;
                    }
                    color.end();
                });
        }
        big.end();

        color_tokens.into_iter().rev().for_each(|v| v.pop());

        if current_index.is_some() {
            ui_state.region_index = current_index;
        }
        ui_state.hide_completed = hide_completed;
    }
}

// Split words across n lines based on character count.
// Was this worth doing? Questionable
fn justify_words(words: Vec<&str>, lines: usize) -> Vec<String> {
    let lens: Vec<usize> = words.iter().map(|w| w.len()).collect();
    let target = lens.iter().sum::<usize>() / lines;
    // Put i words in j lines, with k being placed in previous lines.
    // DP(i, j) = min_{k in 0..i} DP(k, j-1) + line length cost
    let word_count = lens.len();
    let mut dp: Vec<usize> = vec![usize::MAX; (word_count + 1) * (lines + 1)];
    let mut parent: Vec<usize> = vec![usize::MAX; (word_count + 1) * (lines + 1)];
    dp[0] = 0;
    // Can't put any words in 0 lines
    for j in 1..lines + 1 {
        // Try to put all words in this line
        for i in 1..word_count + 1 {
            let index = i + j * (word_count + 1);
            // Lower bound: Previous j-1 lines need at least j-1 words (line 1 starts at word 0)
            // Upper bound: Need to place ith word on this line, so up to i-1 words in previous lines
            // For j=2 line, fitting i=3 words. k=1 word on line 1, measure word 1 and 2. k=2, measure word 2.
            for k in j - 1..i {
                let subindex = k + (j - 1) * (word_count + 1);
                let prev_cost = dp[subindex];
                if prev_cost == usize::MAX {
                    continue;
                }
                // Assume space at end of everything except last word
                let this_len = lens[k..i].iter().sum::<usize>() - (i < word_count - 1) as usize;
                let cost = this_len.abs_diff(target).pow(2);
                let total_cost = prev_cost + cost;
                if total_cost < dp[index] {
                    dp[index] = total_cost;
                    parent[index] = k;
                }
            }
        }
    }
    // This can be done in a way that avoids allocating new strings (to make a Vec<&str>),
    // but we'll want to cache them anyway so just copy everything out here.
    let mut result = Vec::with_capacity(lines);
    let mut end_index = word_count;
    for j in (1..lines + 1).rev() {
        let start_index = parent[end_index + j * (word_count + 1)];
        result.push(words[start_index..end_index].join(""));
        end_index = start_index;
    }
    result.reverse();
    result
}

// Returns the word split into even pieces. Whitespace at the end will be preserved.
fn split_word(word: &str, count: usize) -> Vec<String> {
    if count <= 1 {
        return vec![word.to_owned()];
    }
    let just_word = word.trim_end_matches(' ');
    let count = count.min(just_word.len());
    let indices: Vec<usize> = (0..count).map(|i| (just_word.len() * i + count / 2) / count).chain([word.len()]).collect();
    indices.windows(2).map(|range| word[range[0]..range[1]].to_owned()).collect()
}

fn split_evenly(text: &str, count: usize) -> Vec<String> {
    if count <= 1 {
        // Probably just don't call in this case
        return vec![text.to_owned()];
    }
    // This won't work great in Japanese etc
    let words: Vec<&str> = text.split_inclusive(' ').collect();
    if words.len() == count {
        words.into_iter().map(|w| w.to_owned()).collect()
    } else if count > words.len() {
        // Split up the longest words greedily. List of (length, splits)
        let mut splits: Vec<(usize, usize)> = words.iter().map(|w| (w.trim_end_matches(' ').len(), 1)).collect();
        for _ in 0..count - words.len() {
            if let Some(max) = splits.iter_mut().max_by_key(|(len, amt)| 1000 * len / amt) {
                max.1 += 1;
            }
        }
        words.into_iter().zip(splits).flat_map(|(word, (_, amt))| split_word(word, amt)).collect()
    } else {
        justify_words(words, count)
    }
}
