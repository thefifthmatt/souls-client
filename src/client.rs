use std::sync::{Arc, OnceLock, RwLock};
use chrono::{DateTime, FixedOffset};
use eldenring::{
    cs::{CSTaskGroupIndex, PlayerGameData, CSTaskImp},
    fd4::FD4TaskData,
    util::system::wait_for_system_init
};
use fromsoftware_shared::{FromStatic, SharedTaskImpExt};

use tokio::{sync::{Notify, mpsc}, time::{Duration, sleep}};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, protocol::Message}};
use futures_util::{SinkExt, StreamExt};
use serde::{Serialize, Deserialize};

use crate::{
    game::{FromGame, PlayerGameDataExt},
};

pub trait StreamRequest: Send + erased_serde::Serialize {}
erased_serde::serialize_trait_object!(StreamRequest);

pub trait ApiPostRequest: Send + erased_serde::Serialize {
    fn url(&self) -> &'static str;
}
erased_serde::serialize_trait_object!(ApiPostRequest);

pub trait ClientModule: Send + Sync {
    // Process websocket message. This is sent to all modules, so deserialization should not be incompatible
    // between different modules. Different modules should use different field names (aside from CommonRequest).
    // This could be validated with serde_fields.
    fn handle_message(&self, json: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>>;

    // Memory edits which may need to wait until after Arxan is disabled
    fn hook(&self) {}

    #[cfg(feature = "ui")]
    fn render(&self, _ui: &hudhook::imgui::Ui, _ui_data: &crate::ui::UiData) {}
}

// Request fields shared by multiple modules which can be inserted into other requests using #[serde(flatten)]
#[allow(unused)]
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct CommonRequest {
    // For in-game interactions, filter for local id
    pub player: Option<String>,
}

// Request object for server interactions. Currently not used in favor of CoreStreamRequest.
#[allow(unused)]
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct PlayerName {
    id: String,
    name: String,
}

impl ApiPostRequest for PlayerName {
    fn url(&self) -> &'static str { "/api/players" }
    // fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> { serde_json::to_value(self) }
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
enum CoreStreamRequest {
    #[serde(rename = "set_player")]
    SetPlayer { name: String },
}
impl StreamRequest for CoreStreamRequest {}

pub struct Client {
    host: String,
    pub unique_id: String,
    pub game: FromGame,
    player_name: RwLock<String>,
    modules: RwLock<Vec<Arc<dyn ClientModule>>>,
    start: Notify,
    api_send: mpsc::Sender<Box<dyn ApiPostRequest>>,
    stream_send: mpsc::Sender<Box<dyn StreamRequest>>,
}

static INSTANCE: OnceLock<Arc<Client>> = OnceLock::new();

impl Client {
    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    #[allow(unused)]
    pub fn register_module(&self, module: Arc<dyn ClientModule>) {
        let mut modules = self.modules.write().unwrap();
        modules.push(module);
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
            player_name: RwLock::new(String::default()),
            modules: RwLock::new(Vec::new()),
            game: game,
            start: Notify::new(),
            api_send: api_send,
            stream_send: stream_send,
        });
        let other = client.clone();
        std::thread::spawn(move || other.receive_messages(api_recv, stream_recv));
        let other = client.clone();
        std::thread::spawn(move || other.run_task());
        INSTANCE.set(client).ok().expect("Already initialized");
    }

    pub fn start(&self) {
        self.start.notify_one();
    }

    // This sends a request as fire-and-forget, which is generally required during game logic.
    // To actually wait for a response, a different method would need to be devised.
    #[allow(unused)]
    pub fn api_post(&self, req: impl ApiPostRequest + 'static) {
        // Will typically only fail when queue full
        let _ = self.api_send.try_send(Box::new(req));
    }

    pub fn stream_send(&self, req: impl StreamRequest + 'static) {
        let _ = self.stream_send.try_send(Box::new(req));
    }

    pub fn set_player(&self, name: &str) {
        // Use last valid name
        if name.is_empty() {
            return;
        }
        let mut stored_name = self.player_name.write().unwrap();
        if name != *stored_name {
            *stored_name = name.to_string();
            // let _ = self.api_send.try_send(ApiRequest::SetPlayer(name.to_string()));
            self.stream_send(CoreStreamRequest::SetPlayer { name: name.to_string() });
        }
    }

    pub fn hook(&self) {
        for module in self.modules.read().unwrap().iter() {
            module.hook();
        }
    }

    #[cfg(feature = "ui")]
    pub fn render(&self, ui: &hudhook::imgui::Ui, ui_data: &crate::ui::UiData) {
        for module in self.modules.read().unwrap().iter() {
            module.render(ui, ui_data);
        }
    }

    fn connect(&self) {
        // This can probably be done in loop directly?
        let name = self.player_name.read().unwrap();
        if !name.is_empty() {
            // let _ = self.api_send.try_send(ApiRequest::SetPlayer(name.to_string()));
            self.stream_send(CoreStreamRequest::SetPlayer { name: name.to_string() });
        }
    }

    // No error handling currently...
    fn handle_message(&self, json: &serde_json::Value) {
        for module in self.modules.read().unwrap().iter() {
            match module.handle_message(json) {
                Err(e) => log::error!("handle_message failed: {e}"),
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
    async fn receive_messages(
            &self,
            mut api_recv: mpsc::Receiver<Box<dyn ApiPostRequest>>,
            mut stream_recv: mpsc::Receiver<Box<dyn StreamRequest>>) {
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
                log::info!("-> Sending {:?}", serde_json::to_string(&req));
                let req_client = http_client.clone();
                let api_auth = auth.clone();
                let url = format!("{}/{}", client.get_http_base(), req.url());
                let res = req_client.post(&url).json(&req).header("Authorization", api_auth.to_string()).send().await;
                Self::log_error(&url, &res);
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
            let log_id = if url.starts_with("localhost") { &url } else { &self.unique_id };
            log::info!("Connecting as {}", log_id);
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
                        log::info!("Got to main loop");
                        tokio::select! {
                            Some(message_result) = stream.next() => {
                                match message_result {
                                    Ok(message) => {
                                        if !message.is_ping() && let Ok(text) = message.into_text() {
                                            log::info!("Received {text}");
                                            match serde_json::from_str::<serde_json::Value>(&text) {
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
                                // Currently, message requests are client-internal, but this could accept
                                // an arbitrary JSON value if needed.
                                match serde_json::to_string(&data) {
                                    Ok(text) => {
                                        log::info!("Sending on websocket: {text}");
                                        if let Err(e) = sink.send(Message::from(text)).await {
                                            log::error!("Failed to send: {e}");
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("Failed to serialize message: {e}");
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

    fn run_task(self: Arc<Self>) {
        // Currently Elden Ring only
        if self.game != FromGame::ER {
            return;
        }

        wait_for_system_init(&fromsoftware_shared::Program::current(), Duration::MAX)
            .expect("Could not await system init.");
        // Needed without modloader
        std::thread::sleep(Duration::from_secs(3));

        let cs_task = unsafe { CSTaskImp::instance().expect("Task system not initialized") };
        cs_task.run_recurring(move
            |_: &FD4TaskData| {
                let client = Client::get();
                if let Some(player_game_data) = unsafe { PlayerGameData::main_instance() } {
                    client.set_player(&player_game_data.character_name());
                }
            },
            // Idk
            CSTaskGroupIndex::HavokWorldUpdate_Pre,
        );
    }
}

// Utility

// Server timestamp shouldn't be bad, just log if so
pub fn parse_network_time(timer: &str) -> Option<DateTime<FixedOffset>> {
    if timer == "" {
        None
    } else {
        match DateTime::parse_from_rfc3339(timer) {
            Ok(time) => Some(time),
            Err(e) => {
                log::error!("Bad timestamp {}: {}", timer, e);
                None
            },
        }
    }
}