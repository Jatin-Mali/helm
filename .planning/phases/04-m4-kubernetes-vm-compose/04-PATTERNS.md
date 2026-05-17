# Phase 4: M4 — Kubernetes / VM / Compose - Pattern Map

**Mapped:** 2026-05-18
**Files analyzed:** 3 new collectors + snapshot/engine updates
**Analogs found:** 3 / 3

## File Classification

| New/Modified File | Role | Data Flow | Closest Analog | Match Quality |
|-------------------|------|-----------|----------------|---------------|
| `crates/helm-monitor/src/collectors/kubernetes.rs` | collector | request-response | `crates/helm-monitor/src/collectors/containers.rs` | exact |
| `crates/helm-monitor/src/collectors/libvirt.rs` | collector | request-response | `crates/helm-monitor/src/collectors/disks.rs` | role-match |
| `crates/helm-monitor/src/collectors/compose.rs` | collector | request-response | `crates/helm-monitor/src/collectors/containers.rs` | exact |
| `crates/helm-monitor/src/snapshot.rs` | model | CRUD | (existing) | update-only |
| `crates/helm-monitor/src/engine.rs` | orchestrator | CRUD | (existing) | update-only |

## Pattern Assignments

### `crates/helm-monitor/src/collectors/kubernetes.rs` (collector, request-response)

**Analog:** `crates/helm-monitor/src/collectors/containers.rs` (lines 1–48)

**Collector trait implementation** (lines 6–12):
```rust
#[derive(Default)]
pub struct KubernetesCollector;

impl Collector for KubernetesCollector {
    type Output = KubernetesSnapshot;
    
    fn domain(&self) -> &'static str {
        "kubernetes"
    }
```

**Error handling pattern** (lines 44–45):
```rust
Err(e) => return Err(err("containers", e.message)),
```
Copy error handling to kubernetes.rs: use `err(domain, message)` helper from `crates/helm-monitor/src/collectors/mod.rs` line 92.

**Binary availability check** (lines 18–24):
```rust
let rt = if bin_exists("docker") {
    Some(ContainerRuntime::Docker)
} else if bin_exists("podman") {
    Some(ContainerRuntime::Podman)
} else {
    return Ok(out);
};
```
For kubernetes.rs: check `bin_exists("kubectl")` (line 83 in mod.rs), return early if missing.

**Command execution with timeout** (lines 30–43):
```rust
match run_timed(
    cmd,
    &["ps", "-a", "--no-trunc", "--format", "..."],
    profile,
)
.await
{
    Ok(o) => out.containers = parse_list(&String::from_utf8_lossy(&o.stdout)),
    Err(e) => return Err(err("containers", e.message)),
}
```
For kubernetes.rs: use `run_timed("kubectl", &["get", "pods", ...], profile)` with identical error handling.

**Text parsing helper** (lines 49–76):
```rust
fn parse_list(s: &str) -> Vec<ContainerInfo> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let p: Vec<&str> = l.split('\t').collect();
            // ... parse tab-separated fields into typed struct
        })
        .collect()
}
```
Pattern for kubernetes.rs: create `parse_kubectl(s: &str) -> Vec<PodInfo>` using same structure.

---

### `crates/helm-monitor/src/collectors/libvirt.rs` (collector, request-response)

**Analog:** `crates/helm-monitor/src/collectors/disks.rs` (lines 1–65)

**Multi-step collector pattern** (lines 9–63):
```rust
#[derive(Default)]
pub struct DisksCollector;

impl Collector for DisksCollector {
    type Output = DiskSnapshot;
    
    fn domain(&self) -> &'static str {
        "disks"
    }
    
    async fn collect(self, profile: MonitorProfile) -> Result<Self::Output, CollectorError> {
        let mut out = DiskSnapshot::default();
        
        // Step 1: run_timed + parse
        match run_timed("df", &["-B1"], profile).await {
            Ok(o) => out.filesystems = parse_df(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => return Err(err("disks", e.message)),
        }
        
        // Step 2: optional command (if let)
        if let Ok(o) = run_timed("df", &["-i"], profile).await {
            out.inodes = parse_inode(&String::from_utf8_lossy(&o.stdout));
        }
        
        // Step 3: optional check with bin_exists
        out.smart_available = bin_exists("smartctl");
        if out.smart_available && profile.deep_probes() {
            // ... conditional deep probe
        }
        Ok(out)
    }
}
```
For libvirt.rs: apply exact pattern: check `bin_exists("virsh")`, run primary command, optional secondary commands, return structured snapshot.

**Graceful degradation** (lines 25–26, 28–36):
Use `if let Ok(o) = run_timed(...)` for non-critical commands so collector doesn't fail entirely if one tool is missing.

---

### `crates/helm-monitor/src/collectors/compose.rs` (collector, request-response)

**Analog:** `crates/helm-monitor/src/collectors/containers.rs` (lines 1–48)

**Inherit full implementation pattern** because Compose is a layer over Docker:
- Trait impl with `type Output = ComposeSnapshot`
- `domain()` returns `"compose"`
- `collect()` runs `docker-compose` and `docker` commands via `run_timed()`
- Parse text output into typed struct
- Use identical error handling with `err()` helper

**Key difference:** Compose collector may call both:
```rust
run_timed("docker-compose", &["ps", "-a"], profile).await
run_timed("docker", &["ps", "-a", "--filter", "label=com.docker.compose.project"], profile).await
```

---

## Snapshot and Engine Updates

### Add snapshot types to `crates/helm-monitor/src/snapshot.rs`

**Reference structure** (lines 231–264):
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSnapshot {
    pub runtime: Option<ContainerRuntime>,
    pub containers: Vec<ContainerInfo>,
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
```

Create three new snapshot types following this pattern:
- `KubernetesSnapshot { pods: Vec<PodInfo>, ... }`
- `LibvirtSnapshot { domains: Vec<DomainInfo>, ... }`
- `ComposeSnapshot { projects: Vec<ComposeProject>, ... }`

Add to `SnapshotDomains` struct (lines 426–440):
```rust
pub struct SnapshotDomains {
    pub host: HostIdentity,
    pub load: LoadSnapshot,
    pub disks: DiskSnapshot,
    // ... existing fields ...
    pub kubernetes: KubernetesSnapshot,  // NEW
    pub libvirt: LibvirtSnapshot,        // NEW
    pub compose: ComposeSnapshot,         // NEW
}
```

---

### Update engine.rs to invoke new collectors

**Reference pattern** (lines 6–50):
```rust
use crate::collectors::{
    backups::BackupsCollector, containers::ContainersCollector, disks::DisksCollector,
    // ... existing imports ...
};

pub async fn collect_snapshot(profile: MonitorProfile) -> SystemSnapshot {
    let (
        host_result,
        load_result,
        disks_result,
        // ... existing fields ...
    ) = tokio::join!(
        HostCollector.collect(profile),
        LoadCollector.collect(profile),
        DisksCollector.collect(profile),
        // ... existing calls ...
    );
```

**Steps:**
1. Line 6–11: Add imports for `KubernetesCollector`, `LibvirtCollector`, `ComposeCollector`
2. Line 36: Add three new fields in `tokio::join!()` destructure
3. Line 36+N: Add three new `.collect(profile)` calls in `tokio::join!()`
4. Line 52–80: Add three `unwrap_or_default()` calls, assigning to local vars
5. Line 66–80: Add three fields to `SnapshotDomains { ... }` struct initialization

---

## Shared Patterns

### Command Execution with Timeout (all collectors)
**Source:** `crates/helm-monitor/src/collectors/mod.rs` lines 44–70
```rust
async fn run_timed(
    program: &str,
    args: &[&str],
    profile: MonitorProfile,
) -> Result<Output, CollectorError> {
    let timeout = profile.per_collector_timeout();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout),
        Command::new(program).args(args).output(),
    )
    .await;
    
    match output {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(CollectorError { domain: String::new(), message: format!("{program}: {e}"), is_timeout: false }),
        Err(_elapsed) => Err(CollectorError { domain: String::new(), message: format!("{program} timed out after {timeout}s"), is_timeout: true }),
    }
}
```
**Apply to:** All new collectors; no changes needed — call as-is from all three (kubernetes, libvirt, compose).

### Binary Availability Check
**Source:** `crates/helm-monitor/src/collectors/mod.rs` lines 82–89
```rust
fn bin_exists(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```
**Apply to:** All three collectors before running commands; return early with default snapshot if missing.

### Error Handling
**Source:** `crates/helm-monitor/src/collectors/mod.rs` lines 91–107
```rust
fn err(domain: &str, msg: impl Into<String>) -> CollectorError { ... }
fn timeout_err(domain: &str, msg: impl Into<String>) -> CollectorError { ... }
```
**Apply to:** All error branches; use `err("kubernetes", msg)`, `err("libvirt", msg)`, `err("compose", msg)`.

### Collector Trait
**Source:** `crates/helm-monitor/src/collectors/mod.rs` lines 31–42
All three new collectors must implement exactly this trait (no modifications needed).

---

## Detector Integration (Future)

When Phase 4 includes detector rules for kubernetes/libvirt/compose pods, follow the pattern from `crates/helm-monitor/src/detectors/container_restart.rs`:

1. Create `crates/helm-monitor/src/detectors/pod_restart.rs` (kubernetes)
2. Implement `Detector` trait: `id()`, `domain()`, `detect()` returning `Vec<Finding>`
3. Register in `DetectorRegistry::default_registry()` (lines 77–98 of mod.rs)
4. Add `MonitorDomain::Kubernetes` enum variant if new domain is needed

---

## No Analog Found

None — all patterns are directly analogous to existing collectors.

## Metadata

**Analog search scope:** `crates/helm-monitor/src/collectors/` and `crates/helm-monitor/src/`
**Files scanned:** 14 collector files, mod.rs, engine.rs, snapshot.rs, detectors/
**Pattern extraction date:** 2026-05-18
