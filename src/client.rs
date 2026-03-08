use std::{collections::HashMap, sync::{Arc, OnceLock, RwLock}};
use rand::distr::weighted::WeightedIndex;
use rand::distr::Distribution;

use tokio::{sync::{Notify, mpsc}, time::{Duration, sleep}};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, protocol::Message}};
use futures_util::{SinkExt, StreamExt};
use serde::{Serialize, Deserialize};

use crate::{
    game::FromGame,
    items::{ItemRequest, ItemUpdater},
    name_templates::{DS1R_ENTITY_ID_TEMPLATES, DS3_ENTITY_ID_TEMPLATES, ER_ENTITY_ID_TEMPLATES, SDT_ENTITY_ID_TEMPLATES},
    spawn::{EnemySpawner, SpawnRequest}, ui::{UiRequest, WidgetChannel}
};

// Mainly state that is used by hooks
#[derive(Default, Debug)]
pub struct ClientState {
    name_mode: NameMode,
    player_name: String,
    name_claims: Vec<NameClaim>,
    // If empty or [0], select top.
    // If [a, b, c], select top >= c, otherwise select weighted between [b, c), otherwise [a, b)
    tiers: Vec<u32>,
    healthbar_rewrites: HashMap<HealthbarKey, HealthbarRewrite>,
    msg_to_entity_id: HashMap<i32, u32>,
    // Also for spawn requests
    // Also display state for timer, top names, etc
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
// #[serde(tag = "type")]
pub enum NameMode {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "test")]
    Test,
    #[default]
    #[serde(rename = "cache")]
    Cache,
    #[serde(rename = "fresh")]
    Fresh,
}

pub const SPAWN_NAME_ID: i32 = 10104137;

impl ClientState {
    pub fn update(&mut self, req: &Request) {
        for command in req.commands() {
            log::info!("Processing {:?}", command);
            match command {
                Command::SetNameMode { mode } => { 
                    let mode = *mode;
                    // Only keep on chnage mode if going to cached
                    if mode == NameMode::Fresh && mode == NameMode::Cache {
                        self.clear_cached_names();
                    }
                    self.name_mode = mode;
                },
                Command::SetTiers { tiers } => {
                    let mut tiers = tiers.clone();
                    tiers.sort();
                    self.tiers = tiers;
                }
                Command::ClearNameCache => {
                    // Assume this accompanies getting fresh claims
                    self.clear_cached_names();
                    self.name_claims.clear();
                }
                // Others handled in handling client message
                _ => (),
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
pub struct Request {
    // For in-game interactions, filter for local id
    player: Option<String>,
    spawn: Option<SpawnRequest>,
    ui: Option<UiRequest>,
    claims: Option<Vec<NameClaim>>,
    item: Option<ItemRequest>,
    // This is convenient for manual things, maybe switch in the future
    command: Option<Command>,
    commands: Option<Vec<Command>>,
}

impl Request {
    fn commands(&self) -> Vec<&Command> {
        match &self.commands {
            Some(commands) => match &self.command {
                Some(command) => commands.iter().chain([command]).collect(),
                None => commands.iter().collect(),
            }
            None => self.command.iter().collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct PlayerName {
    id: String,
    name: String,
}

// Misc control commands
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum Command {
    #[serde(rename = "dump_items")]
    DumpItems,
    #[serde(rename = "infinite_arrows")]
    InfiniteArrows { enabled: bool },
    #[serde(rename = "set_name_mode")]
    SetNameMode { mode: NameMode },
    #[serde(rename = "set_tiers")]
    SetTiers { tiers: Vec<u32> },
    #[serde(rename = "clear_name_cache")]
    ClearNameCache,
}

#[derive(Clone, Debug)]
enum ApiRequest {
    MakeClaim(NameClaim),
    SetPlayer(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct LoginResponse {
    token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum StreamRequest {
    #[serde(rename = "set_player")]
    SetPlayer(String),
}

pub struct Client {
    host: String,
    unique_id: String,
    game: FromGame,
    start: Notify,
    state: RwLock<ClientState>,
    api_send: mpsc::Sender<ApiRequest>,
    #[allow(unused)]
    stream_send: mpsc::Sender<StreamRequest>,
}

static INSTANCE: OnceLock<Arc<Client>> = OnceLock::new();

impl Client {
    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    pub fn initialize(host: &str, unique_id: &str, game: FromGame) {
        if INSTANCE.get().is_some() {
            panic!("Already initialized");
        }
        let (stream_send, stream_recv) = mpsc::channel(1000);
        let (api_send, api_recv) = mpsc::channel(1000);
        let client = Arc::new(Client {
            // localhost:8080 locally
            host: host.to_string(),
            unique_id: unique_id.to_string(),
            game: game,
            start: Notify::new(),
            state: RwLock::new(ClientState::default()),
            api_send: api_send,
            stream_send: stream_send,
        });
        let other = Arc::clone(&client);
        std::thread::spawn(move || other.receive_messages(api_recv, stream_recv));
        INSTANCE.set(client).ok().expect("Already initialized");
    }

    pub fn start(&self) {
        self.start.notify_one();
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
        let Some(rewrite) = state.make_claim(entity_id, msg_id, self.game) else {
            // log::info!("claim failed {} {}", entity_id, msg_id);
            return None;
        };
        // This includes prev_amount, though maybe ignore that in the server
        let claim = rewrite.claim.clone();
        if rewrite.update {
            // Fire and forget, it's fine. Possible race condition the set, but the caller is single-threaded at least
            let _ = self.api_send.try_send(ApiRequest::MakeClaim(claim));
        }
        Some(rewrite.name.to_string())
    }

    pub fn set_player(&self, name: &str) {
        // Use last valid name
        if name.is_empty() {
            return;
        }
        let mut state = self.state.write().unwrap();
        if name != state.player_name {
            state.player_name = name.to_string();
            let _ = self.api_send.try_send(ApiRequest::SetPlayer(name.to_string()));
        }
    }

    fn connect(&self) {
        let state = self.state.write().unwrap();
        if !state.player_name.is_empty() {
            let _ = self.api_send.try_send(ApiRequest::SetPlayer(state.player_name.to_string()));
        }
    }

    fn handle_message(&self, req: &Request) {
        {
            let mut state = self.state.write().unwrap();
            state.update(req);
        }
        // The rest is only Elden Ring
        if self.game != FromGame::ER {
            return;
        }
        if let Some(spawn) = &req.spawn {
            EnemySpawner::get().spawn_req(spawn);
        }
        let mut excluded = false;
        // Fix me
        if let Some(select) = &req.player && select != &self.unique_id {
            excluded = true;
        }
        if let Some(ui) = &req.ui {
            WidgetChannel::get().handle_request(ui);
        }
        if let Some(item) = &req.item {
            // Do name/id filtering for items
            if excluded {
                return;
            }
            // Maybe send back error response, rate limited. Or imgui it
            if let Err(e) = ItemUpdater::get().give(item) {
                log::error!("Couldn't equip item: {}", e);
            }
        }
        for command in req.commands() {
            match command {
                Command::DumpItems => ItemUpdater::get().dump_items().unwrap(),
                Command::InfiniteArrows { enabled } => {
                    if !excluded {
                        ItemUpdater::get().set_infinite_arrows(*enabled);
                    }
                },
                // Others handled in client state update
                _ => (),
            }
        }
    }

    fn get_http_base(&self) -> String {
        let protocol = if self.host.starts_with("localhost") { "http" } else { "https" };
        format!("{}://{}", protocol, self.host)
    }

    fn get_ws_base(&self) -> String {
        let protocol = if self.host.starts_with("localhost") { "ws" } else {  "wss" };
        format!("{}://{}", protocol, self.host)
    }

    #[tokio::main]
    async fn receive_messages(&self, mut api_recv: mpsc::Receiver<ApiRequest>, mut stream_recv: mpsc::Receiver<StreamRequest>) {
        // First wait for all dependencies to be set up
        self.start.notified().await;

        let http_client = reqwest::ClientBuilder::new().timeout(Duration::from_secs(10)).build().unwrap();

        // Just do this once, it's super long expiry, otherwise retry gets annoying
        let password = include_str!("server.txt").trim();
        let auth = loop {
            let req_client = http_client.clone();
            let url = format!("{}/api/login", self.get_http_base());
            let req = LoginRequest { username: "game".to_string(), password: password.to_string() };
            match req_client.post(url).json(&req).send().await {
                Ok(res) => {
                    if res.status() == 200 {
                        match res.json::<LoginResponse>().await {
                            Ok(res) => {
                                log::info!("Authenticated with server");
                                break format!("Bearer {}", res.token);
                            }
                            Err(e) => {
                                log::error!("Couldn't parse login result: {e}");
                            }
                        }
                    } else {
                        log::error!("Login failed: {:?}", res.text().await);
                    }
                },
                Err(e ) => {
                    log::error!("Couldn't connect to login: {e}");
                }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        };
        let auth = Arc::new(auth);
        let ws_auth = auth.clone();

        // Misc client tasks to handle in async context
        tokio::spawn(async move {
            // It's static, I guess
            let client = Client::get();
            // let http_client = Arc::new(http_client);
            while let Some(req) = api_recv.recv().await {
                log::info!("-> Sending {:?}", req);
                let req_client = http_client.clone();
                let api_auth = auth.clone();
                match req {
                    ApiRequest::MakeClaim(claim) => {
                        tokio::spawn(async move {
                            // https://stackoverflow.com/questions/73895393/how-to-cheaply-send-a-delay-message
                            // We're forking anyway for parallelism. Actually nvm
                            // tokio::time::sleep(Duration::from_secs(1)).await;
                            let url = format!("{}/api/claims", client.get_http_base());
                            let res = req_client.post(&url).json(&claim).header("Authorization", api_auth.to_string()).send().await;
                            Self::log_error(&url, &res);
                        });
                    },
                    ApiRequest::SetPlayer(player) => {
                        tokio::spawn(async move {
                            let url = format!("{}/api/players", client.get_http_base());
                            let req = PlayerName { id: client.unique_id.to_string(), name: player };
                            let res = req_client.post(&url).json(&req).header("Authorization", api_auth.to_string()).send().await;
                            Self::log_error(&url, &res);
                        });
                    },
                };
            }
            log::error!("API processor ran out of queue");
        });

        // Don't deal with url/uri libraries for now
        let url = format!("{}/socket?id={}", self.get_ws_base(), self.unique_id);
        loop {
            // IntoClientRequest handles a bunch of fields that misbehave otherwise
            let mut websocket_req = (&url).into_client_request().unwrap();
            websocket_req.headers_mut().insert("Authorization", ws_auth.parse().unwrap());
            // Leave out URL
            log::info!("Connecting as {}", self.unique_id);
            match connect_async(websocket_req).await {
                Ok((ws_stream, _)) => {
                    let (mut sink, mut stream) = ws_stream.split();
                    // TODO: Maybe should just use websocket for this, since it's done in the loop anyway
                    // It should be done as soon as possible, but aside from doing it here, it won't be done if the name doesn't change
                    // Unfortunately can return 502 during server restart
                    tokio::spawn(async {
                        let client = Client::get();
                        client.connect();
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        client.connect();
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        client.connect();
                    });
                    loop {
                        tokio::select! {
                            Some(message_result) = stream.next() => {
                                match message_result {
                                    Ok(message) => {
                                        if !message.is_ping() && let Ok(text) = message.into_text() {
                                            log::info!("Received {text}");
                                            match serde_json::from_str::<Request>(&text) {
                                                Ok(msg) => self.handle_message(&msg),
                                                Err(e) => log::error!("Failed to parse: {e}"),
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        log::error!("Failed to receive: {e}");
                                        break;
                                    }
                                }
                            },
                            Some(data) = stream_recv.recv() => {
                                // Use client if any state is needed
                                if let Ok(text) = serde_json::to_string(&data) {
                                    if let Err(e) = sink.send(Message::from(text)).await {
                                        log::error!("Failed to send: {e}");
                                        break;
                                    }
                                }
                            },
                            else => break,
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to connect: {}", e);
                    sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    fn log_error(url: &str, result: &Result<reqwest::Response, reqwest::Error>) {
        match result {
            Ok(res) if res.status() != 200 => {
                log::error!("{} returned status {}", url, res.status());
            }
            Err(e) => {
                log::error!("{} returned {:?}", url, e);
            }
            _ => (),
        };
    }
}

