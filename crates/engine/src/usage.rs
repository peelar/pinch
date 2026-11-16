//! Whole-computer usage: every process grouped into the app it belongs to,
//! and the kernel's memory pressure. Lets Pinch answer "what is slowing my
//! computer down?" beyond the dev servers it manages.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use pinch_proto::{AppUsage, MemoryPressure};
use sysinfo::System;

use crate::engine::app_bundle_path;

/// How many of the heaviest apps to keep, by CPU and by memory each.
const LIMIT: usize = 12;

/// One process, reduced to what grouping needs.
struct Sample<'a> {
    pid: u32,
    name: Cow<'a, str>,
    exe: Option<Cow<'a, str>>,
    cpu_percent: f32,
    memory_bytes: u64,
}

pub fn top_apps(sys: &System, cpu_advanced: &HashSet<u32>) -> Vec<AppUsage> {
    let own_bundle = std::env::current_exe()
        .ok()
        .and_then(|exe| app_bundle_path(&exe.to_string_lossy()));
    let samples = sys.processes().iter().map(|(pid, process)| {
        let pid = pid.as_u32();
        Sample {
            pid,
            name: process.name().to_string_lossy(),
            exe: process.exe().map(|e| e.to_string_lossy()),
            // Only trust a reading when the process actually used CPU time
            // since the last tick, as the per-server metrics do.
            cpu_percent: if cpu_advanced.contains(&pid) {
                process.cpu_usage()
            } else {
                0.0
            },
            memory_bytes: footprint(pid).unwrap_or_else(|| process.memory()),
        }
    });
    group(samples, own_bundle.as_deref())
}

/// Group processes by outermost app bundle, or by name when they have none,
/// and keep the heaviest groups by CPU and by memory.
fn group<'a>(samples: impl Iterator<Item = Sample<'a>>, own_bundle: Option<&str>) -> Vec<AppUsage> {
    let mut groups: HashMap<String, AppUsage> = HashMap::new();
    for sample in samples {
        let exe = sample.exe.as_deref();
        let bundle = exe.and_then(app_bundle_path);
        let key = bundle.clone().unwrap_or_else(|| sample.name.to_string());
        let name = bundle
            .as_deref()
            .and_then(|b| Path::new(b).file_stem())
            .map_or_else(
                || sample.name.to_string(),
                |s| s.to_string_lossy().into_owned(),
            );
        let app = groups.entry(key).or_insert_with(|| AppUsage {
            name,
            app_bundle: bundle.clone(),
            quit_pid: None,
            process_count: 0,
            cpu_percent: 0.0,
            memory_bytes: 0,
        });
        app.process_count += 1;
        app.cpu_percent += sample.cpu_percent;
        app.memory_bytes += sample.memory_bytes;
        // The main executable lives directly in the outer bundle's
        // Contents/MacOS; helpers live in nested bundles. Only installed
        // apps qualify: system apps and runtimes such as the `Python.app`
        // inside Python3.framework are not something to quit from here.
        if let (Some(bundle), Some(exe)) = (&bundle, exe)
            && Path::new(exe).parent() == Some(&Path::new(bundle).join("Contents/MacOS"))
            && pinch_proto::is_installed_app(bundle)
            && Some(bundle.as_str()) != own_bundle
        {
            app.quit_pid = Some(app.quit_pid.map_or(sample.pid, |pid| pid.min(sample.pid)));
        }
    }
    let mut all: Vec<AppUsage> = groups.into_values().collect();
    let mut keep: HashSet<String> = HashSet::new();
    let key = |a: &AppUsage| a.app_bundle.clone().unwrap_or_else(|| a.name.clone());
    all.sort_by_key(|a| std::cmp::Reverse(a.memory_bytes));
    keep.extend(all.iter().take(LIMIT).map(key));
    all.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    keep.extend(
        all.iter()
            .take(LIMIT)
            .filter(|a| a.cpu_percent > 0.0)
            .map(key),
    );
    all.retain(|a| keep.contains(&key(a)));
    all
}

/// Memory a process is charged for, as Activity Monitor's Memory column
/// reports it. Resident size double-counts pages shared between an app's
/// helpers. `None` for processes we may not inspect (other users, root).
#[cfg(target_os = "macos")]
fn footprint(pid: u32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: the buffer is a rusage_info_v2, matching the requested flavor.
    let result = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast(),
        )
    };
    // SAFETY: a zero return means the kernel filled the buffer.
    (result == 0).then(|| unsafe { info.assume_init() }.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
fn footprint(_pid: u32) -> Option<u64> {
    None
}

/// macOS's own judgement of memory: high "used" alone is normal, because
/// the system keeps memory busy with caches and compression.
#[cfg(target_os = "macos")]
pub fn memory_pressure() -> Option<MemoryPressure> {
    let mut level: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated and the output buffer is a c_int.
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr(),
            (&raw mut level).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    match (result, level) {
        (0, 1) => Some(MemoryPressure::Normal),
        (0, 2) => Some(MemoryPressure::Warning),
        (0, 4) => Some(MemoryPressure::Critical),
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn memory_pressure() -> Option<MemoryPressure> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<'a>(pid: u32, name: &'a str, exe: Option<&'a str>, cpu: f32, mb: u64) -> Sample<'a> {
        Sample {
            pid,
            name: name.into(),
            exe: exe.map(Cow::from),
            cpu_percent: cpu,
            memory_bytes: mb << 20,
        }
    }

    #[test]
    fn groups_helpers_into_their_app_and_quits_only_the_main_process() {
        const CHROME: &str = "/Applications/Google Chrome.app";
        let helper = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)";
        let pinch = "/Users/me/Applications/Pinch.app";
        let apps = group(
            [
                sample(30, "Google Chrome Helper (Renderer)", Some(helper), 80.0, 900),
                sample(20, "Google Chrome", Some("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"), 5.0, 300),
                sample(31, "Google Chrome Helper (Renderer)", Some(helper), 40.0, 700),
                sample(40, "node", Some("/opt/homebrew/bin/node"), 120.0, 200),
                sample(41, "node", Some("/opt/homebrew/bin/node"), 0.0, 100),
                sample(50, "Finder", Some("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder"), 1.0, 150),
                sample(60, "pinchd", Some("/Users/me/Applications/Pinch.app/Contents/MacOS/pinchd"), 0.5, 20),
                sample(70, "kernel_task", None, 30.0, 0),
                sample(80, "Python", Some("/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python"), 2.0, 30),
            ]
            .into_iter(),
            Some(pinch),
        );
        let find = |name: &str| apps.iter().find(|a| a.name == name).unwrap();

        let chrome = find("Google Chrome");
        assert_eq!(chrome.app_bundle.as_deref(), Some(CHROME));
        assert_eq!(chrome.process_count, 3);
        assert_eq!(chrome.memory_bytes, 1900 << 20);
        assert_eq!(chrome.cpu_percent, 125.0);
        assert_eq!(chrome.quit_pid, Some(20));

        let node = find("node");
        assert_eq!((node.process_count, node.quit_pid), (2, None));
        assert_eq!(find("Finder").quit_pid, None);
        assert_eq!(find("Pinch").quit_pid, None);
        assert_eq!(find("Python").quit_pid, None);
        assert_eq!(find("kernel_task").cpu_percent, 30.0);
    }

    #[test]
    fn keeps_the_heaviest_apps_by_either_measure() {
        let names: Vec<String> = (0..40).map(|i| format!("p{i}")).collect();
        // p0 burns CPU with little memory; p1..p39 have growing memory and no CPU.
        let apps = group(
            names.iter().enumerate().map(|(i, name)| {
                let cpu = if i == 0 { 300.0 } else { 0.0 };
                sample(i as u32, name, None, cpu, i as u64 * 10)
            }),
            None,
        );
        assert_eq!(apps.len(), LIMIT + 1);
        assert!(apps.iter().any(|a| a.name == "p0"));
        assert!(apps.iter().any(|a| a.name == "p39"));
        assert!(!apps.iter().any(|a| a.name == "p1"));
    }
}
