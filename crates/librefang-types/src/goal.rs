//! Hierarchical goal types for the LibreFang Goals system.
//!
//! Goals represent high-level objectives that agents work toward.
//! They support parent-child hierarchies for organizing complex objectives
//! into smaller, trackable sub-goals.

use crate::agent::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// GoalId
// ---------------------------------------------------------------------------

/// Unique identifier for a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(pub Uuid);

impl GoalId {
    /// Generate a new random GoalId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for GoalId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for GoalId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

// ---------------------------------------------------------------------------
// GoalStatus
// ---------------------------------------------------------------------------

/// The current status of a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Not yet started.
    Pending,
    /// Currently being worked on.
    InProgress,
    /// Successfully completed.
    Completed,
    /// Cancelled or abandoned.
    Cancelled,
}

impl std::fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalStatus::Pending => write!(f, "pending"),
            GoalStatus::InProgress => write!(f, "in_progress"),
            GoalStatus::Completed => write!(f, "completed"),
            GoalStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

// ---------------------------------------------------------------------------
// Goal
// ---------------------------------------------------------------------------

/// Maximum title length in characters.
///
/// Public so that the surfaces which mint goals without going through
/// [`Goal::validate`] (the `/goal` chat command) truncate at the same bound
/// the validator enforces, instead of hard-coding a second copy of it.
pub const MAX_TITLE_LEN: usize = 256;

/// Maximum description length in characters.
pub const MAX_DESCRIPTION_LEN: usize = 4096;

/// A hierarchical goal that agents work toward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    /// Unique goal identifier.
    pub id: GoalId,
    /// Short title for the goal (max 256 chars).
    pub title: String,
    /// Longer description of the goal (max 4096 chars).
    #[serde(default)]
    pub description: String,
    /// Optional parent goal ID for hierarchy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<GoalId>,
    /// Current status of the goal.
    pub status: GoalStatus,
    /// Progress percentage (0-100).
    #[serde(default)]
    pub progress: u8,
    /// Optional agent assigned to this goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Enable loop engineering mode: verifier, auto-sub-agent spawning,
    /// GOAL_LEARNED memory, and auto-skill-creation. Default false —
    /// basic goal loop when off (upstream behavior).
    #[serde(default)]
    pub loop_engineering: bool,
    /// Optional verifier agent that judges output quality after each
    /// iteration. Only active when `loop_engineering` is true. Part of
    /// the Loop Engineering pattern: "Never let an agent grade its own
    /// work."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_agent_id: Option<AgentId>,
    /// Optional evaluator model name for goal completion judgment.
    /// When set, the goal runner uses this model (e.g. "haiku", "deepseek-v4-pro")
    /// to evaluate if the goal is met. When None, defaults to the agent's model.
    /// Claude Code /goal uses Haiku as the evaluator — cheap and objective.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_model: Option<String>,
    /// Seconds the runner waits between iterations of this goal's loop.
    ///
    /// `None` uses [`DEFAULT_GOAL_TICK_INTERVAL_SECS`], which is the value the
    /// cadence was hard-wired to before it became configurable. Clamped into
    /// `[MIN_GOAL_TICK_INTERVAL_SECS, MAX_GOAL_TICK_INTERVAL_SECS]` by
    /// [`clamp_goal_tick_interval_secs`] at run start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_interval_secs: Option<u32>,
    /// When the goal was created.
    pub created_at: DateTime<Utc>,
    /// When the goal was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Goal {
    /// Validate this goal's fields.
    pub fn validate(&self) -> Result<(), String> {
        if self.title.is_empty() {
            return Err("title must not be empty".into());
        }
        if self.title.chars().count() > MAX_TITLE_LEN {
            return Err(format!(
                "title too long ({} chars, max {MAX_TITLE_LEN})",
                self.title.chars().count()
            ));
        }
        if self.description.chars().count() > MAX_DESCRIPTION_LEN {
            return Err(format!(
                "description too long ({} chars, max {MAX_DESCRIPTION_LEN})",
                self.description.chars().count()
            ));
        }
        if self.progress > 100 {
            return Err(format!("progress must be 0-100, got {}", self.progress));
        }
        if let Some(secs) = self.tick_interval_secs {
            if !(MIN_GOAL_TICK_INTERVAL_SECS..=MAX_GOAL_TICK_INTERVAL_SECS).contains(&secs) {
                return Err(format!(
                    "tick_interval_secs must be {MIN_GOAL_TICK_INTERVAL_SECS}-{MAX_GOAL_TICK_INTERVAL_SECS}, got {secs}"
                ));
            }
        }
        Ok(())
    }
}

/// Seconds the goal runner waits between loop iterations when the goal does
/// not override it. Preserves the cadence the runner used while the value was
/// a hard-wired `TICK_INTERVAL` constant.
pub const DEFAULT_GOAL_TICK_INTERVAL_SECS: u32 = 2;

/// Floor for a goal's configurable cadence.
///
/// Deliberately far below cron's `MIN_EVERY_SECS` of 60
/// (`crate::scheduler`): a goal tick is a live in-process loop that evaluates
/// its own stop condition each round, not a scheduler poll, and the pre-
/// existing default of 2s has to remain a legal value — so the floor cannot
/// exceed 2. It is 1 rather than 0 because 0 removes the only gap between
/// consecutive provider calls, turning the run into a tight loop that bills
/// tokens as fast as the provider will answer. One second is the smallest
/// value that still yields between turns.
pub const MIN_GOAL_TICK_INTERVAL_SECS: u32 = 1;

/// Ceiling for a goal's configurable cadence, matching cron's own
/// `MAX_EVERY_SECS` (24 hours) so the two recurring-work surfaces agree on how
/// far apart repetitions may legitimately sit. Past that horizon the work
/// belongs in a cron job, which survives a daemon restart without holding a
/// live task open.
pub const MAX_GOAL_TICK_INTERVAL_SECS: u32 = 86_400;

/// Resolve a goal's requested cadence into the seconds the runner will sleep.
///
/// `None` yields the default. Out-of-range values are clamped rather than
/// rejected — the runner must never refuse to start a goal over a cadence it
/// can correct, matching how the kernel treats `max_history_messages`. Callers
/// that want to reject instead (the HTTP layer, so the operator sees the
/// mistake) validate up front via [`Goal::validate`]. Returns the resolved
/// seconds and whether clamping occurred, so the caller can log it.
pub fn clamp_goal_tick_interval_secs(requested: Option<u32>) -> (u32, bool) {
    match requested {
        None => (DEFAULT_GOAL_TICK_INTERVAL_SECS, false),
        Some(secs) => {
            let clamped = secs.clamp(MIN_GOAL_TICK_INTERVAL_SECS, MAX_GOAL_TICK_INTERVAL_SECS);
            (clamped, clamped != secs)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared storage location
// ---------------------------------------------------------------------------

/// Well-known shared-memory key under which all goals are persisted as a
/// single JSON array. Shared by the API CRUD routes and the kernel-side goal
/// runner so both read and write the same store.
pub const GOALS_STORAGE_KEY: &str = "__librefang_goals";

/// The reserved sentinel agent ID that owns the goals KV entry. Goals are a
/// global, cross-agent resource, so they live under a fixed ID rather than any
/// real agent's namespace.
pub fn goals_storage_agent_id() -> AgentId {
    AgentId(Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

/// The `/goal` flag that switches the run into loop-engineering mode.
pub const GOAL_LOOP_ENGINEERING_FLAG: &str = "--loop-engineering";

/// Split a raw `/goal` argument string into `(description, loop_engineering)`.
///
/// Lives here rather than on any one surface because all three chat surfaces —
/// the channel bridge, the dashboard WebSocket and the TUI chat runner — must
/// spell and strip the flag identically; a per-surface copy is how `/goal`
/// drifted apart in the first place (upstream #3355).
///
/// Whitespace is collapsed, so `--loop-engineering` removed from the middle of
/// a sentence does not leave a double space behind. Returns `None` when the
/// flag was the only thing supplied, which callers surface as a usage hint.
pub fn parse_goal_args(args: &str) -> Option<(String, bool)> {
    let loop_engineering = args.contains(GOAL_LOOP_ENGINEERING_FLAG);
    let description = if loop_engineering {
        args.replace(GOAL_LOOP_ENGINEERING_FLAG, " ")
    } else {
        args.to_string()
    };
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        None
    } else {
        Some((description, loop_engineering))
    }
}

// ---------------------------------------------------------------------------
// GoalRunState — long-horizon autonomous execution (#5744)
// ---------------------------------------------------------------------------

/// Lifecycle phase of an autonomous goal run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalRunPhase {
    /// The runner loop is active and driving the assigned agent.
    Running,
    /// The goal reached `Completed`/`Cancelled` (or 100% progress); loop ended.
    Finished,
    /// The iteration cap was hit before the goal completed.
    MaxIterationsReached,
    /// The loop stopped on the provider rate-limit circuit breaker.
    RateLimited,
    /// An operator cancelled the run. Terminal: the durable row is dropped, so
    /// a subsequent start begins from iteration 0.
    Stopped,
    /// An operator paused the run. Unlike [`GoalRunPhase::Stopped`] this is a
    /// resumable checkpoint: the durable row survives with the iteration count
    /// and progress reached, and a later start continues from there rather
    /// than restarting the goal.
    Paused,
}

impl GoalRunPhase {
    /// Whether this phase is a resumable checkpoint rather than a settled end.
    ///
    /// `Running` counts: a row left in `Running` on disk means the process died
    /// mid-run, and boot recovery resumes it from the last checkpoint.
    pub fn is_resumable(self) -> bool {
        matches!(self, GoalRunPhase::Paused | GoalRunPhase::Running)
    }
}

impl std::fmt::Display for GoalRunPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalRunPhase::Running => write!(f, "running"),
            GoalRunPhase::Finished => write!(f, "finished"),
            GoalRunPhase::MaxIterationsReached => write!(f, "max_iterations_reached"),
            GoalRunPhase::RateLimited => write!(f, "rate_limited"),
            GoalRunPhase::Stopped => write!(f, "stopped"),
            GoalRunPhase::Paused => write!(f, "paused"),
        }
    }
}

/// Parse the `phase` column the `goal_runs` table stores.
///
/// The store persists phases as the [`Display`](std::fmt::Display) string, and
/// reading a run back out of it (to surface a paused run whose in-memory task
/// has already exited) needs the inverse. Keeping both directions in one place
/// stops the round-trip from drifting when a variant is added.
impl std::str::FromStr for GoalRunPhase {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(GoalRunPhase::Running),
            "finished" => Ok(GoalRunPhase::Finished),
            "max_iterations_reached" => Ok(GoalRunPhase::MaxIterationsReached),
            "rate_limited" => Ok(GoalRunPhase::RateLimited),
            "stopped" => Ok(GoalRunPhase::Stopped),
            "paused" => Ok(GoalRunPhase::Paused),
            other => Err(format!("unknown goal run phase: {other}")),
        }
    }
}

/// Default per-run iteration cap when a start request omits one. Bounds a
/// long-horizon run so a goal the agent never marks done cannot loop forever.
pub const DEFAULT_GOAL_MAX_ITERATIONS: u32 = 25;

/// Observable state of a goal's autonomous run, surfaced via the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalRunState {
    /// The goal being pursued.
    pub goal_id: GoalId,
    /// The agent driving the goal.
    pub agent_id: AgentId,
    /// Current lifecycle phase.
    pub phase: GoalRunPhase,
    /// Number of completed iterations (agent turns) so far.
    pub iteration: u32,
    /// Iteration cap for this run.
    pub max_iterations: u32,
    /// Last progress value (0-100) observed from the agent.
    pub last_progress: u8,
    /// Last error message, if the most recent tick failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Optional verifier agent that judges output quality after each
    /// iteration. When set, the runner sends generator output to this
    /// agent for verification before accepting progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_agent_id: Option<AgentId>,
    /// Max verification retries per iteration before blocking.
    /// Default 0 (set at run-start, clamped to ≥1 by the runner).
    #[serde(default)]
    pub verify_max_retries: u32,
    /// Optional evaluator model name used for goal completion judgment.
    /// When set, the runner uses this model to evaluate if the goal is met.
    /// When None, defaults to the agent's model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_model: Option<String>,
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the most recent tick completed.
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_goal_args_extracts_description_and_flag() {
        assert_eq!(
            parse_goal_args("ship the release"),
            Some(("ship the release".to_string(), false))
        );
        assert_eq!(
            parse_goal_args("ship the release --loop-engineering"),
            Some(("ship the release".to_string(), true))
        );
    }

    /// The flag is stripped wherever it sits, and the gap it leaves is
    /// collapsed rather than baked into the stored description.
    #[test]
    fn parse_goal_args_collapses_whitespace_around_the_flag() {
        assert_eq!(
            parse_goal_args("ship   --loop-engineering   the release"),
            Some(("ship the release".to_string(), true))
        );
    }

    #[test]
    fn parse_goal_args_rejects_empty_and_flag_only_input() {
        assert_eq!(parse_goal_args(""), None);
        assert_eq!(parse_goal_args("   "), None);
        assert_eq!(parse_goal_args("--loop-engineering"), None);
    }

    fn valid_goal() -> Goal {
        Goal {
            id: GoalId::new(),
            title: "Ship v1.0".into(),
            description: "Release the first stable version".into(),
            parent_id: None,
            status: GoalStatus::Pending,
            progress: 0,
            agent_id: None,
            loop_engineering: false,
            verify_agent_id: None,
            evaluator_model: None,
            tick_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn goal_id_display_roundtrip() {
        let id = GoalId::new();
        let s = id.to_string();
        let parsed: GoalId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn goal_id_default() {
        let a = GoalId::default();
        let b = GoalId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn valid_goal_passes() {
        assert!(valid_goal().validate().is_ok());
    }

    #[test]
    fn empty_title_rejected() {
        let mut g = valid_goal();
        g.title = String::new();
        let err = g.validate().unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn long_title_rejected() {
        let mut g = valid_goal();
        g.title = "a".repeat(257);
        let err = g.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn long_description_rejected() {
        let mut g = valid_goal();
        g.description = "a".repeat(4097);
        let err = g.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn progress_over_100_rejected() {
        let mut g = valid_goal();
        g.progress = 101;
        let err = g.validate().unwrap_err();
        assert!(err.contains("0-100"), "{err}");
    }

    #[test]
    fn progress_100_ok() {
        let mut g = valid_goal();
        g.progress = 100;
        assert!(g.validate().is_ok());
    }

    #[test]
    fn serde_roundtrip() {
        let goal = valid_goal();
        let json = serde_json::to_string(&goal).unwrap();
        let back: Goal = serde_json::from_str(&json).unwrap();
        assert_eq!(back.title, goal.title);
        assert_eq!(back.id, goal.id);
    }

    #[test]
    fn serde_status_tags() {
        let json = serde_json::to_string(&GoalStatus::InProgress).unwrap();
        assert_eq!(json, "\"in_progress\"");

        let back: GoalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, GoalStatus::InProgress);
    }

    #[test]
    fn goal_with_parent() {
        let parent_id = GoalId::new();
        let mut g = valid_goal();
        g.parent_id = Some(parent_id);
        let json = serde_json::to_string(&g).unwrap();
        assert!(json.contains("parent_id"));
        let back: Goal = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parent_id, Some(parent_id));
    }

    #[test]
    fn goal_without_parent_omits_field() {
        let g = valid_goal();
        let json = serde_json::to_string(&g).unwrap();
        assert!(!json.contains("parent_id"));
    }

    // -----------------------------------------------------------------------
    // Configurable tick cadence
    // -----------------------------------------------------------------------

    /// An unset cadence must reproduce the value the runner used while it was
    /// a hard-wired constant, so upgrading changes nothing for existing goals.
    #[test]
    fn unset_tick_interval_resolves_to_the_historical_default() {
        assert_eq!(
            clamp_goal_tick_interval_secs(None),
            (DEFAULT_GOAL_TICK_INTERVAL_SECS, false)
        );
        assert_eq!(DEFAULT_GOAL_TICK_INTERVAL_SECS, 2);
    }

    /// A goal set to the default explicitly is not reported as clamped.
    #[test]
    fn in_range_tick_interval_passes_through_unclamped() {
        for secs in [
            MIN_GOAL_TICK_INTERVAL_SECS,
            DEFAULT_GOAL_TICK_INTERVAL_SECS,
            300,
            MAX_GOAL_TICK_INTERVAL_SECS,
        ] {
            assert_eq!(clamp_goal_tick_interval_secs(Some(secs)), (secs, false));
        }
    }

    /// Zero is the case the floor exists for: it would remove every gap
    /// between provider calls.
    #[test]
    fn zero_tick_interval_is_clamped_up_to_the_floor() {
        assert_eq!(
            clamp_goal_tick_interval_secs(Some(0)),
            (MIN_GOAL_TICK_INTERVAL_SECS, true)
        );
    }

    #[test]
    fn oversized_tick_interval_is_clamped_down_to_the_ceiling() {
        assert_eq!(
            clamp_goal_tick_interval_secs(Some(u32::MAX)),
            (MAX_GOAL_TICK_INTERVAL_SECS, true)
        );
    }

    #[test]
    fn out_of_range_tick_interval_fails_validation() {
        let mut g = valid_goal();
        g.tick_interval_secs = Some(0);
        assert!(g.validate().unwrap_err().contains("tick_interval_secs"));

        g.tick_interval_secs = Some(MAX_GOAL_TICK_INTERVAL_SECS + 1);
        assert!(g.validate().unwrap_err().contains("tick_interval_secs"));

        g.tick_interval_secs = Some(DEFAULT_GOAL_TICK_INTERVAL_SECS);
        assert!(g.validate().is_ok());
    }

    /// A goal document written before the field existed must still load.
    #[test]
    fn goal_without_tick_interval_deserializes() {
        let json = serde_json::json!({
            "id": GoalId::new(),
            "title": "legacy",
            "status": "pending",
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
        });
        let g: Goal = serde_json::from_value(json).unwrap();
        assert_eq!(g.tick_interval_secs, None);
    }

    // -----------------------------------------------------------------------
    // Run phases
    // -----------------------------------------------------------------------

    /// The store persists phases via `Display` and reads them back via
    /// `FromStr`; a variant that survives one direction but not the other
    /// silently turns a paused run into an unreadable row.
    #[test]
    fn every_run_phase_round_trips_through_its_stored_string() {
        for phase in [
            GoalRunPhase::Running,
            GoalRunPhase::Finished,
            GoalRunPhase::MaxIterationsReached,
            GoalRunPhase::RateLimited,
            GoalRunPhase::Stopped,
            GoalRunPhase::Paused,
        ] {
            let stored = phase.to_string();
            assert_eq!(stored.parse::<GoalRunPhase>().unwrap(), phase, "{stored}");
        }
    }

    #[test]
    fn unknown_phase_string_is_rejected() {
        assert!("wat".parse::<GoalRunPhase>().is_err());
    }

    /// Pause is resumable, cancel is not — that distinction is the whole point
    /// of having both.
    #[test]
    fn only_paused_and_running_are_resumable() {
        assert!(GoalRunPhase::Paused.is_resumable());
        assert!(GoalRunPhase::Running.is_resumable());
        assert!(!GoalRunPhase::Stopped.is_resumable());
        assert!(!GoalRunPhase::Finished.is_resumable());
        assert!(!GoalRunPhase::MaxIterationsReached.is_resumable());
        assert!(!GoalRunPhase::RateLimited.is_resumable());
    }
}
