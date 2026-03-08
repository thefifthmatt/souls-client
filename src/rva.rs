#![allow(unused)]
mod eldenring;
mod ds1r;
mod ds3;
mod sekiro;

pub use eldenring::*;
pub use ds1r::*;
pub use ds3::*;
pub use sekiro::*;

pub struct RvaBundle {
    pub csemevd_repository_vmt: u32,
    pub csemevd_res_cap_vmt: u32,
    pub tpf_repository_vmt: u32,
    pub tpf_res_cap_vmt: u32,
    pub tpf_file_cap_vmt: u32,
}

pub fn get() -> RvaBundle { RvaBundle{ csemevd_repository_vmt, csemevd_res_cap_vmt, tpf_repository_vmt, tpf_res_cap_vmt, tpf_file_cap_vmt } }
