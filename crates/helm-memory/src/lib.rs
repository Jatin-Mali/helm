//! SQLite-backed episode memory store for HELM.

pub mod changesets;
pub mod episodes;
pub mod finding_state;
pub mod formal_skill;
pub mod graph;
pub mod merge_episodes;
pub mod procedures;
pub mod sessions;
pub mod skill_learner;
pub mod skills;
pub mod snapshots;
pub mod user_profile;

pub use changesets::{
    ChangeSetBackupRecord, ChangeSetRecord, ChangeSetStepRecord, ChangeSetStore, FullChangeSet,
    TroubleshootingPlanRecord, TroubleshootingPlanStore,
};
pub use episodes::{
    AuditEventInput, AuditEventRecord, AuditHashParts, AuditVerification, CapabilityGrantRecord,
    EpisodeId, EpisodeOutcome, EpisodeOutcomeCounts, EpisodeRecord, MemoryStore, RoutingStat,
    StepRecord, StepRole, audit_hash, latest_audit_hash, stable_hash_hex,
};
pub use finding_state::{FindingStateRecord, FindingStateStatus, FindingStateStore};
pub use formal_skill::{
    FormalSkill, FormalSkillError, Prerequisites, SkillDescription, SkillMeta, SkillMetadata,
};
pub use graph::{Entity, EntityGraph, GraphError, Relation};
pub use helm_core::MemoryError;
pub use merge_episodes::{ConflictResolution, MergeConflict, MergeResult};
pub use procedures::{Procedure, ProcedureError, ProcedureStep, ProcedureStore};
pub use sessions::{SessionRecord, SessionStore, SnapshotRecord};
pub use skill_learner::{LearnResult, SkillLearner, SkillLearnerError, SkillMatch};
pub use skills::{Skill, SkillError, SkillsManager};
pub use snapshots::{SnapshotRecord as SnapshotStoreRecord, SnapshotStore};
pub use user_profile::{Role, UserPreferences, UserProfileError, UserProfileStore, Verbosity};
