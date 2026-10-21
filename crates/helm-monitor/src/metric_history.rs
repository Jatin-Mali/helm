//! Shared metric history model used by daemon, TUI, and web console.

use serde::{Deserialize, Serialize};

use crate::{
    ring_buffer::RingBuffer,
    snapshot::{PressureStall, SystemSnapshot},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricHistory {
    pub cpu: RingBuffer<f64>,
    pub memory: RingBuffer<f64>,
    pub disk: RingBuffer<f64>,
    pub network: RingBuffer<f64>,
    pub load: RingBuffer<f64>,
    pub swap: RingBuffer<f64>,
    pub io_pressure: RingBuffer<f64>,
}

impl MetricHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            cpu: RingBuffer::new(capacity),
            memory: RingBuffer::new(capacity),
            disk: RingBuffer::new(capacity),
            network: RingBuffer::new(capacity),
            load: RingBuffer::new(capacity),
            swap: RingBuffer::new(capacity),
            io_pressure: RingBuffer::new(capacity),
        }
    }

    pub fn push_snapshot(&mut self, snapshot: &SystemSnapshot) {
        let load = &snapshot.domains.load;
        let cpu = if load.cpu_logical_count > 0 {
            ((load.load_average.one / load.cpu_logical_count as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let memory = if load.memory.total > 0 {
            (load.memory.used as f64 / load.memory.total as f64) * 100.0
        } else {
            0.0
        };
        let disk = snapshot
            .domains
            .disks
            .filesystems
            .iter()
            .filter(|fs| fs.total_bytes > 0)
            .map(|fs| (fs.used_bytes as f64 / fs.total_bytes as f64) * 100.0)
            .fold(0.0, f64::max);
        let network = network_health_percent(snapshot);
        let swap = if load.swap_total > 0 {
            (load.swap_used as f64 / load.swap_total as f64) * 100.0
        } else {
            0.0
        };

        self.cpu.push(cpu);
        self.memory.push(memory);
        self.disk.push(disk);
        self.network.push(network);
        self.load.push(load.load_average.one);
        self.swap.push(swap);
        self.io_pressure
            .push(pressure_value(load.io_pressure.as_ref()));
    }

    pub fn cpu_values(&self) -> Vec<f64> {
        self.cpu.iter().copied().collect()
    }

    pub fn memory_values(&self) -> Vec<f64> {
        self.memory.iter().copied().collect()
    }

    pub fn disk_values(&self) -> Vec<f64> {
        self.disk.iter().copied().collect()
    }

    pub fn network_values(&self) -> Vec<f64> {
        self.network.iter().copied().collect()
    }

    pub fn load_values(&self) -> Vec<f64> {
        self.load.iter().copied().collect()
    }

    pub fn swap_values(&self) -> Vec<f64> {
        self.swap.iter().copied().collect()
    }

    pub fn io_pressure_values(&self) -> Vec<f64> {
        self.io_pressure.iter().copied().collect()
    }
}

impl Default for MetricHistory {
    fn default() -> Self {
        Self::new(60)
    }
}

fn pressure_value(pressure: Option<&PressureStall>) -> f64 {
    pressure
        .and_then(|psi| psi.avg10.or(psi.avg60).or(psi.avg300))
        .unwrap_or(0.0)
}

fn network_health_percent(snapshot: &SystemSnapshot) -> f64 {
    let total = snapshot.domains.network.interfaces.len();
    if total == 0 {
        return 0.0;
    }
    let up = snapshot
        .domains
        .network
        .interfaces
        .iter()
        .filter(|iface| iface.state.eq_ignore_ascii_case("up"))
        .count();
    (up as f64 / total as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::MetricHistory;
    use crate::snapshot::*;

    #[test]
    fn push_snapshot_tracks_expected_series() {
        let mut history = MetricHistory::new(2);
        history.push_snapshot(&snapshot_with_usage(40.0, 50.0, 70.0, 25.0));
        history.push_snapshot(&snapshot_with_usage(60.0, 70.0, 90.0, 40.0));
        history.push_snapshot(&snapshot_with_usage(80.0, 80.0, 95.0, 55.0));

        assert_eq!(history.cpu_values(), vec![60.0, 80.0]);
        assert_eq!(history.memory_values(), vec![70.0, 80.0]);
        assert_eq!(history.disk_values(), vec![90.0, 95.0]);
        let swap = history.swap_values();
        assert!((swap[0] - 40.0).abs() < f64::EPSILON);
        assert!((swap[1] - 55.0).abs() < 0.000_001);
    }

    fn snapshot_with_usage(cpu: f64, memory: f64, disk: f64, swap: f64) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.load.cpu_logical_count = 4;
        domains.load.load_average.one = (cpu / 100.0) * 4.0;
        domains.load.memory.total = 100;
        domains.load.memory.used = memory as u64;
        domains.load.swap_total = 100;
        domains.load.swap_used = swap as u64;
        domains.load.io_pressure = Some(PressureStall {
            avg10: Some(3.5),
            avg60: None,
            avg300: None,
        });
        domains.disks.filesystems.push(FilesystemEntry {
            device: "/dev/sda1".into(),
            mount_point: "/".into(),
            fs_type: "ext4".into(),
            total_bytes: 100,
            used_bytes: disk as u64,
            available_bytes: 100u64.saturating_sub(disk as u64),
        });
        domains.network.interfaces.push(InterfaceEntry {
            name: "eth0".into(),
            state: "up".into(),
            addresses: vec!["192.168.1.10".into()],
        });
        SystemSnapshot {
            id: "test".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains,
            collector_errors: Vec::new(),
            redaction_version: "test".into(),
        }
    }
    // ── Property tests ──────────────────────────────────────────────────────

    #[cfg(test)]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Property 1: CPU values are always clamped to [0, 100].
            #[test]
            fn property_cpu_values_clamped(
                load_avg in 0.0f64..100.0,
                cpu_count in 1u32..16,
            ) {
                let mut history = MetricHistory::new(10);
                let mut snap = snapshot_with_usage(0.0, 50.0, 50.0, 0.0);
                snap.domains.load.cpu_logical_count = cpu_count;
                snap.domains.load.load_average.one = load_avg;
                history.push_snapshot(&snap);
                for v in history.cpu_values() {
                    prop_assert!((0.0..=100.0).contains(&v), "cpu out of range: {v}");
                }
            }

            /// Property 2: MetricHistory respects capacity — values count never exceeds it.
            #[test]
            fn property_history_bounded_by_capacity(
                capacity in 1usize..32,
                pushes in 1usize..64,
            ) {
                let mut history = MetricHistory::new(capacity);
                for _ in 0..pushes {
                    history.push_snapshot(&snapshot_with_usage(50.0, 50.0, 50.0, 10.0));
                }
                prop_assert!(history.cpu_values().len() <= capacity);
                prop_assert!(history.memory_values().len() <= capacity);
                prop_assert!(history.disk_values().len() <= capacity);
            }

            /// Property 3: Memory percent is always in [0, 100].
            #[test]
            fn property_memory_values_clamped(used in 0u64..1000, total in 1u64..1000) {
                let used = used.min(total);
                let mut snap = snapshot_with_usage(0.0, 0.0, 0.0, 0.0);
                snap.domains.load.memory.total = total;
                snap.domains.load.memory.used = used;
                let mut history = MetricHistory::new(10);
                history.push_snapshot(&snap);
                for v in history.memory_values() {
                    prop_assert!((0.0..=100.0).contains(&v), "memory out of range: {v}");
                }
            }
        }
    }
}
