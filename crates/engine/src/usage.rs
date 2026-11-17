//! Whole-computer usage: every process grouped into the app it belongs to,
//! and the kernel's memory pressure. Lets Pinch answer "what is slowing my
//! computer down?" beyond the dev servers it manages.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use pinch_proto::{AppUsage, Close, Instance, MemoryPressure};
use sysinfo::System;

use crate::engine::app_bundle_path;

/// How many of the heaviest apps to keep, by CPU and by memory each.
const LIMIT: usize = 12;

/// Command-line flags that browser automation (Playwright, Puppeteer,
/// Selenium, agent browser tools) passes and people normally don't.
const AUTOMATION_FLAGS: &[&str] = &[
    "--remote-debugging-port",
    "--remote-debugging-pipe",
    "--enable-automation",
    "--headless",
];

/// One process, reduced to what grouping needs.
struct Sample<'a> {
    pid: u32,
    parent: Option<u32>,
    started_at_s: u64,
    name: Cow<'a, str>,
    exe: Option<Cow<'a, str>>,
    automated: bool,
    cpu_percent: f32,
    memory_bytes: u64,
}

/// `skip` holds processes the list already shows as servers.
pub fn top_apps(sys: &System, cpu_advanced: &HashSet<u32>, skip: &HashSet<u32>) -> Vec<AppUsage> {
    let own_bundle = std::env::current_exe()
        .ok()
        .and_then(|exe| app_bundle_path(&exe.to_string_lossy()));
    let samples = sys
        .processes()
        .iter()
        .filter(|(pid, _)| !skip.contains(&pid.as_u32()))
        .map(|(pid, process)| {
            let pid = pid.as_u32();
            Sample {
                pid,
                parent: process.parent().map(|p| p.as_u32()),
                started_at_s: process.start_time(),
                name: process.name().to_string_lossy(),
                exe: process.exe().map(|e| e.to_string_lossy()),
                automated: process.cmd().iter().any(|arg| {
                    let arg = arg.to_string_lossy();
                    AUTOMATION_FLAGS
                        .iter()
                        .any(|flag| arg == *flag || arg.starts_with(&format!("{flag}=")))
                }),
                // Only trust a reading when the process actually used CPU time
                // since the last tick, as the per-server metrics do.
                cpu_percent: if cpu_advanced.contains(&pid) {
                    process.cpu_usage()
                } else {
                    0.0
                },
                memory_bytes: footprint(pid).unwrap_or_else(|| process.memory()),
            }
        })
        .collect::<Vec<_>>();
    group(&samples, own_bundle.as_deref())
}

/// Group processes by outermost app bundle, or by name when they have none,
/// and keep the heaviest groups by CPU and by memory. An app whose main
/// process runs under automation (an agent's browser) is split into one
/// group per automated instance plus the user's own.
fn group(samples: &[Sample<'_>], own_bundle: Option<&str>) -> Vec<AppUsage> {
    let by_pid: HashMap<u32, &Sample> = samples.iter().map(|s| (s.pid, s)).collect();
    let bundle_of = |s: &Sample| s.exe.as_deref().and_then(app_bundle_path);
    // The main executable lives directly in the outer bundle's Contents/MacOS;
    // helpers live in nested bundles.
    let is_main = |s: &Sample, bundle: &str| {
        s.exe.as_deref().and_then(|e| Path::new(e).parent())
            == Some(&Path::new(bundle).join("Contents/MacOS"))
    };
    // The nearest automated main process of the same bundle, walking up
    // from the process itself.
    let automated_root = |s: &Sample, bundle: &str| -> Option<u32> {
        let mut current = Some(s);
        for _ in 0..64 {
            let process = current?;
            if process.automated
                && is_main(process, bundle)
                && bundle_of(process).as_deref() == Some(bundle)
            {
                return Some(process.pid);
            }
            current = process
                .parent
                .filter(|p| *p != process.pid)
                .and_then(|p| by_pid.get(&p).copied());
        }
        None
    };

    let mut groups: HashMap<(String, Option<u32>), AppUsage> = HashMap::new();
    let mut automated_bundles: HashSet<String> = HashSet::new();
    for sample in samples {
        let bundle = bundle_of(sample);
        let root = bundle.as_deref().and_then(|b| automated_root(sample, b));
        if let (Some(b), Some(_)) = (&bundle, root) {
            automated_bundles.insert(b.clone());
        }
        let key = (
            bundle.clone().unwrap_or_else(|| sample.name.to_string()),
            root,
        );
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
            close: None,
            instance: None,
            process_count: 0,
            cpu_percent: 0.0,
            memory_bytes: 0,
        });
        app.process_count += 1;
        app.cpu_percent += sample.cpu_percent;
        app.memory_bytes += sample.memory_bytes;
        let Some(bundle) = &bundle else { continue };
        if Some(bundle.as_str()) == own_bundle {
            continue;
        }
        if let Some(root) = root {
            if root == sample.pid {
                app.close = Some(Close::Terminate {
                    pid: sample.pid,
                    started_at_s: sample.started_at_s,
                });
                app.instance = Some(Instance::Automated {
                    launched_by: launcher(sample, bundle, &by_pid),
                });
            }
        } else if is_main(sample, bundle)
            // System apps and runtimes such as the `Python.app` inside
            // Python3.framework are not something to quit from here.
            && pinch_proto::is_installed_app(bundle)
        {
            let pid = match app.close {
                Some(Close::Quit { pid }) => pid.min(sample.pid),
                _ => sample.pid,
            };
            app.close = Some(Close::Quit { pid });
        }
    }
    for ((bundle, root), app) in &mut groups {
        if root.is_none() && automated_bundles.contains(bundle) {
            app.instance = Some(Instance::Yours);
        }
    }

    let mut all: Vec<AppUsage> = groups.into_values().collect();
    let mut keep: HashSet<(String, Option<u32>)> = HashSet::new();
    let key = |a: &AppUsage| {
        (
            a.app_bundle.clone().unwrap_or_else(|| a.name.clone()),
            match a.close {
                Some(close @ Close::Terminate { .. }) => Some(close.pid()),
                _ => None,
            },
        )
    };
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

/// What started an automated instance: the nearest ancestor app, or else the
/// nearest ancestor's process name (an agent CLI usually runs as `node`).
fn launcher(sample: &Sample, bundle: &str, by_pid: &HashMap<u32, &Sample>) -> Option<String> {
    let mut nearest_name = None;
    let mut current = sample.parent.and_then(|p| by_pid.get(&p).copied());
    for _ in 0..64 {
        let process = current?;
        if process.pid <= 1 {
            break;
        }
        let app = process.exe.as_deref().and_then(app_bundle_path);
        match app {
            Some(app) if app != bundle => {
                return Some(pinch_proto::app_name(&app).to_string());
            }
            Some(_) => {}
            None => {
                nearest_name.get_or_insert_with(|| process.name.to_string());
            }
        }
        current = process
            .parent
            .filter(|p| *p != process.pid)
            .and_then(|p| by_pid.get(&p).copied());
    }
    nearest_name
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
            parent: Some(1),
            started_at_s: 1000 + u64::from(pid),
            name: name.into(),
            exe: exe.map(Cow::from),
            automated: false,
            cpu_percent: cpu,
            memory_bytes: mb << 20,
        }
    }

    fn child<'a>(mut s: Sample<'a>, parent: u32) -> Sample<'a> {
        s.parent = Some(parent);
        s
    }

    const CHROME: &str = "/Applications/Google Chrome.app";
    const CHROME_MAIN: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    const RENDERER: &str = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)";

    #[test]
    fn groups_helpers_into_their_app_and_quits_only_the_main_process() {
        let pinch = "/Users/me/Applications/Pinch.app";
        let apps = group(
            &[
                child(
                    sample(
                        30,
                        "Google Chrome Helper (Renderer)",
                        Some(RENDERER),
                        80.0,
                        900,
                    ),
                    20,
                ),
                sample(20, "Google Chrome", Some(CHROME_MAIN), 5.0, 300),
                child(
                    sample(
                        31,
                        "Google Chrome Helper (Renderer)",
                        Some(RENDERER),
                        40.0,
                        700,
                    ),
                    20,
                ),
                sample(40, "node", Some("/opt/homebrew/bin/node"), 120.0, 200),
                sample(41, "node", Some("/opt/homebrew/bin/node"), 0.0, 100),
                sample(
                    50,
                    "Finder",
                    Some("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder"),
                    1.0,
                    150,
                ),
                sample(
                    60,
                    "pinchd",
                    Some("/Users/me/Applications/Pinch.app/Contents/MacOS/pinchd"),
                    0.5,
                    20,
                ),
                sample(70, "kernel_task", None, 30.0, 0),
                sample(
                    80,
                    "Python",
                    Some(
                        "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python",
                    ),
                    2.0,
                    30,
                ),
            ],
            Some(pinch),
        );
        let find = |name: &str| apps.iter().find(|a| a.name == name).unwrap();

        let chrome = find("Google Chrome");
        assert_eq!(chrome.app_bundle.as_deref(), Some(CHROME));
        assert_eq!(chrome.process_count, 3);
        assert_eq!(chrome.memory_bytes, 1900 << 20);
        assert_eq!(chrome.cpu_percent, 125.0);
        assert_eq!(chrome.close, Some(Close::Quit { pid: 20 }));
        assert_eq!(chrome.instance, None);

        let node = find("node");
        assert_eq!((node.process_count, node.close), (2, None));
        assert_eq!(find("Finder").close, None);
        assert_eq!(find("Pinch").close, None);
        assert_eq!(find("Python").close, None);
        assert_eq!(find("kernel_task").cpu_percent, 30.0);
    }

    #[test]
    fn splits_an_agents_browser_from_the_users_own() {
        let mut agent_chrome = child(
            sample(200, "Google Chrome", Some(CHROME_MAIN), 10.0, 400),
            150,
        );
        agent_chrome.automated = true;
        let apps = group(
            &[
                sample(20, "Google Chrome", Some(CHROME_MAIN), 5.0, 300),
                child(
                    sample(
                        21,
                        "Google Chrome Helper (Renderer)",
                        Some(RENDERER),
                        1.0,
                        700,
                    ),
                    20,
                ),
                sample(
                    140,
                    "Cursor",
                    Some("/Applications/Cursor.app/Contents/MacOS/Cursor"),
                    1.0,
                    50,
                ),
                child(
                    sample(150, "node", Some("/opt/homebrew/bin/node"), 1.0, 50),
                    140,
                ),
                agent_chrome,
                // Helpers are children of the automated main, not of node.
                child(
                    sample(
                        201,
                        "Google Chrome Helper (Renderer)",
                        Some(RENDERER),
                        60.0,
                        900,
                    ),
                    200,
                ),
                child(
                    sample(
                        202,
                        "Google Chrome Helper (Renderer)",
                        Some(RENDERER),
                        0.0,
                        100,
                    ),
                    201,
                ),
            ],
            None,
        );
        let chromes: Vec<&AppUsage> = apps.iter().filter(|a| a.name == "Google Chrome").collect();
        assert_eq!(chromes.len(), 2);
        let yours = chromes
            .iter()
            .find(|a| a.instance == Some(Instance::Yours))
            .unwrap();
        assert_eq!((yours.process_count, yours.memory_bytes), (2, 1000 << 20));
        assert_eq!(yours.close, Some(Close::Quit { pid: 20 }));
        let agent = chromes
            .iter()
            .find(|a| a.instance != Some(Instance::Yours))
            .unwrap();
        assert_eq!((agent.process_count, agent.memory_bytes), (3, 1400 << 20));
        assert_eq!(agent.cpu_percent, 70.0);
        assert_eq!(
            agent.close,
            Some(Close::Terminate {
                pid: 200,
                started_at_s: 1200
            })
        );
        assert_eq!(
            agent.instance,
            Some(Instance::Automated {
                launched_by: Some("Cursor".into())
            })
        );
        let node = apps.iter().find(|a| a.name == "node").unwrap();
        assert_eq!(node.process_count, 1);
    }

    #[test]
    fn keeps_the_heaviest_apps_by_either_measure() {
        let names: Vec<String> = (0..40).map(|i| format!("p{i}")).collect();
        // p0 burns CPU with little memory; p1..p39 have growing memory and no CPU.
        let samples: Vec<Sample> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let cpu = if i == 0 { 300.0 } else { 0.0 };
                sample(i as u32 + 2, name, None, cpu, i as u64 * 10)
            })
            .collect();
        let apps = group(&samples, None);
        assert_eq!(apps.len(), LIMIT + 1);
        assert!(apps.iter().any(|a| a.name == "p0"));
        assert!(apps.iter().any(|a| a.name == "p39"));
        assert!(!apps.iter().any(|a| a.name == "p1"));
    }
}
