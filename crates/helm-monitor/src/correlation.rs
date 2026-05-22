//! Correlation engine for grouping related findings and surfacing likely root causes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    findings::{Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

const DEFAULT_TEMPORAL_WINDOW_SECS: u64 = 15 * 60;
const DEFAULT_MIN_TEMPORAL_CONFIRMATIONS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalRelationship {
    ResourceDependency,
    TemporalPrecedence,
    SharedResource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalLink {
    pub cause_finding_id: String,
    pub effect_finding_id: String,
    pub relationship: CausalRelationship,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrelationGroup {
    pub id: String,
    pub primary_finding_id: String,
    pub finding_ids: Vec<String>,
    pub causal_links: Vec<CausalLink>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorrelationResult {
    pub findings: Vec<Finding>,
    pub groups: Vec<CorrelationGroup>,
}

#[derive(Debug, Clone)]
pub struct CorrelationEngine {
    temporal_window: Duration,
    min_temporal_confirmations: u32,
    temporal_history: HashMap<String, VecDeque<DateTime<Utc>>>,
    precedence_counts: HashMap<(String, String), u32>,
    dependency_graph: HashMap<String, HashSet<String>>,
}

#[derive(Debug, Clone)]
struct RawRelation {
    left: usize,
    right: usize,
    relationship: CausalRelationship,
    directed: Option<(usize, usize)>,
}

impl CorrelationEngine {
    pub fn new(temporal_window: Duration) -> Self {
        Self {
            temporal_window,
            min_temporal_confirmations: DEFAULT_MIN_TEMPORAL_CONFIRMATIONS,
            temporal_history: HashMap::new(),
            precedence_counts: HashMap::new(),
            dependency_graph: HashMap::new(),
        }
    }

    pub fn add_dependency(&mut self, resource: impl AsRef<str>, dependent: impl AsRef<str>) {
        let resource = normalize_resource(resource.as_ref());
        let dependent = normalize_resource(dependent.as_ref());
        if resource.is_empty() || dependent.is_empty() || resource == dependent {
            return;
        }
        self.dependency_graph
            .entry(resource)
            .or_default()
            .insert(dependent);
    }

    pub fn set_min_temporal_confirmations(&mut self, confirmations: u32) {
        self.min_temporal_confirmations = confirmations.max(1);
    }

    pub fn correlate(
        &mut self,
        snapshot: &SystemSnapshot,
        findings: &[Finding],
    ) -> CorrelationResult {
        self.seed_snapshot_dependencies(snapshot);
        self.prune_history(snapshot.collected_at);
        self.observe_precedence(findings, snapshot.collected_at);

        let mut correlated_findings = findings.to_vec();
        for finding in &mut correlated_findings {
            finding.clear_correlation();
        }

        let relations = self.build_relations(&correlated_findings);
        let groups = self.build_groups(&mut correlated_findings, relations);
        self.record_history(&correlated_findings, snapshot.collected_at);

        CorrelationResult {
            findings: correlated_findings,
            groups,
        }
    }

    fn seed_snapshot_dependencies(&mut self, snapshot: &SystemSnapshot) {
        for project in &snapshot.domains.compose.projects {
            let project_prefix = format!("{}_", project.name.to_ascii_lowercase());
            for container in &snapshot.domains.containers.containers {
                if container
                    .name
                    .to_ascii_lowercase()
                    .starts_with(&project_prefix)
                {
                    self.add_dependency(&container.name, &project.name);
                }
            }
        }

        for listener in &snapshot.domains.ports.listeners {
            if let Some(process_name) = listener.process_name.as_deref() {
                let port_resource = format!("{}:{}", listener.local_address, listener.local_port);
                self.add_dependency(process_name, &port_resource);
            }
        }

        for pod in &snapshot.domains.kubernetes.pods {
            let namespaced_pod = format!("{}/{}", pod.namespace, pod.name);
            self.add_dependency(&pod.name, &namespaced_pod);
        }
    }

    fn prune_history(&mut self, observed_at: DateTime<Utc>) {
        let Ok(window) = chrono::Duration::from_std(self.temporal_window) else {
            return;
        };
        let cutoff = observed_at - window;
        self.temporal_history.retain(|_, timestamps| {
            while timestamps.front().is_some_and(|ts| *ts < cutoff) {
                timestamps.pop_front();
            }
            !timestamps.is_empty()
        });
    }

    fn observe_precedence(&mut self, findings: &[Finding], observed_at: DateTime<Utc>) {
        let Ok(window) = chrono::Duration::from_std(self.temporal_window) else {
            return;
        };
        let cutoff = observed_at - window;
        let current_fingerprints = findings
            .iter()
            .map(Finding::fingerprint)
            .collect::<HashSet<_>>();
        let history_snapshot = self.temporal_history.clone();

        for finding in findings {
            let effect = finding.fingerprint();
            for (cause, timestamps) in &history_snapshot {
                if cause == &effect || current_fingerprints.contains(cause) {
                    continue;
                }
                if timestamps
                    .iter()
                    .any(|timestamp| *timestamp >= cutoff && *timestamp < observed_at)
                {
                    *self
                        .precedence_counts
                        .entry((cause.clone(), effect.clone()))
                        .or_insert(0) += 1;
                }
            }
        }
    }

    fn record_history(&mut self, findings: &[Finding], observed_at: DateTime<Utc>) {
        let mut inserted = HashSet::new();
        for finding in findings {
            let fingerprint = finding.fingerprint();
            if !inserted.insert(fingerprint.clone()) {
                continue;
            }
            self.temporal_history
                .entry(fingerprint)
                .or_default()
                .push_back(observed_at);
        }
    }

    fn build_relations(&self, findings: &[Finding]) -> Vec<RawRelation> {
        let mut relations = Vec::new();
        for left in 0..findings.len() {
            for right in (left + 1)..findings.len() {
                let left_resource = normalize_resource(&findings[left].affected_resource);
                let right_resource = normalize_resource(&findings[right].affected_resource);

                if shared_resource(&left_resource, &right_resource) {
                    relations.push(RawRelation {
                        left,
                        right,
                        relationship: CausalRelationship::SharedResource,
                        directed: None,
                    });
                }

                if self.depends_on(&left_resource, &right_resource) {
                    relations.push(RawRelation {
                        left,
                        right,
                        relationship: CausalRelationship::ResourceDependency,
                        directed: Some((left, right)),
                    });
                } else if self.depends_on(&right_resource, &left_resource) {
                    relations.push(RawRelation {
                        left,
                        right,
                        relationship: CausalRelationship::ResourceDependency,
                        directed: Some((right, left)),
                    });
                }

                let left_fp = findings[left].fingerprint();
                let right_fp = findings[right].fingerprint();
                let left_precedes = self.temporal_precedence_confirmed(&left_fp, &right_fp);
                let right_precedes = self.temporal_precedence_confirmed(&right_fp, &left_fp);
                match (left_precedes, right_precedes) {
                    (true, false) => relations.push(RawRelation {
                        left,
                        right,
                        relationship: CausalRelationship::TemporalPrecedence,
                        directed: Some((left, right)),
                    }),
                    (false, true) => relations.push(RawRelation {
                        left,
                        right,
                        relationship: CausalRelationship::TemporalPrecedence,
                        directed: Some((right, left)),
                    }),
                    _ => {}
                }
            }
        }
        relations
    }

    fn depends_on(&self, resource: &str, dependent: &str) -> bool {
        self.dependency_graph
            .get(resource)
            .is_some_and(|dependents| dependents.contains(dependent))
    }

    fn temporal_precedence_confirmed(&self, cause_fp: &str, effect_fp: &str) -> bool {
        self.precedence_counts
            .get(&(cause_fp.to_owned(), effect_fp.to_owned()))
            .copied()
            .unwrap_or(0)
            >= self.min_temporal_confirmations
    }

    fn build_groups(
        &self,
        findings: &mut [Finding],
        relations: Vec<RawRelation>,
    ) -> Vec<CorrelationGroup> {
        if findings.is_empty() {
            return Vec::new();
        }

        let mut parent: Vec<usize> = (0..findings.len()).collect();
        for relation in &relations {
            union(&mut parent, relation.left, relation.right);
        }

        let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
        for index in 0..findings.len() {
            let root = find(&mut parent, index);
            components.entry(root).or_default().push(index);
        }

        let mut groups = Vec::new();
        for mut members in components.into_values() {
            if members.len() < 2 {
                continue;
            }
            members.sort_unstable();
            let component_relations = relations
                .iter()
                .filter(|relation| {
                    members.contains(&relation.left) && members.contains(&relation.right)
                })
                .cloned()
                .collect::<Vec<_>>();
            let primary = select_primary(&members, &component_relations, findings);
            let correlation_id = build_group_id(&members, findings);

            let mut ordered_members = members.clone();
            ordered_members.sort_by(|left, right| {
                if *left == primary {
                    std::cmp::Ordering::Less
                } else if *right == primary {
                    std::cmp::Ordering::Greater
                } else {
                    finding_rank(&findings[*right]).cmp(&finding_rank(&findings[*left]))
                }
            });

            let related_ids = ordered_members
                .iter()
                .map(|index| findings[*index].id.clone())
                .collect::<Vec<_>>();
            let primary_id = findings[primary].id.clone();

            for index in &ordered_members {
                findings[*index].correlation_id = Some(correlation_id.clone());
                findings[*index].root_cause_finding_id = Some(primary_id.clone());
                findings[*index].correlated_finding_ids = related_ids
                    .iter()
                    .filter(|id| *id != &findings[*index].id)
                    .cloned()
                    .collect();
            }

            groups.push(CorrelationGroup {
                id: correlation_id,
                primary_finding_id: primary_id,
                finding_ids: ordered_members
                    .iter()
                    .map(|index| findings[*index].id.clone())
                    .collect(),
                causal_links: build_causal_links(
                    primary,
                    &ordered_members,
                    &component_relations,
                    findings,
                ),
            });
        }

        groups.sort_by(|left, right| left.id.cmp(&right.id));
        groups
    }
}

impl Default for CorrelationEngine {
    fn default() -> Self {
        Self::new(Duration::from_secs(DEFAULT_TEMPORAL_WINDOW_SECS))
    }
}

fn build_causal_links(
    primary: usize,
    members: &[usize],
    relations: &[RawRelation],
    findings: &[Finding],
) -> Vec<CausalLink> {
    let mut links = Vec::new();
    let mut seen = HashSet::new();

    for relation in relations {
        let (cause, effect) = match relation.directed {
            Some(pair) => pair,
            None => {
                if relation.left == primary {
                    (primary, relation.right)
                } else if relation.right == primary {
                    (primary, relation.left)
                } else if finding_rank(&findings[relation.left])
                    >= finding_rank(&findings[relation.right])
                {
                    (relation.left, relation.right)
                } else {
                    (relation.right, relation.left)
                }
            }
        };
        let key = (cause, effect, relation.relationship);
        if !seen.insert(key) {
            continue;
        }
        links.push(CausalLink {
            cause_finding_id: findings[cause].id.clone(),
            effect_finding_id: findings[effect].id.clone(),
            relationship: relation.relationship,
        });
    }

    if links.is_empty() {
        for member in members {
            if *member == primary {
                continue;
            }
            links.push(CausalLink {
                cause_finding_id: findings[primary].id.clone(),
                effect_finding_id: findings[*member].id.clone(),
                relationship: CausalRelationship::SharedResource,
            });
        }
    }

    links
}

fn select_primary(members: &[usize], relations: &[RawRelation], findings: &[Finding]) -> usize {
    let mut incoming: HashMap<usize, usize> = HashMap::new();
    let mut outgoing: HashMap<usize, usize> = HashMap::new();

    for relation in relations {
        if let Some((cause, effect)) = relation.directed {
            *outgoing.entry(cause).or_insert(0) += 1;
            *incoming.entry(effect).or_insert(0) += 1;
        }
    }

    members
        .iter()
        .copied()
        .max_by_key(|index| {
            (
                outgoing.get(index).copied().unwrap_or(0) as i64
                    - incoming.get(index).copied().unwrap_or(0) as i64,
                finding_rank(&findings[*index]),
            )
        })
        .unwrap_or(members[0])
}

fn finding_rank(finding: &Finding) -> (u8, u8, usize) {
    let severity = match finding.severity {
        Severity::Critical => 3,
        Severity::Warning => 2,
        Severity::Info => 1,
    };
    let domain = root_cause_priority(finding.category);
    (severity, domain, finding.evidence.len())
}

fn root_cause_priority(domain: MonitorDomain) -> u8 {
    match domain {
        MonitorDomain::Disks => 16,
        MonitorDomain::Load => 15,
        MonitorDomain::Network => 14,
        MonitorDomain::Processes => 13,
        MonitorDomain::Firewall => 12,
        MonitorDomain::Containers => 11,
        MonitorDomain::Compose => 10,
        MonitorDomain::Kubernetes => 9,
        MonitorDomain::Services => 8,
        MonitorDomain::Ports => 7,
        MonitorDomain::Logs => 6,
        MonitorDomain::Certs => 5,
        MonitorDomain::Backups => 4,
        MonitorDomain::Packages => 3,
        MonitorDomain::Timers => 2,
        MonitorDomain::Libvirt => 1,
    }
}

fn normalize_resource(resource: &str) -> String {
    resource
        .trim()
        .trim_matches('"')
        .trim_matches('`')
        .to_ascii_lowercase()
}

fn shared_resource(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right || left.contains(right) || right.contains(left)
}

fn build_group_id(members: &[usize], findings: &[Finding]) -> String {
    use std::hash::{Hash, Hasher};

    let mut fingerprints = members
        .iter()
        .map(|index| findings[*index].fingerprint())
        .collect::<Vec<_>>();
    fingerprints.sort();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for fingerprint in fingerprints {
        fingerprint.hash(&mut hasher);
    }
    format!("corr-{:016x}", hasher.finish())
}

fn find(parent: &mut [usize], index: usize) -> usize {
    if parent[index] != index {
        parent[index] = find(parent, parent[index]);
    }
    parent[index]
}

fn union(parent: &mut [usize], left: usize, right: usize) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root != right_root {
        parent[right_root] = left_root;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        findings::{Confidence, Severity},
        snapshot::{HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot},
    };
    use chrono::TimeZone;

    fn snapshot_at(second: i64) -> SystemSnapshot {
        SystemSnapshot {
            id: format!("snap-{second}"),
            host: HostIdentity::default(),
            collected_at: Utc.timestamp_opt(second, 0).unwrap(),
            profile: MonitorProfile::Standard,
            domains: SnapshotDomains::default(),
            collector_errors: Vec::new(),
            redaction_version: "test".to_owned(),
        }
    }

    fn finding(
        snapshot_id: &str,
        detector_id: &str,
        resource: &str,
        title: &str,
        severity: Severity,
        domain: MonitorDomain,
    ) -> Finding {
        Finding::new(
            snapshot_id,
            detector_id,
            resource,
            title,
            severity,
            Confidence::High,
            domain,
        )
    }

    #[test]
    fn groups_shared_resources_under_single_primary() {
        let snapshot = snapshot_at(100);
        let findings = vec![
            finding(
                &snapshot.id,
                "disk-usage",
                "/var",
                "Disk /var is full",
                Severity::Critical,
                MonitorDomain::Disks,
            ),
            finding(
                &snapshot.id,
                "filesystem-readonly",
                "/var",
                "/var is mounted read-only",
                Severity::Warning,
                MonitorDomain::Disks,
            ),
        ];

        let mut engine = CorrelationEngine::default();
        let result = engine.correlate(&snapshot, &findings);

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].finding_ids.len(), 2);
        assert_eq!(
            result.findings[0].root_cause_finding_id,
            Some(result.groups[0].primary_finding_id.clone())
        );
        assert_eq!(result.findings[0].correlated_finding_ids.len(), 1);
        assert_eq!(result.findings[1].correlated_finding_ids.len(), 1);
    }

    #[test]
    fn detects_dependency_cascades() {
        let snapshot = snapshot_at(200);
        let findings = vec![
            finding(
                &snapshot.id,
                "unhealthy-container",
                "payments_api_1",
                "payments_api_1 is unhealthy",
                Severity::Critical,
                MonitorDomain::Containers,
            ),
            finding(
                &snapshot.id,
                "compose-health",
                "payments",
                "Compose project payments is degraded",
                Severity::Warning,
                MonitorDomain::Compose,
            ),
        ];

        let mut engine = CorrelationEngine::default();
        engine.add_dependency("payments_api_1", "payments");
        let result = engine.correlate(&snapshot, &findings);

        assert_eq!(result.groups.len(), 1);
        assert!(result.groups[0].causal_links.iter().any(|link| {
            link.relationship == CausalRelationship::ResourceDependency
                && link.cause_finding_id == result.findings[0].id
                && link.effect_finding_id == result.findings[1].id
        }));
        assert_eq!(result.groups[0].primary_finding_id, result.findings[0].id);
    }

    #[test]
    fn detects_temporal_precedence_after_repeated_cycles() {
        let mut engine = CorrelationEngine::new(Duration::from_secs(900));
        engine.set_min_temporal_confirmations(2);

        let disk_1 = snapshot_at(300);
        let service_1 = snapshot_at(360);
        let disk_2 = snapshot_at(420);
        let service_2 = snapshot_at(480);
        let current = snapshot_at(540);

        let disk_finding = finding(
            &disk_1.id,
            "disk-usage",
            "/var",
            "Disk /var is full",
            Severity::Critical,
            MonitorDomain::Disks,
        );
        let service_finding = finding(
            &service_1.id,
            "failed-service",
            "nginx.service",
            "nginx.service has failed",
            Severity::Warning,
            MonitorDomain::Services,
        );

        let _ = engine.correlate(&disk_1, std::slice::from_ref(&disk_finding));
        let _ = engine.correlate(&service_1, std::slice::from_ref(&service_finding));
        let _ = engine.correlate(&disk_2, std::slice::from_ref(&disk_finding));
        let _ = engine.correlate(&service_2, std::slice::from_ref(&service_finding));

        let current_findings = vec![
            finding(
                &current.id,
                "disk-usage",
                "/var",
                "Disk /var is full",
                Severity::Critical,
                MonitorDomain::Disks,
            ),
            finding(
                &current.id,
                "failed-service",
                "nginx.service",
                "nginx.service has failed",
                Severity::Warning,
                MonitorDomain::Services,
            ),
        ];
        let result = engine.correlate(&current, &current_findings);

        assert_eq!(result.groups.len(), 1);
        assert!(result.groups[0].causal_links.iter().any(|link| {
            link.relationship == CausalRelationship::TemporalPrecedence
                && link.cause_finding_id == result.findings[0].id
                && link.effect_finding_id == result.findings[1].id
        }));
    }

    #[test]
    fn property_correlation_group_correctness_for_shared_resources() {
        for group_size in 2..=6 {
            let snapshot = snapshot_at(1_000 + group_size as i64);
            let findings = (0..group_size)
                .map(|index| {
                    finding(
                        &snapshot.id,
                        &format!("disk-{index}"),
                        "/srv/data",
                        &format!("Shared resource finding {index}"),
                        if index == 0 {
                            Severity::Critical
                        } else {
                            Severity::Warning
                        },
                        MonitorDomain::Disks,
                    )
                })
                .collect::<Vec<_>>();

            let mut engine = CorrelationEngine::default();
            let result = engine.correlate(&snapshot, &findings);

            assert_eq!(result.groups.len(), 1);
            assert_eq!(result.groups[0].finding_ids.len(), group_size as usize);
            assert_eq!(result.groups[0].primary_finding_id, result.findings[0].id);
            for finding in &result.findings {
                assert_eq!(
                    finding.correlation_id,
                    Some(result.groups[0].id.clone()),
                    "all members should share the same correlation id"
                );
                assert_eq!(
                    finding.root_cause_finding_id,
                    Some(result.groups[0].primary_finding_id.clone())
                );
                assert_eq!(
                    finding.correlated_finding_ids.len(),
                    group_size as usize - 1,
                    "each member should reference the rest of the group"
                );
            }
        }
    }

    #[test]
    fn property_resource_dependency_cascade_across_generated_graphs() {
        for width in 1..=4 {
            let snapshot = snapshot_at(2_000 + width as i64);
            let mut engine = CorrelationEngine::default();
            let mut findings = Vec::new();

            for index in 0..width {
                let root = format!("container-{index}");
                let dependent = format!("service-{index}");
                engine.add_dependency(&root, &dependent);
                findings.push(finding(
                    &snapshot.id,
                    &format!("container-root-{index}"),
                    &root,
                    &format!("{root} is unhealthy"),
                    Severity::Critical,
                    MonitorDomain::Containers,
                ));
                findings.push(finding(
                    &snapshot.id,
                    &format!("service-symptom-{index}"),
                    &dependent,
                    &format!("{dependent} is degraded"),
                    Severity::Warning,
                    MonitorDomain::Services,
                ));
            }

            let result = engine.correlate(&snapshot, &findings);
            assert_eq!(result.groups.len(), width as usize);
            for index in 0..width as usize {
                let group = &result.groups[index];
                assert_eq!(group.finding_ids.len(), 2);
                assert!(
                    group.causal_links.iter().any(|link| {
                        link.relationship == CausalRelationship::ResourceDependency
                    })
                );
            }
        }
    }

    #[test]
    fn property_temporal_correlation_detection_across_generated_sequences() {
        for cycle in 0..4 {
            let mut engine = CorrelationEngine::new(Duration::from_secs(900));
            engine.set_min_temporal_confirmations(2);

            for repetition in 0..2 {
                let cause_snapshot = snapshot_at(3_000 + cycle * 100 + repetition * 20);
                let effect_snapshot = snapshot_at(3_010 + cycle * 100 + repetition * 20);
                let cause = finding(
                    &cause_snapshot.id,
                    "disk-usage",
                    "/var",
                    "Disk /var is full",
                    Severity::Critical,
                    MonitorDomain::Disks,
                );
                let effect = finding(
                    &effect_snapshot.id,
                    "failed-service",
                    &format!("service-{cycle}"),
                    "Dependent service has failed",
                    Severity::Warning,
                    MonitorDomain::Services,
                );
                let _ = engine.correlate(&cause_snapshot, std::slice::from_ref(&cause));
                let _ = engine.correlate(&effect_snapshot, std::slice::from_ref(&effect));
            }

            let current = snapshot_at(3_100 + cycle * 100);
            let findings = vec![
                finding(
                    &current.id,
                    "disk-usage",
                    "/var",
                    "Disk /var is full",
                    Severity::Critical,
                    MonitorDomain::Disks,
                ),
                finding(
                    &current.id,
                    "failed-service",
                    &format!("service-{cycle}"),
                    "Dependent service has failed",
                    Severity::Warning,
                    MonitorDomain::Services,
                ),
            ];

            let result = engine.correlate(&current, &findings);
            assert_eq!(result.groups.len(), 1);
            assert!(
                result.groups[0]
                    .causal_links
                    .iter()
                    .any(|link| { link.relationship == CausalRelationship::TemporalPrecedence })
            );
        }
    }
}
