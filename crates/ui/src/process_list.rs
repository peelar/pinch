use pinch_proto::{Decision, Origin, ProcessInfo};

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
}
