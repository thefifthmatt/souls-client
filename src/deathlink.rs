use std::{error::Error, sync::{Arc, OnceLock, RwLock}, time::{Duration, Instant}};
use eldenring::{
    cs::{CSEventFlagMan, CSMenuManImp, CSTaskGroupIndex, CSTaskImp, WorldChrMan},
    fd4::FD4TaskData,
    util::system::wait_for_system_init
};
use fromsoftware_shared::{FromStatic, SharedTaskImpExt};

use serde::{Serialize, Deserialize};

use crate::{
    client::{Client, ClientModule, CommonRequest, StreamRequest}, event::emedf, program::current_module_path, ui::WidgetChannel
};

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct DeathlinkRequest {
    id: String,
    name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct DeathlinkApiRequest {
    #[serde(flatten)]
    common: CommonRequest,
    deathlink: Option<DeathlinkRequest>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")] 
struct DeathlinkSettings {
    #[serde(default)]
    enable_deathlink: bool,
    #[serde(default)]
    rune_loss: bool,
    #[serde(default)]
    ignore_scripted: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct DeathlinkApiSettings {
    deathlink: Option<DeathlinkSettings>,
}

// Sent to server
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum DeathlinkStreamRequest {
    #[serde(rename = "deathlink")]
    Deathlink {},
}
impl StreamRequest for DeathlinkStreamRequest {}

#[derive(Clone, Debug, Default)]
struct DeathlinkState {
    // This could be based on the time the request was sent or received, but currently based on moment of death
    previous: Option<Instant>,
    // Unprocessed deathlink from other user
    pending: Option<DeathlinkRequest>,
    // Use same struct as network for now
    settings: DeathlinkSettings,
    // Just for debugging
    debug_inv: Option<String>,
}

impl DeathlinkState {
    fn new() -> Self {
        Self { ..Default::default() }
    }
}

pub struct DeathlinkClient {
    state: RwLock<DeathlinkState>,
}

// Alternate static enable mechanism
#[allow(unused)]
fn is_enabled_by_file() -> Result<bool, Box<dyn Error>> {
    let mut path = current_module_path();
    path.set_file_name("souls_deathlink_config.txt");
    let path = path.as_path();
    let text = std::fs::read_to_string(path).map_err(|e| format!("Failed to open {}: {e}", path.to_str().unwrap_or("")))?;
    Ok(text.trim() == "enabled")
}

static INSTANCE: OnceLock<Arc<DeathlinkClient>> = OnceLock::new();

impl DeathlinkClient {
    #[allow(unused)]
    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    pub fn initialize() {
        if INSTANCE.get().is_some() {
            panic!("Already initialized");
        }
        let client = Arc::new(DeathlinkClient {
            state: RwLock::new(DeathlinkState::new()),
        });
        let other = client.clone();
        std::thread::spawn(move || other.run_task());
        Client::get().register_module(client.clone());
        INSTANCE.set(client).ok().expect("Already initialized");
    }

    fn handle_settings(&self, settings: &DeathlinkSettings) {
        let mut state = self.state.write().unwrap();
        if &state.settings != settings {
            WidgetChannel::get().show_toast(if settings.enable_deathlink {
                "Deathlink enabled"
            } else {
                "Deathlink disabled"
            });
            // Go back to initial state if enable/disable
            if state.settings.enable_deathlink != settings.enable_deathlink {
                state.pending = None;
                state.previous = None;
            }
            state.settings = settings.clone();
        }
    }

    fn handle_request(&self, req: &DeathlinkRequest) {
        let mut state = self.state.write().unwrap();
        if !state.settings.enable_deathlink {
            log::info!("Ignoring deathlink {:?} because deathlink is disabled", req);
        } else if req.id == Client::get().unique_id {
            log::info!("Ignoring deathlink {:?} because id matches own game", req);
        } else {
            state.pending = Some(req.clone());
        }
    }

    fn in_main_game() -> bool {
        unsafe { CSEventFlagMan::instance() }
            .map(|ev| ev.virtual_memory_flag.get_flag(101)).unwrap_or(true)
    }

    fn show_deathlink(req: &DeathlinkRequest, ignored: bool) {
        let suffix = if ignored { " (ignored)" } else { "" };
        let msg = match &req.name {
            Some(name) => format!("Received deathlink from {name}!{suffix}"),
            None => format!("Received deathlink!{suffix}"),
        };
        WidgetChannel::get().show_toast(&msg);
    }

    fn run_task(self: Arc<Self>) {
        wait_for_system_init(&fromsoftware_shared::Program::current(), Duration::MAX)
            .expect("Could not await system init.");
        // Needed without modloader
        std::thread::sleep(Duration::from_secs(3));

        let cs_task = unsafe { CSTaskImp::instance().expect("Task system not initialized") };
        cs_task.run_recurring(move
            |_: &FD4TaskData| {
                let mut state = self.state.write().unwrap();
                if !state.settings.enable_deathlink {
                    if let Some(pending) = &state.pending {
                        log::info!("Discarding deathlink {:?} because deathlink is disabled", pending);
                        state.pending.take();
                    }
                    return;
                }
                if let Some(prev) = state.previous && prev.elapsed() < Duration::from_secs(30) {
                    if let Some(pending) = &state.pending {
                        log::info!("Discarding deathlink {:?} because {:?} within 30-second cooldown", pending, prev.elapsed());
                        Self::show_deathlink(&pending, true);
                        state.pending.take();
                    }
                    return;
                }

                // Cases where the game is not fully loaded or the player is unable to die
                let Ok(menu_man) = (unsafe { CSMenuManImp::instance() }) else {
                    return;
                };
                if menu_man.loading_screen_data.is_loading {
                    return;
                }
                let Ok(world_chr_man) = (unsafe { WorldChrMan::instance() }) else {
                    return;
                };
                let Some(player) = &world_chr_man.main_player else {
                    return;
                };
                let flags = player.modules.action_flag.action_modifiers_flags;
                // This doesn't seem to do anything for the player character
                if flags.perfect_invincibility() || flags.invincible_during_throw_attacker() || flags.invincible_excluding_throw_attacks_defender() {
                    return;
                }
                if let Some(inv_desc) = &state.debug_inv {
                    let new_desc = format!("Invincibility: perfect {} thrower {} thrown {}", flags.perfect_invincibility(), flags.invincible_during_throw_attacker(), flags.invincible_excluding_throw_attacks_defender());
                    if inv_desc != &new_desc {
                        log::info!("{}", new_desc);
                        state.debug_inv = Some(new_desc);
                    }
                }

                let client = Client::get();
                if let Some(pending) = state.pending.take() {
                    // Needs to be different player id and not within 30 seconds of previous
                    log::info!("Processing deathlink {:?} as {}", pending, client.unique_id);
                    // Assume this will kill them (it might not, can check other parts of player state)
                    // It should be a no-op if player is already dead?
                    // Talisman effect is 360700, physick 511019 has less confusing GoodsDialog message
                    // 4290 is used for respawn after Grafted Scion, not sure if there are other side effects
                    emedf::set_sp_effect(10000, 4290);
                    if Self::in_main_game() {
                        emedf::force_character_death(10000, false);
                    } else {
                        // Event 10010030 doesn't work with an actual kill, so apply effectEndurance=0.1, changeHpRate=100
                        // 4290 may already apply here if Grafted Scion has started
                        emedf::set_sp_effect(10000, 9639);
                    }
                    Self::show_deathlink(&pending, false);
                    state.previous = Some(Instant::now());
                    return;
                }

                // Possibly send deathlink. death_flag flag does not seem to get set (it's in PlayerGameData or a similar struct)
                if player.chr_flags1c5.death_flag() || player.modules.data.hp == 0 {
                    let mut excluded = false;
                    if state.settings.ignore_scripted {
                        if !Self::in_main_game() {
                            log::info!("Dead but not sending deathlink (still in tutorial)");
                            excluded = true;
                        }
                        // Try to use game state to infer Raya Lucaria abduction. Anims 7029[0-2] by FieldIns(m14_00_00_00, 18, 251)
                        // Would have to do region check for randomizer case
                        let anim = player.modules.action_request.action_request_queue.current_tae_id;
                        let hit_by = player.last_hit_by;
                        if anim / 10 == 7029 && !hit_by.is_empty() && hit_by.block_id.area() == 14 && hit_by.selector.index() == 251 {
                            log::info!("Dead but not sending deathlink (Raya Lucaria abduction)");
                            excluded = true;
                        }
                        // TODO: Also Radahn stake skip
                    };
                    if !excluded {
                        log::info!("Sending deathlink (dead: {}, hp: {})", player.chr_flags1c5.death_flag(), player.modules.data.hp);
                        client.stream_send(DeathlinkStreamRequest::Deathlink {});
                        WidgetChannel::get().show_toast("Sending deathlink!");
                    }
                    state.previous = Some(Instant::now())
                }
            },
            CSTaskGroupIndex::WorldChrMan_Respawn,
        );
    }
}

impl ClientModule for DeathlinkClient {
    fn handle_message(&self, json: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
        let api_req = DeathlinkApiRequest::deserialize(json)?;
        if let Some(raw_settings) = &api_req.common.settings {
            let api_settings = DeathlinkApiSettings::deserialize(raw_settings)?;
            if let Some(settings) = &api_settings.deathlink {
                self.handle_settings(settings);
            }
        }
        if let Some(req) = &api_req.deathlink {
            self.handle_request(req);
        }
        Ok(())
    }
}
