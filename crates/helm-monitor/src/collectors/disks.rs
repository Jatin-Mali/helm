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
                    {
                        let h = Some(smart_health(
                            o.status.code().unwrap_or(1),
                            &String::from_utf8_lossy(&o.stdout),
                        ));
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
fn smart_health(code: i32, text: &str) -> String {
    if code & 0b11000 != 0 || text.contains("FAIL") {
        "FAIL".into()
    } else if code & 0b11100000 != 0 {
        format!("WARN: SMART history or error log flags ({code})")
    } else if text.contains("PASSED") || text.contains("OK") {
        "PASSED".into()
    } else {
        "UNKNOWN".into()
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

#[cfg(test)]
mod tests {
    #[test]
    fn smart_nonzero_health_bits_remain_visible() {
        assert_eq!(
            super::smart_health(
                8,
                "SMART overall-health self-assessment test result: FAILED!"
            ),
            "FAIL"
        );
        assert_eq!(super::smart_health(16, "PASSED"), "FAIL");
        assert!(super::smart_health(64, "PASSED").starts_with("WARN:"));
        assert_eq!(super::smart_health(0, "PASSED"), "PASSED");
        assert_eq!(super::smart_health(0, "unrecognized result"), "UNKNOWN");
    }
}
