use std::sync::{LazyLock, Mutex, RwLock};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Local};
use eldenring::cs::CSWindowImp;
use eldenring::util::system::wait_for_system_init;
use fromsoftware_shared::FromStatic;
use hudhook::RenderContext;
use hudhook::imgui::{Condition, Context, FontAtlas, FontId, FontSource, StyleVar, Ui, WindowFlags};
use hudhook::{ImguiRenderLoop, hooks::dx12::ImguiDx12Hooks, Hudhook};
use serde::{Serialize, Deserialize};

use crate::items::ItemUpdater;

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
    pub timer: RwLock<Option<DateTime<FixedOffset>>>,
    pub info: RwLock<Vec<String>>,
}

impl WidgetChannel {
    fn new() -> Self {
        let (toast_send, toast_recv) = mpsc::channel();
        // To test: "a\nb\nc".split("\n").map(|s| s.to_string()).collect()
        Self { toast_send, toast_recv: Mutex::new(toast_recv), timer: RwLock::new(None), info: RwLock::new(vec![]) }
    }

    pub fn get() -> &'static Self {
        static INSTANCE: LazyLock<WidgetChannel> = LazyLock::new(|| WidgetChannel::new());
        &INSTANCE
    }

    pub fn show_toast(&self, toast: &str) {
        let _ = self.toast_send.send(toast.to_string());
    }

    pub fn handle_request(&self, req: &UiRequest) {
        if let Some(toast) = &req.toast {
            self.show_toast(toast);
        }
        if let Some(timer) = &req.timer {
            // Hijack this, maybe expand to general match state if there's more like this
            ItemUpdater::get().set_infinite_arrows(timer != "");
            if timer == "" {
                *self.timer.write().unwrap() = None;
            } else {
                match DateTime::parse_from_rfc3339(timer) {
                    Ok(time) => *self.timer.write().unwrap() = Some(time),
                    Err(e) => log::error!("Bad timestamp {}: {}", timer, e),
                };
            }
        }
        if let Some(info) = &req.info {
            *self.info.write().unwrap() = info.clone();
        }
    }

    fn recv(&self) -> Vec<String> {
        let recv = self.toast_recv.lock().unwrap();
        recv.try_iter().collect()
    }
}

// Wrapper object just to force Send/Sync for it
#[derive(Debug, Clone, Copy)]
struct Fonts {
    regular: FontId,
    big: FontId,
}
unsafe impl Send for Fonts {}
unsafe impl Sync for Fonts {}

#[derive(Default, Debug)]
pub struct Widget
{
    toasts: Vec<(Instant, String)>,
    font: Option<Fonts>,
    scale: f32,
}

impl Widget {
    fn new() -> Self {
        Default::default()
    }

    pub fn initialize() {
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

fn add_font(fonts: &mut FontAtlas, size: f32) -> FontId {
    fonts.add_font(&[FontSource::TtfData {
        // Not free. Find it or else substitute other standard font
        data: include_bytes!("AgmenaW1GForBandai.ttf"),
        size_pixels: size,
        config: None,
    }])
}

impl ImguiRenderLoop for Widget {
    fn initialize(&mut self, ctx: &mut Context, _: &mut dyn RenderContext) {
        let fonts = ctx.fonts();
        self.font = Some(Fonts {
            regular: add_font(fonts, 36.0),
            big: add_font(fonts, 72.0),
        });
        if self.update_scale() {
            ctx.style_mut()
                .scale_all_sizes(self.scale);
        }
        // *WidgetChannel::get().timer.write().unwrap() = Some(Local::now().fixed_offset());
    }

    fn render(&mut self, ui: &mut Ui) {
        let channel = WidgetChannel::get();

        // This is simple enough that it can use machine-relative Instants, but the timer getting external input must use DateTime
        let now = Instant::now();
        self.toasts.extend(channel.recv().into_iter()
            .inspect(|text| log::info!("Toast: {}", text))
            .map(|text| (now, text)));
        self.toasts.retain(|(then, _)| then.elapsed() < std::time::Duration::from_secs(7));

        let fonts = self.font.unwrap();
        let regular_font = ui.push_font(fonts.regular);

        // ImGui flags/styles used by practice tool toasts
        let invisible_flags =
            WindowFlags::NO_TITLE_BAR | WindowFlags::NO_RESIZE | WindowFlags::NO_MOVE
            | WindowFlags::NO_SCROLLBAR | WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_INPUTS;
        let invisible_styles = vec![StyleVar::WindowRounding(0.0), StyleVar::FrameBorderSize(0.0), StyleVar::WindowBorderSize(0.0)];

        let style_tokens: Vec<_> = invisible_styles.iter().map(|&v| ui.push_style_var(v)).collect();

        let create_invisible_window = |name: &'static str| {
            ui.window(name)
                .flags(invisible_flags)
                .bg_alpha(0.0)
        };

        let io = ui.io();
        let [dw, dh] = io.display_size;

        let size = [dw * 0.6, dh * 0.21];
        let pos = [dw * 0.18, dh * 1.0];

        create_invisible_window("##toasts")
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

        create_invisible_window("##sidebar")
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

        style_tokens.into_iter().rev().for_each(|v| v.pop());
        regular_font.end();
    }
}

fn get_elapsed_time(time: DateTime<FixedOffset>) -> String {
    let elapsed = Local::now().fixed_offset() - time;
    let secs = 0.max(elapsed.as_seconds_f64() as i32);
    format!("{}:{:02}", secs / 60, secs % 60)
}
