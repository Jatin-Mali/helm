//! Disks collector: filesystems, mounts, block devices, inodes, SMART.
use crate::{
    collectors::{Collector, bin_exists, err, run_timed},
    snapshot::{
        BlockDevice, DiskSnapshot, FilesystemEntry, InodeEntry, MonitorProfile, MountEntry,
        SmartDevice,
    },
};
#[derive(Default)]
pub struct DisksCollector;
impl Collector for DisksCollector {
    type Output = DiskSnapshot;
    fn domain(&self) -> &'static str {
        "disks"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = DiskSnapshot::default();
        match run_timed("df", &["-B1"], profile).await {
            Ok(o) => out.filesystems = parse_df(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => return Err(err("disks", e.message)),
        }
        if let Ok(o) = run_timed("df", &["-i"], profile).await {
            out.inodes = parse_inode(&String::from_utf8_lossy(&o.stdout));
        }
        if let Ok(o) = run_timed(
            "findmnt",
            &["-nlo", "SOURCE,TARGET,FSTYPE,OPTIONS"],
            profile,
        )
        .await
        {
            out.mounts = parse_mnt(&String::from_utf8_lossy(&o.stdout));
        }
        if let Ok(o) = run_timed("lsblk", &["-nlo", "NAME,SIZE,RO,MOUNTPOINTS"], profile).await {
            out.block_devices = parse_blk(&String::from_utf8_lossy(&o.stdout));
        }
        out.smart_available = bin_exists("smartctl");
        if out.smart_available && profile.deep_probes() {
            for d in out.block_devices.iter().map(|d| format!("/dev/{}", d.name)) {
                if let Ok(o) = run_timed("smartctl", &["-H", &d], profile).await {
                    if o.status.success() {
                        let s = String::from_utf8_lossy(&o.stdout);
                        let h = if s.contains("PASSED") || s.contains("OK") {
                            Some("PASSED".into())
                        } else if s.contains("FAIL") {
                            Some("FAIL".into())
                        } else {
                            Some("UNKNOWN".into())
                        };
                        out.smart_devices.push(SmartDevice {
                            device: d,
                            model: None,
                            health: h,
                            temperature_celsius: None,
                        });
                    }
                }
            }
        }
        Ok(out)
    }
}
fn parse_df(s: &str) -> Vec<FilesystemEntry> {
    s.lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 6 {
                None
            } else {
                Some(FilesystemEntry {
                    device: p[0].into(),
                    mount_point: p[5].into(),
                    fs_type: String::new(),
                    total_bytes: p[1].parse().unwrap_or(0),
                    used_bytes: p[2].parse().unwrap_or(0),
                    available_bytes: p[3].parse().unwrap_or(0),
                })
            }
        })
        .collect()
}
fn parse_inode(s: &str) -> Vec<InodeEntry> {
    s.lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 6 {
                None
            } else {
                Some(InodeEntry {
                    device: p[0].into(),
                    mount_point: p[5].into(),
                    total: p[1].parse().unwrap_or(0),
                    used: p[2].parse().unwrap_or(0),
                    free: p[3].parse().unwrap_or(0),
                })
            }
        })
        .collect()
}
fn parse_mnt(s: &str) -> Vec<MountEntry> {
    s.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 3 {
                None
            } else {
                Some(MountEntry {
                    source: p[0].into(),
                    target: p[1].into(),
                    fs_type: p[2].into(),
                    options: p.get(3).unwrap_or(&"").to_string(),
                })
            }
        })
        .collect()
}
fn parse_blk(s: &str) -> Vec<BlockDevice> {
    s.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.is_empty() {
                None
            } else {
                let mps: Vec<String> = if p.len() > 3 {
                    p[3..].iter().map(|s| s.to_string()).collect()
                } else {
                    vec![]
                };
                Some(BlockDevice {
                    name: p[0].into(),
                    size: p.get(1).and_then(|s| parse_hsize(s)),
                    ro: p.get(2).map(|s| *s == "1").unwrap_or(false),
                    mount_points: mps,
                })
            }
        })
        .collect()
}
fn parse_hsize(s: &str) -> Option<u64> {
    let (n, m) = if let Some(r) = s.trim().strip_suffix('G') {
        (r.trim(), 1073741824u64)
    } else if let Some(r) = s.trim().strip_suffix('M') {
        (r.trim(), 1048576)
    } else if let Some(r) = s.trim().strip_suffix('K') {
        (r.trim(), 1024)
    } else if let Some(r) = s.trim().strip_suffix('T') {
        (r.trim(), 1099511627776u64)
    } else {
        (s.trim(), 1)
    };
    n.parse::<f64>().ok().map(|v| (v * m as f64) as u64)
}
