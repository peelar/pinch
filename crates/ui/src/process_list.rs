use std::collections::BTreeMap;

use pinch_proto::{Decision, Origin, ProcessGroup, ProcessInfo};

pub struct ProcessList<'a> {
    pub stale: Vec<(&'a ProcessInfo, &'a Decision)>,
    pub managed: Vec<(usize, &'a ProcessInfo)>,
    pub recognized: Vec<(usize, &'a ProcessInfo)>,
    pub unmanaged: Vec<(usize, &'a ProcessInfo)>,
}

impl<'a> ProcessList<'a> {
    pub fn new(processes: &'a [ProcessInfo], decisions: &'a [Decision]) -> Self {
        let mut list = Self {
            stale: Vec::new(),
            managed: Vec::new(),
            unmanaged: Vec::new(),
            recognized: Vec::new(),
        };
        for (index, process) in processes.iter().enumerate() {
            if !process.status.is_alive() {
                continue;
            }
            if process.origin == Origin::Unmanaged {
                if process.recognized {
                    list.recognized.push((index, process));
                } else {
                    list.unmanaged.push((index, process));
                }
            } else if let Some(decision) = decisions.iter().find(|d| d.process == process.id) {
                list.stale.push((process, decision));
            } else {
                list.managed.push((index, process));
            }
        }
        list.stale.sort_by(|(a, da), (b, db)| {
            ram(b)
                .cmp(&ram(a))
                .then_with(|| da.created_at_ms.cmp(&db.created_at_ms))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        let by_resources = |a: &(usize, &ProcessInfo), b: &(usize, &ProcessInfo)| {
            let a = a.1;
            let b = b.1;
            b.metrics
                .map_or(0.0, |m| m.cpu_percent)
                .total_cmp(&a.metrics.map_or(0.0, |m| m.cpu_percent))
                .then_with(|| ram(b).cmp(&ram(a)))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.0.cmp(&b.id.0))
        };
        list.managed.sort_by(by_resources);
        list.unmanaged.sort_by(by_resources);
        list.recognized.sort_by(by_resources);
        list
    }

    /// Servers Pinch started, stale ones first, by agent session.
    pub fn managed_groups(
        &self,
    ) -> BTreeMap<ProcessGroup, Vec<(&'a ProcessInfo, Option<&'a Decision>)>> {
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for &(p, d) in &self.stale {
            groups.entry(p.group()).or_default().push((p, Some(d)));
        }
        for &(_, p) in &self.managed {
            groups.entry(p.group()).or_default().push((p, None));
        }
        groups
    }

    /// Servers started outside Pinch that belong in the server list: unknown
    /// ones, and ones recognized until they exit. Servers an app runs itself
    /// are listed under that app instead.
    pub fn discovered(&self) -> Vec<&'a ProcessInfo> {
        self.unmanaged
            .iter()
            .map(|(_, p)| *p)
            .chain(
                self.recognized
                    .iter()
                    .map(|(_, p)| *p)
                    .filter(|p| p.app.is_none()),
            )
            .collect()
    }

    pub fn stale_ram(&self) -> u64 {
        self.stale.iter().map(|(process, _)| ram(process)).sum()
    }
}

fn ram(process: &ProcessInfo) -> u64 {
    process.metrics.map_or(0, |m| m.rss_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinch_proto::{Metrics, ProcessId, ProcessKind, Status};

    fn process(id: &str, origin: Origin, status: Status, cpu: f32, ram: u64) -> ProcessInfo {
        ProcessInfo {
            agent: None,
            id: ProcessId(id.into()),
            name: id.into(),
            kind: ProcessKind::Node,
            origin,
            recognized: false,
            status,
            command: vec![],
            cwd: format!("/work/{id}"),
            executable: None,
            app_bundle: None,
            launcher_app: None,
            app: None,
            repo: None,
            branch: None,
            owner: None,
            pid: None,
            ports: vec![],
            started_at_ms: 0,
            last_output_ms: None,
            log_path: None,
            metrics: Some(Metrics {
                cpu_percent: cpu,
                rss_bytes: ram,
            }),
        }
    }

    fn decision(id: &str) -> Decision {
        Decision {
            id: format!("decision-{id}"),
            process: ProcessId(id.into()),
            process_name: id.into(),
            reason: String::new(),
            created_at_ms: 1,
            quiet_since_ms: Some(0),
        }
    }

    #[test]
    fn cleanup_candidates_appear_once_and_largest_memory_comes_first() {
        let processes = vec![
            process("small", Origin::Managed, Status::Stale, 0.0, 20),
            process("large", Origin::Managed, Status::Stale, 0.0, 200),
            process("busy", Origin::Managed, Status::Running, 60.0, 10),
            process("quiet", Origin::Managed, Status::Running, 0.0, 80),
            process("external", Origin::Unmanaged, Status::Running, 90.0, 100),
            process(
                "exited",
                Origin::Managed,
                Status::Exited { code: None },
                0.0,
                1000,
            ),
        ];
        let decisions = vec![
            decision("small"),
            decision("large"),
            decision("exited"),
            decision("external"),
            decision("missing"),
        ];
        let list = ProcessList::new(&processes, &decisions);
        assert_eq!(
            list.stale
                .iter()
                .map(|(p, _)| p.name.as_str())
                .collect::<Vec<_>>(),
            ["large", "small"]
        );
        assert_eq!(list.stale_ram(), 220);
        assert_eq!(
            list.managed
                .iter()
                .map(|(_, p)| p.name.as_str())
                .collect::<Vec<_>>(),
            ["busy", "quiet"]
        );
        assert_eq!(list.unmanaged.len(), 1);
        assert_eq!(list.unmanaged[0].1.name, "external");
    }

    #[test]
    fn recognized_processes_stay_visible_without_becoming_stale() {
        let mut expected = process("expected", Origin::Unmanaged, Status::Running, 0.0, 200);
        expected.recognized = true;
        let processes = vec![
            expected,
            process("unknown", Origin::Unmanaged, Status::Running, 0.0, 20),
        ];
        let decisions = vec![decision("expected")];
        let list = ProcessList::new(&processes, &decisions);
        assert_eq!(list.recognized.len(), 1);
        assert_eq!(list.unmanaged.len(), 1);
        assert!(list.stale.is_empty());
        assert_eq!(list.recognized[0].1.metrics.unwrap().rss_bytes, 200);
    }

    #[test]
    fn resolved_decision_returns_process_to_managed_list() {
        let processes = vec![process("server", Origin::Managed, Status::Running, 0.0, 20)];
        let list = ProcessList::new(&processes, &[]);
        assert!(list.stale.is_empty());
        assert_eq!(list.managed.len(), 1);
    }

    #[test]
    fn exited_history_does_not_affect_window_height_or_keep_empty_groups() {
        use crate::view::{Snapshot, height_for};
        let mut snapshot = Snapshot::default();
        let empty_height = height_for(&snapshot);
        snapshot.processes.push(process(
            "old",
            Origin::Managed,
            Status::Exited { code: None },
            0.0,
            100,
        ));
        snapshot.decisions.push(decision("old"));
        assert_eq!(height_for(&snapshot), empty_height);
        snapshot
            .processes
            .push(process("live", Origin::Managed, Status::Running, 0.0, 20));
        let with_history = height_for(&snapshot);
        snapshot.processes.remove(0);
        snapshot.decisions.clear();
        assert_eq!(height_for(&snapshot), with_history);
    }

    #[test]
    fn heavy_apps_keep_agent_browsers_and_big_apps_with_instances_together() {
        use crate::view::heavy_apps;
        use pinch_proto::{AppUsage, Instance};
        let app = |name: &str, gib: f32, cpu: f32, instance: Option<Instance>| AppUsage {
            name: name.into(),
            app_bundle: Some(format!("/Applications/{name}.app")),
            close: None,
            instance,
            process_count: 1,
            cpu_percent: cpu,
            memory_bytes: (gib * (1u64 << 30) as f32) as u64,
        };
        let agent = Instance::Automated { launched_by: None };
        let apps = vec![
            app("Slack", 0.5, 1.0, None),
            app("Chrome", 0.2, 0.0, Some(agent.clone())),
            app("Docker", 2.0, 0.0, None),
            app("Chrome", 3.0, 0.0, Some(Instance::Yours)),
            app("Xcode", 0.3, 80.0, None),
        ];
        let heavy: Vec<(&str, u64)> = heavy_apps(&apps)
            .iter()
            .map(|a| (a.name.as_str(), a.memory_bytes >> 20))
            .collect();
        assert_eq!(
            heavy,
            [
                ("Chrome", 3072),
                ("Chrome", 204),
                ("Docker", 2048),
                ("Xcode", 307)
            ]
        );
    }
}
