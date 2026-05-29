use std::{collections::HashMap, sync::{Arc, OnceLock, RwLock}};
use rand::distr::weighted::WeightedIndex;
use rand::distr::Distribution;

use serde::{Serialize, Deserialize};

use crate::{
    client::{ApiPostRequest, Client, ClientModule},
    game::FromGame,
    names::hooks::hook_messages,
    names::name_templates::{DS1R_ENTITY_ID_TEMPLATES, DS3_ENTITY_ID_TEMPLATES, ER_ENTITY_ID_TEMPLATES, SDT_ENTITY_ID_TEMPLATES},
};

mod hooks;
mod name_templates;

// Mainly state that is used by hooks
#[derive(Default, Debug)]
pub struct NameClientState {
    name_mode: NameMode,
    name_claims: Vec<NameClaim>,
    // If empty or [0], select top.
    // If [a, b, c], select top >= c, otherwise select weighted between [b, c), otherwise [a, b)
    tiers: Vec<u32>,
    healthbar_rewrites: HashMap<HealthbarKey, HealthbarRewrite>,
    msg_to_entity_id: HashMap<i32, u32>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
// #[serde(tag = "type")]
pub enum NameMode {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "test")]
    Test,
    #[serde(rename = "cache")]
    Cache,
    #[serde(rename = "fresh")]
    Fresh,
}

pub const SPAWN_NAME_ID: i32 = 10104137;

impl NameClientState {
    pub fn update(&mut self, req: &NameApiRequest) {
        for command in req.names.iter().flatten() {
            log::info!("Processing {:?}", command);
            match command {
                NameCommand::SetNameMode { mode } => { 
                    let mode = *mode;
                    // Only keep on chnage mode if going to cached
                    if mode == NameMode::Fresh && mode == NameMode::Cache {
                        self.clear_cached_names();
                    }
                    self.name_mode = mode;
                },
                NameCommand::SetTiers { tiers } => {
                    let mut tiers = tiers.clone();
                    tiers.sort();
                    self.tiers = tiers;
                }
                NameCommand::ClearNameCache => {
                    // Assume this accompanies getting fresh claims
                    self.clear_cached_names();
                    self.name_claims.clear();
                }
            }
        }
        if let Some(claims) = &req.claims {
            // Continue adding claims regardless of mode
            let mut updated = 0;
            for claim in claims {
                match self.name_claims.iter_mut().find(|c| c.name == claim.name) {
                    Some(exist_claim) => {
                        exist_claim.amount = claim.amount;
                        exist_claim.ignore = claim.ignore;
                        exist_claim.claimed = false;
                    }
                    None => {
                        self.name_claims.push(claim.clone());
                        updated += 1;
                    }
                }
            }
            if updated > 0 { log::info!("Claims: Added {}", updated); }
        }
    }

    fn top_claim_mut(&mut self) -> Option<&mut NameClaim> {
        // Above top amount, select top. Can set really high to prevent this
        let min_for_top = self.tiers.last().copied().unwrap_or(0);
        // iter_mut is pretty inconvenient as it takes ownership away, do this immutably for now
        let top = self.name_claims.iter()
            .filter(|c| c.is_usable() && c.amount >= min_for_top)
            .max_by_key(|c| c.amount)
            .map(|c| c.name.clone());
        if let Some(select) = top {
            return self.name_claims.iter_mut().find(|c| c.name == select);
        }
        for window in self.tiers.windows(2).rev() {
            let min = window[0];
            let max = window[1];
            let range_claims: Vec<&NameClaim> = self.name_claims.iter()
                .filter(|c| c.is_usable() && c.amount >= min && c.amount < max)
                .collect();
            if !range_claims.is_empty() {
                let weights: Vec<u32> = range_claims.iter().map(|c| c.amount).collect();
                let dist = WeightedIndex::new(&weights).unwrap();
                let mut rng = rand::rng();
                let index = dist.sample(&mut rng);
                // There's probably a way to do ownership correctly but keep it simple here
                let select = range_claims[index].name.clone();
                return self.name_claims.iter_mut().find(|c| c.name == select);
            }
        }
        None
    }

    fn healthbar_key(&self, entity_id: u32, msg_id: i32) -> HealthbarKey {
        if msg_id == SPAWN_NAME_ID { HealthbarKey::EntityId(entity_id) } else { HealthbarKey::MsgId(msg_id) }
    }

    fn get_msg_boss(&self, entity_id: u32, msg_id: i32) -> Option<u32> {
        if entity_id > 0 {
            Some(entity_id)
        } else if let Some(&boss_id) = self.msg_to_entity_id.get(&msg_id) {
            Some(boss_id)
        } else {
            None
        }
    }

    fn set_msg_boss(&mut self, entity_id: u32, msg_id: i32) {
        self.msg_to_entity_id.insert(msg_id, entity_id);
        log::info!("Healthbar {} {} in {:?}", entity_id, msg_id, self.name_mode);
        if self.name_mode == NameMode::Fresh {
            self.healthbar_rewrites.remove(&HealthbarKey::MsgId(msg_id));
        }
    }

    fn clear_cached_names(&mut self) {
        // Remove all FMG messages
        self.healthbar_rewrites.retain(|key, _| matches!(key, HealthbarKey::EntityId(_)));
    }

    // This can probably be encapsulated in make_claim?
    fn get_healthbar_rewrite(&self, entity_id: u32, msg_id: i32) -> Option<&HealthbarRewrite> {
        let key = self.healthbar_key(entity_id, msg_id);
        self.healthbar_rewrites.get(&key)
    }

    pub fn set_spawn_name(&mut self, entity_id: u32, claim: NameClaim) {
        let key = HealthbarKey::EntityId(entity_id);
        let name = claim.name.to_string();
        let rewrite = HealthbarRewrite { claim: claim, name: name, update: false };
        self.healthbar_rewrites.insert(key, rewrite);
    }

    // Makes a new claim if possible, mutating the healthbar map and returning the consumed claim
    pub fn make_claim(&mut self, entity_id: u32, msg_id: i32, game: FromGame) -> Option<&HealthbarRewrite> {
        let key = self.healthbar_key(entity_id, msg_id);
        if let HealthbarKey::EntityId(_) = &key {
            return self.healthbar_rewrites.get(&key);
        }
        // None mode still allows spawn names
        if self.name_mode == NameMode::None {
            return None;
        }
        // Other cases use template
        let templates = match game {
            FromGame::DS1R => &DS1R_ENTITY_ID_TEMPLATES,
            FromGame::DS3 => &DS3_ENTITY_ID_TEMPLATES,
            FromGame::SDT => &SDT_ENTITY_ID_TEMPLATES,
            FromGame::ER => &ER_ENTITY_ID_TEMPLATES,
        };
        let mut template = match templates.get(&entity_id) {
            Some(template) => template,
            None => {
                if game == FromGame::ER && (entity_id / 100) % 10 != 8 {
                    return None;
                }
                &"$1"
            },
        };
        // In specific case of vanilla Malenia, set template
        if entity_id == 15000800 && msg_id == 902120001 {
            template = &"$1, Goddess of Rot";
        }
        let claim;
        let test;
        if self.name_mode == NameMode::Test {
            static COUNT: std::sync::Mutex<u32> = std::sync::Mutex::new(1);
            let mut count = COUNT.lock().unwrap();
            claim = NameClaim { name: format!("[{}-{} #{}]", entity_id, msg_id, *count), ..Default::default() };
            *count += 1;
            test = true;
        } else {
            let Some(top_claim) = self.top_claim_mut() else {
                return None;
            };
            top_claim.claimed = true;
            top_claim.amount = 0;
            claim = top_claim.clone();
            test = false;
        }
        let enemy_name = template.replace("$1", &claim.name);
        let rewrite = HealthbarRewrite { claim: claim, name: enemy_name, update: !test };
        // Some(self.healthbar_rewrites.entry(key).or_insert_with(|| rewrite.into()))
        self.healthbar_rewrites.insert(key.clone(), rewrite);
        self.healthbar_rewrites.get(&key)
    }
}

// Local state for claims, as well as sent and received for updates
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct NameClaim {
    pub name: String,
    pub amount: u32,
    #[serde(default)]
    pub ignore: bool,
    // Use to avoid locally
    #[serde(skip)]
    claimed: bool,
}

impl NameClaim {
    pub fn new(name: String, amount: u32) -> Self {
        Self { name: name, amount: amount, ..Default::default() }
    }

    pub fn is_usable(&self) -> bool {
        self.amount > 0 && !self.ignore
    }
}

impl ApiPostRequest for NameClaim {
    fn url(&self) -> &'static str { "/api/claims" }
    // fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> { serde_json::to_value(self) }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HealthbarKey {
    EntityId(u32),
    MsgId(i32),
}

#[derive(Clone, Default, Debug)]
pub struct HealthbarRewrite {
    // claim at the time the rewrite was added
    claim: NameClaim,
    name: String,
    update: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct NameApiRequest {
    // For in-game interactions, filter for local id
    claims: Option<Vec<NameClaim>>,
    names: Option<Vec<NameCommand>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum NameCommand {
    #[serde(rename = "set_name_mode")]
    SetNameMode { mode: NameMode },
    #[serde(rename = "set_tiers")]
    SetTiers { tiers: Vec<u32> },
    #[serde(rename = "clear_name_cache")]
    ClearNameCache,
}

pub struct NameClient {
    state: RwLock<NameClientState>,
}

static INSTANCE: OnceLock<Arc<NameClient>> = OnceLock::new();

impl NameClient {
    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    pub fn initialize() {
        if INSTANCE.get().is_some() {
            panic!("Already initialized");
        }
        let name_client = Arc::new(NameClient {
            state: RwLock::new(NameClientState::default()),
        });
        Client::get().register_module(name_client.clone());
        INSTANCE.set(name_client).ok().expect("Already initialized");
    }

    // Probably best to encapsulate this, though state_mut taking FnOnce(&mut ClientState)
    pub fn set_spawn_name(&self, entity_id: u32, claim: NameClaim) {
        let mut state = self.state.write().unwrap();
        state.set_spawn_name(entity_id, claim);
    }

    pub fn set_msg_boss(&self, entity_id: u32, msg_id: i32) {
        let mut state = self.state.write().unwrap();
        state.set_msg_boss(entity_id, msg_id);
    }

    pub fn claim_name(&self, entity_id: u32, msg_id: i32) -> Option<String> {
        let mut state = self.state.write().unwrap();
        let Some(entity_id) = state.get_msg_boss(entity_id, msg_id) else {
            return None;
        };
        // Look up from cache
        if let Some(rewrite) = state.get_healthbar_rewrite(entity_id, msg_id) {
            return Some(rewrite.name.to_string());
        }
        let client = Client::get();
        let Some(rewrite) = state.make_claim(entity_id, msg_id, client.game) else {
            // log::info!("claim failed {} {}", entity_id, msg_id);
            return None;
        };
        // This includes prev_amount, though maybe ignore that in the server
        let claim = rewrite.claim.clone();
        if rewrite.update {
            // Previously this waited for Seamless Co-op purposes, but more names are better probably
            // https://stackoverflow.com/questions/73895393/how-to-cheaply-send-a-delay-message
            client.api_post(claim);
        }
        Some(rewrite.name.to_string())
    }
}

impl ClientModule for NameClient {
    fn handle_message(&self, json: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
        let req = NameApiRequest::deserialize(json)?;
        let mut state = self.state.write().unwrap();
        state.update(&req);
        Ok(())
    }

    fn hook(&self) {
        // This is done separately in case Arxan needs to be dealt with first
        let client = Client::get();
        hook_messages(client.game);
    }
}
