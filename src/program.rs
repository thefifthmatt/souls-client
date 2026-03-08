use std::{ffi::{OsString, c_void}, mem, os::windows::ffi::OsStringExt, sync::LazyLock};

use pelite::pe64::{Pe, PeView};
use windows::{
    Win32::{Foundation::HMODULE, System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleA, GetModuleHandleExW, GetModuleHandleW}},
    core::{Error, PCSTR, PCWSTR},
};
use std::path::PathBuf;

// use crate::rva::Rva;

#[derive(Clone, Copy, Debug, Hash)]
pub struct Program(*mut c_void);

// Based on erfps2. TODO: Use fromsoftware-rs instead
impl Program {
    pub fn current() -> Self {
        Self::try_current().expect("GetModuleHandleW failed")
    }

    pub fn try_current() -> Result<Self, Error> {
        static CURRENT: LazyLock<Result<Program, Error>> = LazyLock::new(|| unsafe {
            GetModuleHandleW(PCWSTR::null()).map(Program::from_hmodule)
        });
        CURRENT.clone()
    }

    /// # Safety
    ///
    /// Safe, but using the resulting pointer is incredibly unsafe.
    pub fn derva<T>(self, rva: u32) -> *mut T {
        self.0.wrapping_byte_add(rva as usize).cast()
    }

    /// # Safety
    ///
    /// Incredibly unsafe and may cause spontaneous burst pipes, gas leaks, explosions, etc.
    pub unsafe fn derva_ptr<P>(self, rva: u32) -> P {
        assert!(size_of::<P>() == size_of::<*mut ()>());
        unsafe { mem::transmute_copy::<*mut (), P>(&self.derva::<()>(rva)) }
    }

    fn from_hmodule(HMODULE(base): HMODULE) -> Self {
        Self(base)
    }
}

impl From<Program> for PeView<'static> {
    fn from(value: Program) -> Self {
        unsafe { Self::module(value.0 as _) }
    }
}

unsafe impl Send for Program {}
unsafe impl Sync for Program {}

pub fn get_product_info() -> Option<(String, String)> {
    let module = unsafe {
        PeView::module(GetModuleHandleA(PCSTR(std::ptr::null())).unwrap().0 as *const u8)
    };

    let resources = module.resources().ok()?;
    let info = resources.version_info().ok()?;

    // Extract version info
    let product_version = info.fixed()?.dwProductVersion;
    let version = format!(
        "{}.{}.{}.{}",
        product_version.Major, product_version.Minor, product_version.Patch, product_version.Build,
    );

    // Extract product name
    let language = *info.translation().first()?;
    let mut product_name: Option<String> = None;
    info.strings(language, |k, v| {
        if k == "ProductName" {
            product_name = Some(v.to_string());
        }
    });

    let product = product_name?;
    // let lang_id_base = language.lang_id & 0x03FF;

    Some((product, version))
}

pub fn current_module_path() -> PathBuf {
    static PATH_STRING: LazyLock<Option<String>> = LazyLock::new(|| {
        match current_module_path_string() {
            Ok(str) => Some(str),
            Err(e) => {
                log::error!("Couldn't load module path: {e:?}");
                None
            }
        }
    });
    match &*PATH_STRING {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir().expect("Could not access current path"),
    }
}

fn current_module_path_string() -> Result<String, windows::core::Error> {
    let module_handle = unsafe {
        fn in_module_dummy() {}
        let mut module_handle = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            PCWSTR(in_module_dummy as *const u16),
            &mut module_handle,
        )?;
        module_handle
    };

    // Approx. reasonable max length:
    // https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation
    let mut module_filename = vec![0u16; 32767];

    unsafe {
        let len = GetModuleFileNameW(Some(module_handle), &mut module_filename);

        if len == 0 || len == 32767 {
            return Err(windows::core::Error::from_thread());
        }

        module_filename.truncate(len as usize);
    }

    OsString::from_wide(&module_filename).into_string().map_err(|_| windows::core::Error::empty())
}
