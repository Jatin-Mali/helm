//! Typed system snapshot model — the canonical read-only inventory of a Linux host.
#![allow(clippy::derivable_impls)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Opaque snapshot identifier.
pub type SnapshotId = String;

/// Duration for bounded work.
pub type Seconds = u64;

// ── Profile ────────────────────────────────────────────────────────────────

/// Controls collector depth and timeout budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonitorProfile {
    /// Quick scan: essential collectors, short timeouts.
    Quick,
    /// Standard scan: all collectors, moderate timeouts.
    Standard,
    /// Deep scan: all collectors, extended timeouts, extra probes.
    Deep,
}

impl MonitorProfile {
    /// Per-collector timeout in seconds.
    pub fn per_collector_timeout(self) -> Seconds {
        match self {
            Self::Quick => 5,
            Self::Standard => 10,
            Self::Deep => 30,
        }
    }

    /// Whether to run heavy probes (SMART, docker inspect deep).
    pub fn deep_probes(self) -> bool {
        matches!(self, Self::Deep)
    }
}

impl std::str::FromStr for MonitorProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "quick" => Ok(Self::Quick),
            "standard" => Ok(Self::Standard),
            "deep" => Ok(Self::Deep),
            other => Err(format!(
                "unknown profile '{}'; expected quick|standard|deep",
                other
            )),
        }
    }
}

// ── Host identity ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostIdentity {
    /// `uname -n` hostname.
    pub hostname: String,
    /// `uname -s` kernel name.
    pub kernel_name: String,
    /// `uname -r` kernel release.
    pub kernel_release: String,
    /// `uname -m` machine architecture.
    pub machine: String,
    /// OS pretty name from /etc/os-release.
    pub os_pretty_name: Option<String>,
    /// OS ID from /etc/os-release.
    pub os_id: Option<String>,
    /// OS version from /etc/os-release.
    pub os_version_id: Option<String>,
    /// System uptime in seconds (from /proc/uptime).
    pub uptime_seconds: u64,
}

// ── Load / CPU / Memory / PSI ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSnapshot {
    /// 1, 5, 15-minute load averages.
    pub load_average: LoadAverage,
    /// CPU count (logical).
    pub cpu_logical_count: u32,
    /// /proc/pressure/cpu some avg10/avg60/avg300.
    pub cpu_pressure: Option<PressureStall>,
    /// /proc/pressure/memory some avg10/avg60/avg300.
    pub memory_pressure: Option<PressureStall>,
    /// /proc/pressure/io some avg10/avg60/avg300.
    pub io_pressure: Option<PressureStall>,
    /// `free -b` — total, used, available.
    pub memory: MemoryInfo,
    /// Total swap in bytes.
    pub swap_total: u64,
    /// Used swap in bytes.
    pub swap_used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PressureStall {
    pub avg10: Option<f64>,
    pub avg60: Option<f64>,
    pub avg300: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInfo {
    /// Total physical memory in bytes.
    pub total: u64,
    /// Used memory in bytes.
    pub used: u64,
    /// Available memory in bytes (MemAvailable from /proc/meminfo).
    pub available: Option<u64>,
}

// ── Disks ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskSnapshot {
    /// Filesystem entries from df -B1.
    pub filesystems: Vec<FilesystemEntry>,
    /// Mount entries from findmnt.
    pub mounts: Vec<MountEntry>,
    /// Block device info from lsblk.
    pub block_devices: Vec<BlockDevice>,
    /// Whether smartctl binary was available.
    pub smart_available: bool,
    /// SMART health for at least one device (profile ≥ Standard).
    pub smart_devices: Vec<SmartDevice>,
    /// Inode usage per filesystem (from df -i).
    pub inodes: Vec<InodeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemEntry {
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountEntry {
    pub source: String,
    pub target: String,
    pub fs_type: String,
    pub options: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockDevice {
    pub name: String,
    pub size: Option<u64>,
    pub ro: bool,
    pub mount_points: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartDevice {
    pub device: String,
    pub model: Option<String>,
    pub health: Option<String>,
    pub temperature_celsius: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InodeEntry {
    pub device: String,
    pub mount_point: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

// ── Services ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    /// Loaded systemd units (list-units --all).
    pub units: Vec<SystemdUnit>,
    /// Failed units only.
    pub failed_units: Vec<FailedUnit>,
    /// Systemd timers (list-timers --all).
    pub timers: Vec<SystemdTimer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemdUnit {
    pub name: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedUnit {
    pub name: String,
    pub description: String,
    pub loaded: String,
    pub active: String,
    pub sub: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemdTimer {
    pub name: String,
    pub next_trigger: String,
    pub last_trigger: String,
    pub passed: String,
    pub unit: String,
    pub activates: String,
}

// ── Containers ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSnapshot {
    /// Container runtime detected: Docker, Podman, or None.
    pub runtime: Option<ContainerRuntime>,
    /// Running and stopped containers.
    pub containers: Vec<ContainerInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerRuntime {
    Docker,
    Podman,
}

impl std::fmt::Display for ContainerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Docker => write!(f, "Docker"),
            Self::Podman => write!(f, "Podman"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub ports: Vec<String>,
    pub mounts: Vec<String>,
    pub restart_count: Option<u32>,
    pub health: Option<String>,
}

// ── Ports ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSnapshot {
    /// Listening sockets from ss -tulpn.
    pub listeners: Vec<ListenerEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerEntry {
    pub protocol: String,
    pub local_address: String,
    pub local_port: u16,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
}

// ── Logs ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogSnapshot {
    /// Journal summary — error count in the last window.
    pub journal_errors_last_hour: u64,
    /// Recent kernel errors (bounded).
    pub kernel_errors: Vec<String>,
    /// Recent auth failures (truncated at 10).
    pub auth_failures: Vec<String>,
    /// Rate-per-minute spike detection.
    pub error_rate_per_minute: Option<f64>,
}

// ── Backups ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSnapshot {
    /// Backup tools detected on the system.
    pub tools_detected: Vec<BackupTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupTool {
    pub name: String,
    /// Binary path if found.
    pub binary_path: Option<String>,
    /// Config directory if found (e.g. /etc/borgmatic).
    pub config_path: Option<String>,
    /// Repo or snapshot root if found.
    pub repo_path: Option<String>,
    /// Whether restore-test evidence was found (e.g. restic check cache).
    pub restore_test_evidence: Option<String>,
}

// ── Packages ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSnapshot {
    /// Detected package manager.
    pub package_manager: Option<String>,
    /// Total upgradable packages.
    pub upgradable_count: Option<u64>,
    /// Total security updates (if distinguishable).
    pub security_count: Option<u64>,
}

// ── Network ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    /// IPv4/IPv6 routes (ip route / ip -6 route).
    pub routes: Vec<RouteEntry>,
    /// Interfaces and addresses (ip -br addr).
    pub interfaces: Vec<InterfaceEntry>,
    /// /etc/resolv.conf nameservers.
    pub nameservers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEntry {
    pub destination: String,
    pub gateway: Option<String>,
    pub interface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceEntry {
    pub name: String,
    pub state: String,
    pub addresses: Vec<String>,
}

// ── Timers (separate snapshot for standalone or v1.8) ──────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerSnapshot {
    pub systemd_timers: Vec<SystemdTimer>,
    /// Cron jobs found in /etc/cron.* (profile ≥ Standard).
    pub cron_jobs: Vec<CronJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronJob {
    pub path: String,
    pub schedule: Option<String>,
}

// ── Processes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessSnapshot {
    /// Top processes by RSS from ps aux --sort=-%mem (bounded to top 20).
    pub top_by_memory: Vec<ProcessInfo>,
    /// Top processes by CPU from ps aux --sort=-%cpu (bounded to top 10).
    pub top_by_cpu: Vec<ProcessInfo>,
    /// Total running process count.
    pub total_count: u64,
    /// Zombie process count.
    pub zombie_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub command: String,
}

// ── Firewall ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallSnapshot {
    /// Whether iptables or nftables is available.
    pub firewall_tool: Option<String>,
    /// Whether ufw is active.
    pub ufw_active: Option<bool>,
    /// Whether firewalld is active.
    pub firewalld_active: Option<bool>,
    /// iptables rule count (from iptables -L -n | wc -l).
    pub iptables_rule_count: Option<u64>,
    /// Whether any default ACCEPT policy is found on INPUT chain.
    pub default_accept_input: Option<bool>,
}

// ── Root snapshot ──────────────────────────────────────────────────────────

/// Root `SystemSnapshot` per TRD §4.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub id: SnapshotId,
    pub host: HostIdentity,
    pub collected_at: DateTime<Utc>,
    pub profile: MonitorProfile,
    pub domains: SnapshotDomains,
    pub collector_errors: Vec<CollectorError>,
    pub redaction_version: String,
}

/// All domain sub-snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDomains {
    pub host: HostIdentity,
    pub load: LoadSnapshot,
    pub disks: DiskSnapshot,
    pub services: ServiceSnapshot,
    pub containers: ContainerSnapshot,
    pub ports: PortSnapshot,
    pub logs: LogSnapshot,
    pub backups: BackupSnapshot,
    pub packages: PackageSnapshot,
    pub timers: TimerSnapshot,
    pub network: NetworkSnapshot,
    pub processes: ProcessSnapshot,
    pub firewall: FirewallSnapshot,
    pub kubernetes: crate::collectors::kubernetes::KubernetesSnapshot,
    pub libvirt: crate::collectors::libvirt::LibvirtSnapshot,
    pub compose: crate::collectors::compose::ComposeSnapshot,
}

/// Non-fatal error from one collector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectorError {
    pub domain: String,
    pub message: String,
    pub is_timeout: bool,
}

impl SnapshotDomains {
    /// Return the 16 domain names in a stable order matching the struct fields.
    pub fn domain_names() -> Vec<&'static str> {
        vec![
            "host",
            "load",
            "disks",
            "services",
            "containers",
            "ports",
            "logs",
            "backups",
            "packages",
            "timers",
            "network",
            "processes",
            "firewall",
            "kubernetes",
            "libvirt",
            "compose",
        ]
    }
}

// ── Helper constructors ────────────────────────────────────────────────────

impl SystemSnapshot {
    /// Create a new snapshot with the given id, host, profile, and domains.
    pub fn new(
        id: SnapshotId,
        host: HostIdentity,
        profile: MonitorProfile,
        domains: SnapshotDomains,
    ) -> Self {
        Self {
            id,
            collected_at: Utc::now(),
            host: host.clone(),
            profile,
            domains,
            collector_errors: Vec::new(),
            redaction_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl Default for HostIdentity {
    fn default() -> Self {
        Self {
            hostname: String::from("unknown"),
            kernel_name: String::from("unknown"),
            kernel_release: String::from("unknown"),
            machine: String::from("unknown"),
            os_pretty_name: None,
            os_id: None,
            os_version_id: None,
            uptime_seconds: 0,
        }
    }
}

impl Default for LoadSnapshot {
    fn default() -> Self {
        Self {
            load_average: LoadAverage {
                one: 0.0,
                five: 0.0,
                fifteen: 0.0,
            },
            cpu_logical_count: 0,
            cpu_pressure: None,
            memory_pressure: None,
            io_pressure: None,
            memory: MemoryInfo {
                total: 0,
                used: 0,
                available: None,
            },
            swap_total: 0,
            swap_used: 0,
        }
    }
}

impl Default for DiskSnapshot {
    fn default() -> Self {
        Self {
            filesystems: Vec::new(),
            mounts: Vec::new(),
            block_devices: Vec::new(),
            smart_available: false,
            smart_devices: Vec::new(),
            inodes: Vec::new(),
        }
    }
}

impl Default for ServiceSnapshot {
    fn default() -> Self {
        Self {
            units: Vec::new(),
            failed_units: Vec::new(),
            timers: Vec::new(),
        }
    }
}

impl Default for ContainerSnapshot {
    fn default() -> Self {
        Self {
            runtime: None,
            containers: Vec::new(),
        }
    }
}

impl Default for PortSnapshot {
    fn default() -> Self {
        Self {
            listeners: Vec::new(),
        }
    }
}

impl Default for LogSnapshot {
    fn default() -> Self {
        Self {
            journal_errors_last_hour: 0,
            kernel_errors: Vec::new(),
            auth_failures: Vec::new(),
            error_rate_per_minute: None,
        }
    }
}

impl Default for BackupSnapshot {
    fn default() -> Self {
        Self {
            tools_detected: Vec::new(),
        }
    }
}

impl Default for PackageSnapshot {
    fn default() -> Self {
        Self {
            package_manager: None,
            upgradable_count: None,
            security_count: None,
        }
    }
}

impl Default for NetworkSnapshot {
    fn default() -> Self {
        Self {
            routes: Vec::new(),
            interfaces: Vec::new(),
            nameservers: Vec::new(),
        }
    }
}

impl Default for TimerSnapshot {
    fn default() -> Self {
        Self {
            systemd_timers: Vec::new(),
            cron_jobs: Vec::new(),
        }
    }
}

impl Default for ProcessSnapshot {
    fn default() -> Self {
        Self {
            top_by_memory: Vec::new(),
            top_by_cpu: Vec::new(),
            total_count: 0,
            zombie_count: 0,
        }
    }
}

impl Default for FirewallSnapshot {
    fn default() -> Self {
        Self {
            firewall_tool: None,
            ufw_active: None,
            firewalld_active: None,
            iptables_rule_count: None,
            default_accept_input: None,
        }
    }
}

impl Default for SnapshotDomains {
    fn default() -> Self {
        Self {
            host: HostIdentity::default(),
            load: LoadSnapshot::default(),
            disks: DiskSnapshot::default(),
            services: ServiceSnapshot::default(),
            containers: ContainerSnapshot::default(),
            ports: PortSnapshot::default(),
            logs: LogSnapshot::default(),
            backups: BackupSnapshot::default(),
            packages: PackageSnapshot::default(),
            timers: TimerSnapshot::default(),
            network: NetworkSnapshot::default(),
            processes: ProcessSnapshot::default(),
            firewall: FirewallSnapshot::default(),
            kubernetes: crate::collectors::kubernetes::KubernetesSnapshot::default(),
            libvirt: crate::collectors::libvirt::LibvirtSnapshot::default(),
            compose: crate::collectors::compose::ComposeSnapshot::default(),
        }
    }
}
