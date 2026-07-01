use std::{error::Error, iter, panic, time::Duration, path::{Path, PathBuf}};
use hudhook::inject::Process;

use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW},
    core::{PCWSTR, w},
};

fn main() {
    set_panic_hook();
    match inject() {
        Err(e) => show_error_message_box(&e.to_string()),
        _ => (),
    };
}

fn inject() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from("souls_client.dll");
    if !Path::exists(&path) {
        return Err(format!("{path:?} not found").into());
    }
    let process = Process::by_name("eldenring.exe").map_err(|e| format!("Couldn't detect eldenring.exe\n\n{e:?}"))?;
    process.inject(path.clone()).map_err(|e| format!("Couldn't inject {path:?}\n\n{e:?}"))?;
    Ok(())
}

// Simplified version of logger in main dll. Not really used since Result is used instead
pub fn set_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let mut msg = format!(
            "Error: {}",
            info.payload_as_str().unwrap_or("Failed to inject dll")
        );

        if let Some(location) = info.location() {
            msg += &format!(
                "\n    {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }

        show_error_message_box(&msg);
        std::thread::sleep(Duration::from_millis(1));
    }));
}

fn show_error_message_box(msg: &str) {
    let msg = msg.encode_utf16().chain(iter::once(0)).collect::<Vec<_>>();
    unsafe {
        let _ = MessageBoxW(None, PCWSTR(msg.as_ptr()), w!("Failed to inject dll"), MB_ICONERROR);
    }
}
