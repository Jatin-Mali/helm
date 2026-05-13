# Detector False-Positive Review Checklist — v1.8

Per TRD §6, every detector must be reviewed for false-positive risk before
release. This checklist tracks that review.

## Review status

| Detector | Reviewed | False-positive risk | Mitigation |
|----------|----------|---------------------|------------|
| disk-usage | 2026-05-13 | Low | Threshold-based (80/90/95%). Fixed filesystems like /boot or /efi may legitimately be >80% full without being a problem. Future: add allowlist for known-small partitions. |
| inode-usage | 2026-05-13 | Low | Threshold 85/95%. Rarely triggers on normal systems. Inode-exhausted filesystems are almost always a real issue. |
| smart-health | 2026-05-13 | Low | SMART FAIL is objective. Missing SMART is info-only (not a false positive — it's a true statement that SMART data is unavailable). |
| fs-readonly | 2026-05-13 | Low | RO mounts are directly observable from mount options. Excludes /sys to avoid noise. |
| failed-services | 2026-05-13 | Low | Systemd failed-state is objective. May flag intentionally-failed test units; these are rare in production. |
| restart-loop | 2026-05-13 | Medium | Heuristic: sub == "auto-restart" or contains "restart". RestartPolicy=always containers will cycle normally. Mitigation: confidence limited to Medium. |
| inactive-service | 2026-05-13 | Medium | Loaded-but-inactive may be intentional (sockets, manually stopped). Mitigation: severity is Info only. Confidence Medium. |
| unhealthy-container | 2026-05-13 | Low | Docker health check status is objective. "Exited" containers may be intentional one-shot jobs. These are Info only. |
| container-restart | 2026-05-13 | Medium | Restart count threshold of 5. Containers under orchestrators may legitimately restart. Mitigation: confidence Medium. |
| exposed-port | 2026-05-13 | High | Flags ALL non-loopback listeners, including intentional services like nginx on :80. Mitigation: severity is Info only. Future: add expected-port allowlist. |
| high-load | 2026-05-13 | Low | 1.5x/3x core threshold. Build servers or HPC nodes may naturally exceed this. Mitigation: confidence High only when sustained at 15min avg. |
| memory-pressure | 2026-05-13 | Low | 85/95% thresholds. Linux uses memory for caching; available-memory check guards against false "full" alarms. PSI >10 avg60 is reliable pressure signal. |
| swap-exhaustion | 2026-05-13 | Low | >90% swap used. On systems with very small swap, this can trigger during normal operation. Mitigation: only warns, not critical. |
| oom-event | 2026-05-13 | Very Low | OOM killer logged in kernel messages is always a real event. |
| journal-errors | 2026-05-13 | Medium | Fixed thresholds (20/100 per hour). Noisy systems may have persistent error rates. Mitigation: baseline-aware mode compares against previous snapshot; elevated-info only fires when >2x prior rate. |
| backup-stale | 2026-05-13 | Low | True negative: no backup tools found is objective. May miss non-standard backup solutions. Mitigation: confidence High for detection, notes missing-data. |
| backup-schedule | 2026-05-13 | Medium | Schedule detection is heuristic (checking cron/timers for tool names). External orchestrators are invisible. Mitigation: confidence Medium, explicit assumption note. |
| security-updates | 2026-05-13 | Low | Package manager listing is objective. May flag updates that are not security-critical on this system. Mitigation: only flags count, not specific CVEs. |
| restore-test-missing | 2026-05-14 | Low | Checks for restic/borg cache evidence as proxy for restore-test execution. Cache absence does not prove restores were never tested. Mitigation: confidence Medium, notes missing data explicitly. |

## Release decision

All detectors reviewed. No blocker false-positive risks for v1.8 release.
The highest-risk detector is **exposed-port** (Info-only). Future releases
should add user-configurable port allowlists to reduce noise.
