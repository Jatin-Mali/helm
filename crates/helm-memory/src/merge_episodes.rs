//! Episode merge and conflict resolution.

use serde::{Deserialize, Serialize};

/// Result of merging two episodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    /// ID of the merged episode.
    pub merged_episode_id: String,
    /// Conflicts that were encountered.
    pub conflicts: Vec<MergeConflict>,
    /// Number of auto-merged steps.
    pub auto_merged_count: u32,
    /// Number of conflicting steps.
    pub conflict_count: u32,
}

/// A single conflict between two episodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConflict {
    /// Step index where conflict occurred.
    pub step_index: u32,
    /// Tool name or action in first episode.
    pub ep1_action: String,
    /// Tool name or action in second episode.
    pub ep2_action: String,
    /// How this conflict was resolved.
    pub resolution: ConflictResolution,
}

/// Strategy for resolving a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Keep the first episode's version.
    KeepFirst,
    /// Keep the second episode's version.
    KeepSecond,
    /// Keep both versions (sequential).
    KeepBoth,
    /// Unresolved — requires user intervention.
    Unresolved,
}
