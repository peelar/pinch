//! Optional bundled Sparkle bridge. Source previews do not load an updater.

use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use pinch_proto::UpdateStatus;

struct Api {
    status: unsafe extern "C" fn(*mut libc::c_char, usize) -> u32,
    check: unsafe extern "C" fn(),
    set_automatic: unsafe extern "C" fn(bool),
}

static API: OnceLock<Api> = OnceLock::new();
static STATUS: Mutex<UpdateStatus> = Mutex::new(UpdateStatus {
    enabled: false,
    can_check: false,
    automatic_checks: false,
    available_version: None,
});
static COMMAND: AtomicU8 = AtomicU8::new(0);

extern "C" fn before_install() {
    if let Some(callbacks) = super::CALLBACKS.get() {
        (callbacks.on_update_install)();
    }
}

pub(super) fn start() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(contents) = exe.parent().and_then(|p| p.parent()) else {
        return;
    };
    let path = contents.join("Frameworks/libpinch-updater.dylib");
    if !path.is_file() {
        return;
    }
    let Ok(path) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return;
    };
    unsafe {
        let library = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if library.is_null() {
            eprintln!(
                "pinchd: cannot load updater: {}",
                CStr::from_ptr(libc::dlerror()).to_string_lossy()
            );
            return;
        }
        let start = libc::dlsym(library, c"pinch_updater_start".as_ptr());
        let status = libc::dlsym(library, c"pinch_updater_status".as_ptr());
        let check = libc::dlsym(library, c"pinch_updater_check".as_ptr());
        let set_automatic = libc::dlsym(library, c"pinch_updater_set_automatic".as_ptr());
        if [start, status, check, set_automatic]
            .iter()
            .any(|p| p.is_null())
        {
            eprintln!("pinchd: incomplete updater bridge");
            return;
        }
        let start: unsafe extern "C" fn(extern "C" fn()) -> bool = std::mem::transmute(start);
        if !start(before_install) {
            return;
        }
        let _ = API.set(Api {
            status: std::mem::transmute::<
                *mut libc::c_void,
                unsafe extern "C" fn(*mut libc::c_char, usize) -> u32,
            >(status),
            check: std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn()>(check),
            set_automatic: std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(bool)>(
                set_automatic,
            ),
        });
    }
    poll();
}

pub(super) fn poll() {
    let Some(api) = API.get() else { return };
    unsafe {
        match COMMAND.swap(0, Ordering::SeqCst) {
            1 => (api.check)(),
            2 => (api.set_automatic)(true),
            3 => (api.set_automatic)(false),
            _ => {}
        }
        let mut version = [0; 256];
        let flags = (api.status)(version.as_mut_ptr(), version.len());
        let version = CStr::from_ptr(version.as_ptr())
            .to_string_lossy()
            .into_owned();
        *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = UpdateStatus {
            enabled: flags & 1 != 0,
            can_check: flags & 2 != 0,
            automatic_checks: flags & 4 != 0,
            available_version: (!version.is_empty()).then_some(version),
        };
    }
}

pub fn status() -> UpdateStatus {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn check() -> Result<(), &'static str> {
    let status = status();
    if !status.can_check && status.available_version.is_none() {
        return Err("Updates are unavailable or a check is already in progress");
    }
    COMMAND
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .map(|_| ())
        .map_err(|_| "An updater action is already pending")
}

pub fn set_automatic(enabled: bool) -> Result<(), &'static str> {
    if !status().enabled {
        return Err("Updates are unavailable in this build");
    }
    COMMAND
        .compare_exchange(
            0,
            if enabled { 2 } else { 3 },
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .map(|_| ())
        .map_err(|_| "An updater action is already pending")
}
