//! Processes collector: ps aux top-by-memory and top-by-cpu.
use crate::{
    collectors::{Collector, run_timed},
    snapshot::{MonitorProfile, ProcessInfo, ProcessSnapshot},
};

pub struct ProcessCollector;
impl Collector for ProcessCollector {
    type Output = ProcessSnapshot;
    fn domain(&self) -> &'static str {
        "processes"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = ProcessSnapshot::default();

        // ps aux --sort=-%mem for top memory consumers
        if let Ok(o) = run_timed("ps", &["aux", "--sort=-%mem"], profile).await {
            let s = String::from_utf8_lossy(&o.stdout);
            out.top_by_memory = parse_ps(&s, 20);
        }

        // ps aux --sort=-%cpu for top CPU consumers
        if let Ok(o) = run_timed("ps", &["aux", "--sort=-%cpu"], profile).await {
            let s = String::from_utf8_lossy(&o.stdout);
            out.top_by_cpu = parse_ps(&s, 10);
        }

        // Total process count and zombies via ps -eo stat
        if let Ok(o) = run_timed("ps", &["-eo", "stat"], profile).await {
            let s = String::from_utf8_lossy(&o.stdout);
            for line in s.lines().skip(1) {
                out.total_count += 1;
                if line.starts_with('Z') {
                    out.zombie_count += 1;
                }
            }
        }

        Ok(out)
    }
}

fn parse_ps(output: &str, limit: usize) -> Vec<ProcessInfo> {
    output
        .lines()
        .skip(1) // header
        .take(limit)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 11 {
                return None;
            }
            Some(ProcessInfo {
                pid: parts[1].parse().unwrap_or(0),
                user: parts[0].to_string(),
                cpu_percent: parts[2].parse().unwrap_or(0.0),
                mem_percent: parts[3].parse().unwrap_or(0.0),
                command: parts[10..].join(" "),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_parses_realistic() {
        let input = "USER PID %CPU %MEM VSZ RSS TTY STAT START TIME COMMAND\nroot 1 0.0 0.1 169740 13348 ? Ss May12 0:12 /sbin/init\n";
        let procs = parse_ps(input, 5);
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].pid, 1);
        assert_eq!(procs[0].user, "root");
        assert!(procs[0].command.contains("/sbin/init"));
    }
}
