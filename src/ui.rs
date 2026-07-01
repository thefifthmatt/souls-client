use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Local};
use debug::InputBlocker;
use eldenring::cs::CSWindowImp;
use eldenring::util::system::wait_for_system_init;
use fromsoftware_shared::FromStatic;
use hudhook::RenderContext;
use hudhook::imgui::{Condition, Context, FontAtlas, FontId, FontSource, StyleVar, Ui, WindowFlags};
use hudhook::{ImguiRenderLoop, hooks::dx12::ImguiDx12Hooks, Hudhook};
use serde::{Serialize, Deserialize};

use crate::client::{Client, ClientModule, parse_network_time};

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct UiApiRequest {
    ui: Option<UiRequest>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct UiRequest {
    pub toast: Option<String>,
    pub timer: Option<String>,
    pub info: Option<Vec<String>>,
}

pub struct WidgetChannel {
    // crossbeam_channel (used by practice tool) seems better but just use something simple
    toast_recv: Mutex<mpsc::Receiver<String>>,
    toast_send: mpsc::Sender<String>,
    // Set by server (avoid lock contention if at all possible)
    pub timer: RwLock<Option<DateTime<FixedOffset>>>,
    pub info: RwLock<Vec<String>>,
}

static INSTANCE: OnceLock<Arc<WidgetChannel>> = OnceLock::new();

impl WidgetChannel {
    fn new() -> Self {
        let (toast_send, toast_recv) = mpsc::channel();
        // To test: "a\nb\nc".split("\n").map(|s| s.to_string()).collect()
        Self {
            toast_send,
            toast_recv: Mutex::new(toast_recv),
            timer: RwLock::new(None),
            info: RwLock::new(vec![]),
        }
    }

    pub fn get() -> &'static Self {
        INSTANCE.get().expect("Accessed before initialization")
    }

    pub fn initialize() {
        if INSTANCE.get().is_some() {
            panic!("Already initialized");
        }
        let channel = Arc::new(WidgetChannel::new());
        Widget::initialize();
        Client::get().register_module(channel.clone());
        INSTANCE.set(channel).ok().expect("Already initialized");
    }

    pub fn show_toast(&self, toast: &str) {
        let _ = self.toast_send.send(toast.to_string());
    }

    fn handle_request(&self, req: &UiRequest) {
        if let Some(toast) = &req.toast {
            self.show_toast(toast);
        }
        if let Some(timer) = &req.timer {
            // TODO: Set infinite arrows on match start. This was previously done here if timer != ""
            *self.timer.write().unwrap() = parse_network_time(timer);
        }
        if let Some(info) = &req.info {
            *self.info.write().unwrap() = info.clone();
        }
    }

    // Called from ImGui loop
    fn recv(&self) -> Vec<String> {
        let recv = self.toast_recv.lock().unwrap();
        recv.try_iter().collect()
    }
}

impl ClientModule for WidgetChannel {
    fn handle_message(&self, json: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
        let api_req = UiApiRequest::deserialize(json)?;
        if let Some(req) = &api_req.ui {
            self.handle_request(req);
        }
        Ok(())
    }
}

// Wrapper object just to force Send/Sync for it
#[derive(Debug, Clone, Copy)]
pub struct Fonts {
    pub small: FontId,
    pub small_bold: FontId,
    pub regular: FontId,
    pub big: FontId,
    pub very_big: FontId,
}
unsafe impl Send for Fonts {}
unsafe impl Sync for Fonts {}

pub struct UiData {
    pub fonts: Fonts,
}

// This is the actual renderer, but it's kept internal to the channel as that's the public-facing interface
#[derive(Default, Debug)]
struct Widget
{
    toasts: Vec<(Instant, String)>,
    // TODO: If this is needed in other modules, make a widget context
    font: Option<Fonts>,
    scale: f32,
}

impl Widget {
    fn new() -> Self {
        Default::default()
    }

    fn initialize() {
        std::thread::spawn(|| {
            wait_for_system_init(&fromsoftware_shared::Program::current(), Duration::MAX)
                .expect("Could not await system init.");

            if let Err(e) = Hudhook::builder().with::<ImguiDx12Hooks>(Widget::new()).build().apply() {
                log::error!("Couldn't apply hook: {e:?}");
                hudhook::eject();
            }
        });
    }

    fn update_scale(&mut self) -> bool {
        if let Ok(window) = unsafe { CSWindowImp::instance() } {
            self.scale = window.screen_width as f32 / 1920.0;
            return true;
        }
        return false;
    }
}

fn add_font(fonts: &mut FontAtlas, size: f32, data: &'static [u8]) -> FontId {
    fonts.add_font(&[FontSource::TtfData {
        // TODO: Make feature for it
        data,
        size_pixels: size,
        config: None,
    }])
}

impl ImguiRenderLoop for Widget {
    fn initialize(&mut self, ctx: &mut Context, _: &mut dyn RenderContext) {
        // TODO: Does this work moved first?
        if self.update_scale() {
            ctx.style_mut()
                .scale_all_sizes(self.scale);
            // There's also per-window font scaling
            ctx.io_mut().font_global_scale = self.scale;
        }
        let fonts = ctx.fonts();
        self.font = Some(Fonts {
            small: add_font(fonts, 22.0, include_bytes!("fonts/OpenSans_Condensed-Regular.ttf")),
            small_bold: add_font(fonts, 22.0, include_bytes!("fonts/OpenSans_Condensed-Bold.ttf")),
            regular: add_font(fonts, 36.0, include_bytes!("fonts/AgmenaW1GForBandai.ttf")),
            // Could make these feature-dependent
            big: add_font(fonts, 48.0, include_bytes!("fonts/AgmenaW1GForBandai.ttf")),
            // 60 or 72
            very_big: add_font(fonts, 72.0, include_bytes!("fonts/AgmenaW1GForBandai.ttf")),
        });
        // *WidgetChannel::get().timer.write().unwrap() = Some(Local::now().fixed_offset());
    }

    fn render(&mut self, ui: &mut Ui) {
        let blocker = InputBlocker::get_instance();
        unsafe {
            // Don't let this be a blocking error, especially since InputBlocker cannot coexist in multiple dlls
            match blocker.install_hooks() {
                Err(e) => log::error!("Failed to install input hooks for ImGui: {e}"),
                _ => (),
            }
        }

        let channel = WidgetChannel::get();

        // This is simple enough that it can use machine-relative Instants, but the timer getting external input must use DateTime
        let now = Instant::now();
        self.toasts.extend(channel.recv().into_iter()
            .inspect(|text| log::info!("Toast: {}", text))
            .map(|text| (now, text)));
        self.toasts.retain(|(then, _)| then.elapsed() < std::time::Duration::from_secs(4));

        let fonts = self.font.unwrap();
        let regular_font = ui.push_font(fonts.regular);

        // NO_INPUTS = NO_NAV | NO_MOUSE_INPUTS
        // NO_NAV = NO_NAV_INPUTS | NO_NAV_FOCUS
        // NO_DECORATIONS = NO_TITLE_BAR | NO_RESIZE | NO_SCROLLBAR | NO_COLLAPSE
        // NO_INPUTS and NO_MOUSE_INPUTS effectively imply NO_COLLAPSE (double-click to collapse)
        // ImGui flags/styles used by practice tool toasts
        let invisible_flags =
            WindowFlags::NO_TITLE_BAR | WindowFlags::NO_RESIZE | WindowFlags::NO_MOVE | WindowFlags::NO_SCROLLBAR 
            | WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_INPUTS;

        let invisible_styles = vec![StyleVar::WindowRounding(0.0), StyleVar::FrameBorderSize(0.0), StyleVar::WindowBorderSize(0.0)];

        let style_tokens: Vec<_> = invisible_styles.iter().map(|&v| ui.push_style_var(v)).collect();

        let io = ui.io();
        blocker.block_from_io(io);

        let [dw, dh] = io.display_size;

        // Height was previously 0.21 1.0, moved to avoid boss healthbar overlap
        let size = [dw * 0.6, dh * 0.18];
        let pos = [dw * 0.18, dh * 1.0];
        ui.window("##toasts")
            .flags(invisible_flags)
            .bg_alpha(0.0)
            .position_pivot([0.0, 1.0])
            .position(pos, Condition::Always)
            .size(size, Condition::Always)
            .build(|| {
                for l in self.toasts.iter() {
                    ui.text(&l.1);
                }
            });

        let size = [dw * 0.3, dh * 0.5];
        let pos = [dw * 0.025, dh * 0.12];
        ui.window("##sidebar")
            .flags(invisible_flags)
            .bg_alpha(0.0)
            .position_pivot([0.0, 0.0])
            .position(pos, Condition::Always)
            .size(size, Condition::Always)
            .build(|| {
                let channel = WidgetChannel::get();
                let big_font = ui.push_font(fonts.big);
                // Hope no deadlock :)
                {
                    let title = match *channel.timer.read().unwrap() {
                        Some(instant) => get_elapsed_time(instant),
                        None => "".to_string(),
                    };
                    ui.text(title);
                }
                big_font.end();
                let infos = channel.info.read().unwrap();
                for info in infos.iter() {
                    ui.text(info);
                    if ui.cursor_pos()[1] >= size[1] - 36.0 {
                        break;
                    }
                }
            });

        let ui_data = UiData { fonts: fonts };
        Client::get().render(ui, &ui_data);

        style_tokens.into_iter().rev().for_each(|v| v.pop());
        regular_font.end();
    }
}

fn get_elapsed_time(time: DateTime<FixedOffset>) -> String {
    let elapsed = Local::now().fixed_offset() - time;
    let secs = 0.max(elapsed.as_seconds_f64() as i32);
    format!("{}:{:02}", secs / 60, secs % 60)
}
