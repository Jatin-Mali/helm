use chrono::{TimeZone, Utc};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use helm_monitor::{
    correlation::CorrelationEngine,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    metric_history::MetricHistory,
    ring_buffer::RingBuffer,
    snapshot::{HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot},
};
use std::time::Duration;

// ── helpers ─────────────────────────────────────────────────────────────────

fn make_snapshot(second: i64) -> SystemSnapshot {
    SystemSnapshot {
        id: format!("bench-snap-{second}"),
        host: HostIdentity::default(),
        collected_at: Utc.timestamp_opt(second, 0).unwrap(),
        profile: MonitorProfile::Standard,
        domains: SnapshotDomains::default(),
        collector_errors: Vec::new(),
        redaction_version: "bench".to_owned(),
    }
}

fn make_finding(snapshot_id: &str, n: usize) -> Finding {
    Finding::new(
        snapshot_id,
        &format!("detector-{n}"),
        &format!("/resource/{n}"),
        &format!("Bench finding {n}"),
        Severity::Warning,
        Confidence::High,
        MonitorDomain::Load,
    )
}

// ── RingBuffer ───────────────────────────────────────────────────────────────

fn bench_ring_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring_buffer");

    for cap in [64usize, 256, 1024] {
        group.bench_with_input(BenchmarkId::new("push", cap), &cap, |b, &cap| {
            let mut buf: RingBuffer<f64> = RingBuffer::new(cap);
            let mut v = 0.0f64;
            b.iter(|| {
                buf.push(v);
                v += 0.001;
            });
        });
    }

    group.finish();
}

// ── MetricHistory ─────────────────────────────────────────────────────────────

fn bench_metric_history(c: &mut Criterion) {
    let mut group = c.benchmark_group("metric_history");

    for cap in [60usize, 300, 1440] {
        group.bench_with_input(BenchmarkId::new("push_snapshot", cap), &cap, |b, &cap| {
            let mut hist = MetricHistory::new(cap);
            let snap = make_snapshot(0);
            b.iter(|| {
                hist.push_snapshot(&snap);
            });
        });
    }

    group.finish();
}

// ── CorrelationEngine ─────────────────────────────────────────────────────────

fn bench_correlation(c: &mut Criterion) {
    let mut group = c.benchmark_group("correlation");
    group.measurement_time(Duration::from_secs(10));

    for n_findings in [10usize, 50, 200] {
        group.bench_with_input(
            BenchmarkId::new("correlate", n_findings),
            &n_findings,
            |b, &n| {
                let snap = make_snapshot(1000);
                let findings: Vec<Finding> = (0..n).map(|i| make_finding(&snap.id, i)).collect();

                b.iter(|| {
                    let mut engine = CorrelationEngine::new(Duration::from_secs(300));
                    engine.correlate(&snap, &findings)
                });
            },
        );
    }

    group.finish();
}

// ── harness ───────────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_ring_buffer,
    bench_metric_history,
    bench_correlation
);
criterion_main!(benches);
