//! RDP-Worker auf Grundlage von FreeRDP 3 (dynamisch gelinkt).

pub mod clip;
pub mod keymap;
#[cfg(not(rj_no_freerdp))]
pub mod session;

#[cfg(not(rj_no_freerdp))]
#[allow(non_camel_case_types, non_upper_case_globals, dead_code, non_snake_case)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/rjrdp.rs"));
}
