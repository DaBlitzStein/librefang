//! Long-horizon autonomous goal execution (#5744).
//!
//! The Goals system (CRUD + dashboard) tracks objectives but, on its own, is
//! purely passive — nothing ever drives an agent toward a goal. The
//! [`GoalRunner`] closes that gap: starting a run for a goal with an assigned
//! agent spawns a bounded loop that repeatedly prompts the agent with the
//! goal's context and parses the agent's reply for progress / completion
//! markers, updating the goal in the shared memory store until the goal is
//! done, the iteration cap is hit, an operator stops it, or the kernel shuts
//! down.
//!
//! ## Why response markers instead of a tool
//!
//! The agent reports progress by ending its turn with structured lines:
//!
//! ```text
//! GOAL_PROGRESS: 60
//! GOAL_DONE          (optional — signals the goal is complete)
//! GOAL_BLOCKED       (optional — signals it cannot proceed without input)
//! ```
//!
//! This keeps the v1 runner entirely kernel-side: no new runtime tool, no
//! tool-registry / capability-permission surgery. The parsing is forgiving
//! (case-insensitive, last marker wins) so an agent that forgets the marker
//! simply keeps iterating to the cap rather than failing.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use librefang_memory::{GoalRunRow, GoalRunStore, MemorySubstrate};
use librefang_types::agent::AgentId;
use librefang_types::goal::{
    clamp_goal_tick_interval_secs, goals_storage_agent_id, Goal, GoalId, GoalRunPhase,
    GoalRunState, GoalStatus, GOALS_STORAGE_KEY, MAX_DESCRIPTION_LEN, MAX_TITLE_LEN,
};

use crate::background::{classify_tick_error, TickOutcome};
use crate::kernel_api::KernelApi;

/// Consecutive provider rate-limit ticks before the loop gives up, mirroring
/// the background executor's circuit breaker (#5168) so a quota-exhausted
/// provider does not get hammered on every iteration.
const MAX_RATE_LIMIT_STREAK: u32 = 3;
/// Consecutive non-rate-limit errors before the loop gives up. Prevents
/// burning all max_iterations on a permanently broken condition (wrong
/// API key, deleted agent, network down). Separate from the rate-limit
/// circuit breaker so transient rate-limits don't also count.
const MAX_ERROR_STREAK: u32 = 5;

/// Result of parsing one agent reply for goal-control markers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedTick {
    /// Progress value (0-100) if the agent emitted `GOAL_PROGRESS:`.
    pub progress: Option<u8>,
    /// The agent signalled completion (`GOAL_DONE`).
    pub done: bool,
    /// The agent signalled it is blocked (`GOAL_BLOCKED`).
    pub blocked: bool,
    /// The agent captured a learning (`GOAL_LEARNED: <text>`).
    pub learnings: Vec<String>,
}

/// Line prefix an agent uses to capture a reusable learning.
const LEARNED_MARKER: &str = "GOAL_LEARNED:";

/// Parse an agent reply for `GOAL_PROGRESS:` / `GOAL_DONE` / `GOAL_BLOCKED`
/// markers. Case-insensitive; the last `GOAL_PROGRESS` line wins.
pub fn parse_tick(reply: &str) -> ParsedTick {
    let mut out = ParsedTick::default();
    for line in reply.lines() {
        let t = line.trim();
        let upper = t.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("GOAL_PROGRESS:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                out.progress = Some(n.min(100) as u8);
            }
        } else if marker_present(&upper, "GOAL_DONE") || marker_present(&upper, "GOAL_COMPLETE") {
            out.done = true;
        } else if marker_present(&upper, "GOAL_BLOCKED") {
            out.blocked = true;
        } else if upper.starts_with(LEARNED_MARKER) {
            // Match the marker on the uppercased copy (case-insensitive, like
            // every other marker here) but slice the *original* line for the
            // payload. Slicing `upper` would persist every captured learning
            // in all caps — and those strings are stored in memory and fed
            // verbatim into auto-generated skills, so the damage outlives the
            // run. The byte offset is valid on `t` because `to_ascii_uppercase`
            // is length-preserving and only rewrites ASCII bytes.
            let learning = t[LEARNED_MARKER.len()..].trim();
            if !learning.is_empty() {
                out.learnings.push(learning.to_string());
            }
        }
    }
    out
}

/// Match `marker` as a standalone token at the start of `line`, not a bare
/// prefix. The marker counts only when the line begins with it AND the byte
/// immediately after is a word boundary — end-of-line, or any character that is
/// not a word-continuation char (i.e. not alphanumeric and not `_`). That
/// admits the bare form (`GOAL_DONE`), trailing punctuation the model tends to
/// add (`GOAL_DONE!`, `GOAL_DONE.`), and the trailing-note form the prompt
/// suggests (`GOAL_BLOCKED: need a key`), while still rejecting a longer
/// identifier that merely starts with the token (`GOAL_DONE_CRITERIA`,
/// `GOAL_DONENESS`). `line` is expected to be already uppercased.
fn marker_present(line: &str, marker: &str) -> bool {
    match line.strip_prefix(marker) {
        Some(rest) => rest
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_'),
        None => false,
    }
}

/// Outcome of a `/goal` launch.
///
/// The goal document is durable in both cases; `started` only reports whether
/// a run was also scheduled. Surfaces with their own message catalog (the TUI)
/// render from the fields; the rest use [`GoalLaunch::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalLaunch {
    /// Id of the goal that was persisted.
    pub goal_id: GoalId,
    /// Whether a run was scheduled for it.
    pub started: bool,
}

impl GoalLaunch {
    /// Confirmation text for surfaces without a localized message catalog
    /// (channel adapters, dashboard chat WebSocket).
    pub fn message(&self, description: &str) -> String {
        let goal_id = self.goal_id;
        if self.started {
            format!("Goal created and started: {description} (ID: {goal_id})")
        } else {
            format!(
                "Goal created (ID: {goal_id}) but the run could not start — \
                 kernel self-handle unset. Restart the daemon and resume the goal."
            )
        }
    }
}

/// Persist a goal for `agent_id` and immediately start a run for it.
///
/// Every chat surface that exposes `/goal` — the channel bridge, the dashboard
/// chat WebSocket and the TUI chat runner — goes through this one function, so
/// a goal created from Telegram is identical in shape to one created from the
/// dashboard (upstream #3355).
///
/// A failure to schedule the run is deliberately **not** an error: the goal
/// document is already durable at that point, so the operator has not lost the
/// request and the message says so instead of implying nothing was saved.
pub fn create_and_start_goal(
    kernel: &dyn KernelApi,
    agent_id: AgentId,
    description: &str,
    loop_engineering: bool,
) -> Result<GoalLaunch, String> {
    // Reject over-long input rather than persisting a document that
    // `Goal::validate` would refuse — a goal no later PUT can update is worse
    // than a rejected `/goal`.
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(format!(
            "Goal description too long ({} chars, max {MAX_DESCRIPTION_LEN})",
            description.chars().count()
        ));
    }

    let goal_id = GoalId::new();
    let now = Utc::now().to_rfc3339();
    let title: String = description.chars().take(MAX_TITLE_LEN).collect();
    let entry = serde_json::json!({
        "id": goal_id.to_string(),
        "title": title,
        "description": description,
        "status": GoalStatus::Pending.to_string(),
        "progress": 0,
        "agent_id": agent_id.to_string(),
        "loop_engineering": loop_engineering,
        "created_at": now,
        "updated_at": now,
    });

    kernel
        .memory_substrate()
        .structured_modify(goals_storage_agent_id(), GOALS_STORAGE_KEY, |current| {
            let mut goals: Vec<serde_json::Value> = match current {
                Some(serde_json::Value::Array(arr)) => arr,
                _ => Vec::new(),
            };
            goals.push(entry.clone());
            Ok((serde_json::Value::Array(goals), ()))
        })
        .map_err(|e| format!("Failed to create goal: {e}"))?;

    let started = kernel.start_goal_run(
        goal_id,
        agent_id,
        None, // max_iterations — use default
        loop_engineering,
        None, // verify_agent_id — the goals route auto-spawns one when needed
        None, // verify_max_retries — runner clamps None up to its minimum
        None, // evaluator_model — use agent default
        None, // tick_interval_secs — use the default cadence
    );
    Ok(GoalLaunch { goal_id, started })
}

/// Build the per-iteration prompt that frames the goal for the agent.
pub fn build_goal_prompt(
    goal: &Goal,
    iteration: u32,
    max_iterations: u32,
    has_verifier: bool,
    learnings: &[String],
) -> String {
    let loop_section = if has_verifier {
        "\n\n## Loop Engineering Mode\n\
         You are part of an autonomous loop (Generator + Verifier).\n\
         - Your output will be sent to a verifier agent for judgment.\n\
         - If the verifier rejects your work, you will retry automatically.\n\
         - For complex tasks, spawn sub-agents with `agent_spawn` and delegate with `agent_send`.\n\
         - Never grade your own work — the verifier is the final judge.\n\
         - When you discover something reusable (a pattern, a pitfall, a technique),\n\
           capture it with `GOAL_LEARNED: <one sentence>` so it persists in memory."
    } else {
        "\n\n## Autonomous Mode\n\
         For complex tasks, spawn sub-agents with `agent_spawn` and delegate with `agent_send`.\n\
         When you discover something reusable, capture it with `GOAL_LEARNED: <one sentence>`."
    };
    let learnings_block = if learnings.is_empty() {
        String::new()
    } else {
        let recent = learnings.iter().rev().take(6).cloned().collect::<Vec<_>>();
        format!(
            "\n\n## Memory (captured learnings from prior iterations)\n{}\n",
            recent
                .iter()
                .map(|l| format!("- {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "[LONG-HORIZON GOAL] You are autonomously pursuing a goal across multiple turns.\n\
         Goal: {title}\n\
         Description: {description}\n\
         Current progress: {progress}%\n\
         Iteration: {iter} of {max}\n\
         {learnings}\
         {loop}\n\
         Take the next concrete action toward completing this goal. When you finish a \
         step, end your reply with a line `GOAL_PROGRESS: <0-100>` reflecting overall \
         completion. Add a line `GOAL_DONE` once the goal is fully achieved, or \
         `GOAL_BLOCKED` if you cannot proceed without operator input.",
        title = goal.title,
        description = if goal.description.is_empty() {
            "(none)"
        } else {
            &goal.description
        },
        progress = goal.progress,
        iter = iteration + 1,
        max = max_iterations,
        learnings = learnings_block,
        loop = loop_section,
    )
}

/// Load the goal with `goal_id` from the shared goals store.
fn load_goal(substrate: &MemorySubstrate, goal_id: GoalId) -> Option<Goal> {
    let arr = match substrate.structured_get(goals_storage_agent_id(), GOALS_STORAGE_KEY) {
        Ok(Some(serde_json::Value::Array(arr))) => arr,
        _ => return None,
    };
    let target = goal_id.to_string();
    arr.into_iter()
        .find(|g| g.get("id").and_then(|v| v.as_str()) == Some(target.as_str()))
        .and_then(|v| serde_json::from_value(v).ok())
}

/// Atomically patch a goal's progress / status / `updated_at` in the shared
/// store. Uses `structured_modify` so concurrent writers (the API CRUD path)
/// never lose this update to a last-writer-wins race.
fn patch_goal(
    substrate: &MemorySubstrate,
    goal_id: GoalId,
    progress: Option<u8>,
    status: Option<GoalStatus>,
) {
    let target = goal_id.to_string();
    let res =
        substrate.structured_modify(goals_storage_agent_id(), GOALS_STORAGE_KEY, |existing| {
            let mut arr = match existing {
                Some(serde_json::Value::Array(arr)) => arr,
                _ => Vec::new(),
            };
            for g in arr.iter_mut() {
                if g.get("id").and_then(|v| v.as_str()) != Some(target.as_str()) {
                    continue;
                }
                if let Some(obj) = g.as_object_mut() {
                    if let Some(p) = progress {
                        obj.insert("progress".into(), serde_json::json!(p));
                    }
                    if let Some(s) = status {
                        obj.insert("status".into(), serde_json::json!(s.to_string()));
                    }
                    obj.insert("updated_at".into(), serde_json::json!(Utc::now()));
                }
                break;
            }
            Ok((serde_json::Value::Array(arr), ()))
        });
    if let Err(e) = res {
        warn!(goal_id = %goal_id, "Failed to persist goal update: {e}");
    }
}

/// Flatten a `GoalRunState` into the `goal_runs` row shape the store persists.
fn row_from_state(state: &GoalRunState) -> GoalRunRow {
    GoalRunRow {
        goal_id: state.goal_id.to_string(),
        agent_id: state.agent_id.to_string(),
        phase: state.phase.to_string(),
        iteration: state.iteration as i64,
        max_iterations: state.max_iterations as i64,
        last_progress: state.last_progress as i64,
        last_error: state.last_error.clone(),
        started_at: state.started_at.to_rfc3339(),
        updated_at: state.updated_at.to_rfc3339(),
    }
}

/// Mirror the live run state into the durable store. A persistence failure is
/// logged and swallowed — the in-memory DashMap stays the hot path, so a
/// transient DB hiccup must never abort or stall the run loop.
fn persist_run(store: &Option<GoalRunStore>, state: &GoalRunState) {
    let Some(store) = store else { return };
    if let Err(e) = store.save_run(&row_from_state(state)) {
        warn!(goal_id = %state.goal_id, "Failed to persist goal run state: {e}");
    }
}

/// Persist the first snapshot of a new run, replacing any durable predecessor
/// in one SQLite statement so a crash cannot land between delete and insert.
fn persist_new_run(store: &Option<GoalRunStore>, state: &GoalRunState) {
    let Some(store) = store else { return };
    if let Err(e) = store.start_run(&row_from_state(state)) {
        warn!(goal_id = %state.goal_id, "Failed to persist new goal run state: {e}");
    }
}

/// Drop the durable mirror once a run ends. Same failure policy as
/// [`persist_run`]: log and swallow.
fn delete_persisted_run(store: &Option<GoalRunStore>, goal_id: GoalId) {
    let Some(store) = store else { return };
    if let Err(e) = store.delete_run(&goal_id.to_string()) {
        warn!(goal_id = %goal_id, "Failed to delete persisted goal run: {e}");
    }
}

/// Shared-memory key holding a goal's captured `GOAL_LEARNED:` lines.
///
/// Written both when a run ends and when it is paused: learnings accumulate in
/// a local on the loop task's stack, so without this checkpoint a pause would
/// silently discard everything the agent had discovered so far and the resumed
/// run would re-derive it at full token cost.
fn goal_learnings_key(goal_id: GoalId) -> String {
    format!("goal_learnings_{goal_id}")
}

/// Checkpoint captured learnings so a resumed run can pick them back up.
fn persist_goal_learnings(substrate: &MemorySubstrate, goal_id: GoalId, learnings: &[String]) {
    if learnings.is_empty() {
        return;
    }
    if let Err(e) = substrate.structured_set(
        goals_storage_agent_id(),
        &goal_learnings_key(goal_id),
        serde_json::json!({
            "goal_id": goal_id.to_string(),
            "learnings": learnings,
            "captured_at": Utc::now().to_rfc3339(),
        }),
    ) {
        warn!(goal_id = %goal_id, error = %e, "Failed to persist goal learnings");
    } else {
        info!(goal_id = %goal_id, count = learnings.len(),
              "Persisted goal learnings to shared memory");
    }
}

/// Read back the learnings checkpointed by a prior run segment of this goal.
///
/// An unreadable or malformed entry yields an empty list rather than an error:
/// losing accumulated learnings degrades the resumed run's prompt, it must not
/// prevent the resume itself.
fn load_goal_learnings(substrate: &MemorySubstrate, goal_id: GoalId) -> Vec<String> {
    let Ok(Some(value)) =
        substrate.structured_get(goals_storage_agent_id(), &goal_learnings_key(goal_id))
    else {
        return Vec::new();
    };
    value
        .get("learnings")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The state a paused (or crash-interrupted) run hands to its successor.
///
/// Carrying this as one value keeps [`GoalRunner::start`] from silently
/// resuming half of it — an iteration count restored without the learnings, or
/// vice versa, is the failure mode that made "resume" cosmetic.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ResumePoint {
    iteration: u32,
    last_progress: u8,
    max_iterations: u32,
    learnings: Vec<String>,
}

/// Shared-memory key holding a paused run's resume checkpoint.
///
/// ## Why not the `goal_runs` table
///
/// That table is the durable mirror of *active* runs, and its schema pins
/// `phase` with `CHECK (phase IN ('running','finished','max_iterations_reached',
/// 'rate_limited','stopped'))` (`migration::migrate_v42`). Admitting a `paused`
/// row means rebuilding the table under a new migration version, because SQLite
/// cannot alter a CHECK constraint in place.
///
/// A paused run is by definition not an active run, so the mirror is the wrong
/// home for it regardless. The shared KV is where the goal-adjacent durable
/// state already lives — the goals array itself and the captured learnings —
/// and it holds the whole checkpoint as one value, which makes the pause write
/// atomic rather than a multi-column update that could tear.
fn goal_pause_key(goal_id: GoalId) -> String {
    format!("goal_pause_{goal_id}")
}

/// Write the checkpoint a paused run resumes from.
fn persist_pause_checkpoint(
    substrate: &MemorySubstrate,
    goal_id: GoalId,
    state: &GoalRunState,
    learnings: &[String],
) {
    if let Err(e) = substrate.structured_set(
        goals_storage_agent_id(),
        &goal_pause_key(goal_id),
        serde_json::json!({
            "goal_id": goal_id.to_string(),
            "agent_id": state.agent_id.to_string(),
            "iteration": state.iteration,
            "last_progress": state.last_progress,
            "max_iterations": state.max_iterations,
            "learnings": learnings,
            "paused_at": Utc::now().to_rfc3339(),
        }),
    ) {
        warn!(goal_id = %goal_id, error = %e,
              "Failed to persist goal pause checkpoint — resume will restart the goal");
    }
}

/// Read a paused run's checkpoint, if one is stored.
fn load_pause_checkpoint(substrate: &MemorySubstrate, goal_id: GoalId) -> Option<ResumePoint> {
    let value = substrate
        .structured_get(goals_storage_agent_id(), &goal_pause_key(goal_id))
        .ok()
        .flatten()?;
    if value.is_null() {
        return None;
    }
    Some(ResumePoint {
        iteration: value.get("iteration").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        last_progress: value
            .get("last_progress")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(100) as u8,
        max_iterations: value
            .get("max_iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        learnings: value
            .get("learnings")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The agent a paused checkpoint belongs to, for reconstructing its snapshot.
fn pause_checkpoint_agent(substrate: &MemorySubstrate, goal_id: GoalId) -> Option<AgentId> {
    substrate
        .structured_get(goals_storage_agent_id(), &goal_pause_key(goal_id))
        .ok()
        .flatten()?
        .get("agent_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
}

/// Drop a pause checkpoint once it has been consumed or cancelled.
///
/// Load-bearing: a checkpoint that outlives its pause would silently seed the
/// *next* fresh start of the same goal with a stale iteration count, which is
/// the same class of bug as the one resume exists to fix, only inverted.
fn clear_pause_checkpoint(substrate: &MemorySubstrate, goal_id: GoalId) {
    if let Err(e) = substrate.structured_delete(goals_storage_agent_id(), &goal_pause_key(goal_id))
    {
        warn!(goal_id = %goal_id, error = %e, "Failed to clear goal pause checkpoint");
    }
}

/// Read the resume point a prior run segment left behind, if any.
///
/// Two sources, in priority order:
/// 1. An operator pause checkpoint in the shared KV — the explicit case.
/// 2. A `goal_runs` row still in `Running` phase, meaning the process driving
///    it died. Its learnings come from the separate key the loop writes on exit.
///
/// A terminal row means the run settled and a fresh start legitimately begins
/// at iteration 0 — but the run loop deletes terminal rows, so in practice a
/// row on disk is already the crash-interrupted kind.
fn read_resume_point(
    store: &Option<GoalRunStore>,
    substrate: &MemorySubstrate,
    goal_id: GoalId,
) -> Option<ResumePoint> {
    if let Some(checkpoint) = load_pause_checkpoint(substrate, goal_id) {
        return Some(checkpoint);
    }
    let row = store
        .as_ref()?
        .get_run(&goal_id.to_string())
        .ok()
        .flatten()?;
    let phase = row.phase.parse::<GoalRunPhase>().ok()?;
    if !phase.is_resumable() {
        return None;
    }
    Some(ResumePoint {
        iteration: row.iteration.clamp(0, u32::MAX as i64) as u32,
        last_progress: row.last_progress.clamp(0, 100) as u8,
        max_iterations: row.max_iterations.clamp(0, u32::MAX as i64) as u32,
        learnings: load_goal_learnings(substrate, goal_id),
    })
}

/// A single goal run entry: the spawned loop task plus its observable state
/// and a cooperative stop flag.
struct RunHandle {
    /// The spawned loop task. `None` for a terminal entry reconstructed at boot
    /// by [`GoalRunner::recover_stale_runs`] — that run's process already died,
    /// so there is no live loop to abort; the entry exists only so the demoted
    /// `Stopped` state stays observable via [`GoalRunner::state`].
    task: Option<JoinHandle<()>>,
    state: Arc<Mutex<GoalRunState>>,
    stop: Arc<AtomicBool>,
    /// Cooperative pause flag. Distinct from `stop` because the two mean
    /// opposite things to the durable row: `stop` deletes it, `pause`
    /// checkpoints it. The loop must therefore be able to tell which one it
    /// woke up to.
    pause: Arc<AtomicBool>,
    /// Monotonic id for this run, used by the task's self-cleanup so it only
    /// removes its OWN registry entry — never a newer run that replaced it.
    generation: u64,
}

/// Registry + driver for autonomous goal runs. One [`GoalRunner`] lives on the
/// kernel; it tracks at most one active run per goal.
pub struct GoalRunner {
    runs: Arc<DashMap<GoalId, RunHandle>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Source of monotonic run generations (see [`RunHandle::generation`]).
    next_gen: Arc<AtomicU64>,
    /// Durable mirror of active run state (#5744 follow-up). `None` when the
    /// runner is constructed without persistence (e.g. unit tests that drive
    /// `run_loop` directly); the in-memory DashMap remains the hot path either
    /// way.
    store: Option<GoalRunStore>,
    /// Shared-memory handle for pause checkpoints.
    ///
    /// Held at construction rather than taken from `start`'s argument, because
    /// `stop` and `state` must reach a checkpoint written by a *previous
    /// process* — a goal paused before a daemon restart has to be cancellable
    /// and observable without anyone calling `start` first.
    substrate: Option<Arc<MemorySubstrate>>,
    /// Serializes the compound `start()` / `stop()` sequences for one goal so a
    /// concurrent `start()` cannot observe an empty registry slot between an
    /// in-flight `start()`'s stop and its insert and spawn a second, orphaned
    /// loop. The per-generation self-cleanup guard only protects the sequential
    /// replace path; it does nothing for two `start()` calls racing on the same
    /// goal id. The guarded region is fully synchronous (no `.await`), so this
    /// std `Mutex` is never held across an await point.
    start_lock: std::sync::Mutex<()>,
}

impl GoalRunner {
    /// Create a runner wired to the kernel shutdown signal, without durable
    /// persistence. Used where no memory substrate is available.
    pub fn new(shutdown_rx: watch::Receiver<bool>) -> Self {
        Self {
            runs: Arc::new(DashMap::new()),
            shutdown_rx,
            next_gen: Arc::new(AtomicU64::new(0)),
            store: None,
            substrate: None,
            start_lock: std::sync::Mutex::new(()),
        }
    }

    /// Create a runner backed by a [`GoalRunStore`] so active runs survive a
    /// daemon restart. Boot wires this with the shared memory connection pool.
    pub fn new_with_store(
        shutdown_rx: watch::Receiver<bool>,
        store: GoalRunStore,
        substrate: Arc<MemorySubstrate>,
    ) -> Self {
        Self {
            runs: Arc::new(DashMap::new()),
            shutdown_rx,
            next_gen: Arc::new(AtomicU64::new(0)),
            store: Some(store),
            substrate: Some(substrate),
            start_lock: std::sync::Mutex::new(()),
        }
    }

    /// Snapshot the observable state of a goal's run, if one exists.
    ///
    /// Falls back to the durable store when the registry has no live entry. A
    /// paused run's loop task exits and self-cleans its registry slot, so
    /// without this fallback pausing a goal would make it vanish from
    /// `GET /api/goals/{id}/run` entirely — an operator could pause work and
    /// then have no way to see that they had. The same fallback surfaces a run
    /// left paused across a daemon restart.
    pub fn state(&self, goal_id: GoalId) -> Option<GoalRunState> {
        if let Some(handle) = self.runs.get(&goal_id) {
            // try_lock: None → `running:false`; run_loop must never hold this lock across I/O.
            if let Ok(s) = handle.state.try_lock() {
                return Some(s.clone());
            }
            return None;
        }
        self.persisted_state(goal_id)
    }

    /// Reconstruct a run snapshot from durable state — the pause checkpoint
    /// first, then the `goal_runs` row.
    ///
    /// `verify_agent_id` / `verify_max_retries` / `evaluator_model` are not
    /// persisted with the run; they live on the `Goal` document and are re-read
    /// from there at the next start, so they come back as their defaults here
    /// rather than being invented.
    fn persisted_state(&self, goal_id: GoalId) -> Option<GoalRunState> {
        let parse_ts = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        };

        if let Some(substrate) = self.substrate.as_ref() {
            if let Some(checkpoint) = load_pause_checkpoint(substrate, goal_id) {
                let agent_id = pause_checkpoint_agent(substrate, goal_id)?;
                let now = Utc::now();
                return Some(GoalRunState {
                    goal_id,
                    agent_id,
                    phase: GoalRunPhase::Paused,
                    iteration: checkpoint.iteration,
                    max_iterations: checkpoint.max_iterations,
                    last_progress: checkpoint.last_progress,
                    last_error: None,
                    verify_agent_id: None,
                    verify_max_retries: 0,
                    evaluator_model: None,
                    started_at: now,
                    updated_at: now,
                });
            }
        }

        let store = self.store.as_ref()?;
        let row = store.get_run(&goal_id.to_string()).ok().flatten()?;
        let phase = row.phase.parse::<GoalRunPhase>().ok()?;
        Some(GoalRunState {
            goal_id,
            agent_id: row.agent_id.parse().ok()?,
            phase,
            iteration: row.iteration.clamp(0, u32::MAX as i64) as u32,
            max_iterations: row.max_iterations.clamp(0, u32::MAX as i64) as u32,
            last_progress: row.last_progress.clamp(0, 100) as u8,
            last_error: row.last_error.clone(),
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: parse_ts(&row.started_at),
            updated_at: parse_ts(&row.updated_at),
        })
    }

    /// Pause a goal's run, preserving everything needed to resume it.
    ///
    /// Returns whether a live run was signalled. The loop finishes the turn it
    /// is on, checkpoints its iteration count, progress and accumulated
    /// learnings, and exits in [`GoalRunPhase::Paused`]; a later
    /// [`GoalRunner::start`] picks up from that checkpoint.
    ///
    /// Deliberately does NOT abort the task the way [`GoalRunner::stop`] does.
    /// Aborting drops the loop's stack mid-turn, and the accumulated learnings
    /// live there — killing the task is precisely how a "pause" ends up losing
    /// the work it was supposed to protect. The cost is latency: the pause
    /// lands after the in-flight agent turn plus at most one tick interval.
    pub fn pause(&self, goal_id: GoalId) -> bool {
        let _guard = self
            .start_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(handle) = self.runs.get(&goal_id) else {
            return false;
        };
        // A recovered terminal entry has no loop to signal.
        if handle.task.is_none() {
            return false;
        }
        handle.pause.store(true, Ordering::SeqCst);
        info!(goal_id = %goal_id, "Goal run pause requested");
        true
    }

    /// Stop a goal's run if active. Returns whether a run was stopped.
    ///
    /// An operator stop is a terminal boundary, so the durable mirror is
    /// dropped too — a stopped run must not be resurrected as "stale" at the
    /// next boot.
    pub fn stop(&self, goal_id: GoalId) -> bool {
        // Serialize against `start()` so the two never interleave on the same
        // goal id. The critical section is synchronous, so this std guard never
        // spans an await point. Poison is irrelevant for a `Mutex<()>` used
        // purely for mutual exclusion — recover the guard rather than panic.
        let _guard = self
            .start_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.stop_locked(goal_id)
    }

    /// Stop body assuming the caller already holds `start_lock`. Split out so
    /// `start()` can run it inside its own critical section without re-locking
    /// the non-reentrant `start_lock` (which would deadlock).
    fn stop_locked(&self, goal_id: GoalId) -> bool {
        // Cancelling discards the resume checkpoint, whether or not a loop is
        // live: a goal paused before a daemon restart has no registry entry and
        // no `goal_runs` row, so the checkpoint is the ONLY thing cancel has to
        // remove. Leaving it would make the next start silently resume a run
        // the operator cancelled.
        let had_checkpoint = match self.substrate.as_ref() {
            Some(substrate) => {
                let existed = load_pause_checkpoint(substrate, goal_id).is_some();
                if existed {
                    clear_pause_checkpoint(substrate, goal_id);
                }
                existed
            }
            None => false,
        };

        if let Some((_, handle)) = self.runs.remove(&goal_id) {
            handle.stop.store(true, Ordering::SeqCst);
            // A recovered terminal entry has no live loop task to abort.
            if let Some(task) = handle.task {
                task.abort();
            }
            delete_persisted_run(&self.store, goal_id);
            true
        } else {
            had_checkpoint
        }
    }

    /// Start an autonomous run that drives `agent_id` toward `goal_id`.
    ///
    /// `send_message` performs one agent turn and yields the agent's reply text
    /// (or an error string). The loop owns iteration counting, marker parsing,
    /// goal persistence, and the rate-limit circuit breaker.
    ///
    /// Replaces any existing run for the same goal.
    ///
    /// **Resumes rather than restarts** when the goal has a resumable durable
    /// row — one left by [`GoalRunner::pause`], or one left `Running` by a
    /// process that died mid-run. The iteration count, last progress and
    /// accumulated learnings are seeded from that checkpoint. A cancelled
    /// ([`GoalRunner::stop`]) or completed run leaves no row, so starting it
    /// again correctly begins from iteration 0.
    ///
    /// `tick_interval_secs` sets the pause between iterations; see
    /// [`clamp_goal_tick_interval_secs`] for the accepted range.
    #[allow(clippy::too_many_arguments)]
    pub fn start<F, Fut, S, Sfut, L, E, Efut>(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: u32,
        substrate: Arc<MemorySubstrate>,
        send_message: F,
        spawn_sub_agent: S,
        on_learnings_captured: L,
        evaluate_goal: E,
        loop_engineering: bool,
        verify_agent_id: Option<AgentId>,
        verify_max_retries: Option<u32>,
        evaluator_model: Option<String>,
        tick_interval_secs: Option<u32>,
    ) where
        F: Fn(AgentId, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
        S: Fn(String) -> Sfut + Send + Sync + 'static,
        Sfut: std::future::Future<Output = Option<AgentId>> + Send + 'static,
        L: Fn(Vec<String>) + Send + Sync + 'static,
        E: Fn(String, String) -> Efut + Send + Sync + 'static,
        Efut: std::future::Future<Output = Result<bool, String>> + Send + 'static,
    {
        // Hold `start_lock` for the whole stop→gen→spawn→insert sequence so a
        // concurrent `start()` for the same goal cannot observe the empty slot
        // this creates between the stop and the insert and spawn a second,
        // orphaned loop. The sequence is synchronous (no `.await`), so this std
        // guard is never held across an await point; `tokio::spawn` only
        // enqueues the task and does not block.
        let _guard = self
            .start_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Read the resume checkpoint BEFORE `stop_locked`, which deletes the
        // durable row: reading after it would make every start look like a
        // fresh one and silently reset a paused goal to iteration 0.
        let resume = read_resume_point(&self.store, &substrate, goal_id).unwrap_or_default();
        if resume.iteration > 0 || !resume.learnings.is_empty() {
            info!(
                goal_id = %goal_id,
                agent_id = %agent_id,
                from_iteration = resume.iteration,
                last_progress = resume.last_progress,
                learnings = resume.learnings.len(),
                "Resuming goal run from persisted checkpoint"
            );
        }

        let (tick_secs, clamped) = clamp_goal_tick_interval_secs(tick_interval_secs);
        if clamped {
            warn!(
                goal_id = %goal_id,
                requested = ?tick_interval_secs,
                effective_secs = tick_secs,
                "Goal tick interval out of range; clamped"
            );
        }
        let tick_interval = Duration::from_secs(tick_secs as u64);

        // Replace any prior run for this goal. `stop_locked` (not `stop`)
        // because we already hold `start_lock`, which is non-reentrant.
        self.stop_locked(goal_id);
        let now = Utc::now();
        let initial = GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: resume.iteration,
            max_iterations,
            last_progress: resume.last_progress,
            last_error: None,
            verify_agent_id,
            verify_max_retries: verify_max_retries.unwrap_or(3),
            evaluator_model,
            started_at: now,
            updated_at: now,
        };
        // Persist the initial Running row before the first tick so a crash
        // mid-tick still leaves a recoverable record at the next boot. The
        // new-run upsert also atomically replaces a terminal predecessor's
        // start time if one survived an earlier daemon restart.
        persist_new_run(&self.store, &initial);
        let state = Arc::new(Mutex::new(initial));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let generation = self.next_gen.fetch_add(1, Ordering::SeqCst);

        let shutdown_rx = self.shutdown_rx.clone();
        let loop_state = state.clone();
        let loop_stop = stop.clone();
        let loop_pause = pause.clone();
        let loop_store = self.store.clone();
        let loop_learnings = resume.learnings;

        // Insert the RunHandle BEFORE spawning so the spawned task's
        // self-cleanup `remove_if` always finds its entry — even if the
        // loop exits immediately (max_iterations=0, pre-signalled shutdown,
        // goal not found, …). The generation guard ensures a concurrent
        // `start()` replacement never has its entry removed by the old task.
        let dashmap_key = goal_id;
        let handle_generation = generation;
        let runs_for_cleanup = Arc::clone(&self.runs);
        self.runs.insert(
            dashmap_key,
            RunHandle {
                task: None, // filled after spawn
                state: state.clone(),
                stop: stop.clone(),
                pause: pause.clone(),
                generation,
            },
        );

        let task = tokio::spawn(async move {
            run_loop(
                goal_id,
                agent_id,
                max_iterations,
                substrate,
                send_message,
                spawn_sub_agent,
                on_learnings_captured,
                evaluate_goal,
                loop_engineering,
                loop_state,
                loop_stop,
                loop_pause,
                shutdown_rx,
                loop_store,
                RunSeed {
                    iteration: resume.iteration,
                    learnings: loop_learnings,
                    tick_interval,
                },
            )
            .await;
            // Self-cleanup: drop the registry entry once the loop ends.
            // Guarded by generation so a replacement run is never removed.
            runs_for_cleanup.remove_if(&goal_id, |_, h| h.generation == handle_generation);
        });

        // Backfill the task handle into the already-inserted RunHandle so
        // `stop()` can abort the spawned task.
        if let Some(mut entry) = self.runs.get_mut(&goal_id) {
            if entry.generation == generation {
                entry.task = Some(task);
            }
        }
        info!(
            goal_id = %goal_id,
            agent_id = %agent_id,
            max_iterations,
            from_iteration = resume.iteration,
            tick_interval_secs = tick_secs,
            "Goal run started"
        );
    }

    /// Recover goal runs left in `Running` phase by a prior crash or restart.
    ///
    /// Called once at boot, mirroring `WorkflowEngine::recover_stale_running_runs`.
    /// Only persisted rows still in `Running` phase are candidates — any
    /// terminal-phase row was already deleted when its run ended, so the only
    /// `Running` rows on disk are ones whose process died mid-run. For each such
    /// row older than `stale_timeout`, demote it to `Stopped` with the same
    /// `"Interrupted by daemon restart"` marker workflow recovery uses, persist
    /// that, and checkpoint the WAL so the transition is durable.
    /// Returns (goal_id, agent_id) pairs for stale runs that should be
    /// auto-resumed by the caller. The caller calls start() for each,
    /// which spawns a fresh loop continuing from the last persisted state.
    pub fn recover_stale_runs(&self, stale_timeout: Duration) -> Vec<(GoalId, AgentId)> {
        let Some(store) = self.store.as_ref() else {
            return Vec::new();
        };
        if stale_timeout.is_zero() {
            return Vec::new();
        }
        let rows = match store.load_all_runs() {
            Ok(rows) => rows,
            Err(e) => {
                warn!("Failed to load persisted goal runs for recovery: {e}");
                return Vec::new();
            }
        };

        let now = Utc::now();
        let stale_secs = stale_timeout.as_secs() as i64;
        let mut recovered: Vec<(GoalId, AgentId)> = Vec::new();
        for row in rows {
            // Terminal-phase rows are settled; only `Running` rows are stale
            // candidates. (Belt-and-braces: the run loop deletes terminal rows,
            // so a non-running row on disk would be a bug elsewhere.)
            if row.phase != GoalRunPhase::Running.to_string() {
                continue;
            }
            let Ok(goal_id) = row.goal_id.parse::<GoalId>() else {
                warn!(goal_id = %row.goal_id, "Skipping goal run with unparseable id during recovery");
                continue;
            };
            let started_at = match chrono::DateTime::parse_from_rfc3339(&row.started_at) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(e) => {
                    warn!(goal_id = %goal_id, "Skipping goal run with unparseable started_at during recovery: {e}");
                    continue;
                }
            };
            let age = now.signed_duration_since(started_at).num_seconds();
            // Wall-clock skew guard, identical to the workflow sweep (#5114):
            // `Utc::now()` is not monotonic, so a backwards NTP step makes `age`
            // negative. Treat a negative age as "fresh" rather than silently
            // masking a real stale row, and warn so operators see the skew.
            if age < 0 {
                warn!(
                    goal_id = %goal_id,
                    now = %now,
                    started_at = %started_at,
                    age_secs = age,
                    "Negative goal run age — wall-clock moved backwards; \
                     treating run as fresh, not stale"
                );
                continue;
            }
            if age < stale_secs {
                continue;
            }
            warn!(
                goal_id = %goal_id,
                started_at = %started_at,
                age_secs = age,
                iteration = row.iteration,
                "Stale goal run detected — will auto-resume"
            );
            match row.agent_id.parse::<AgentId>() {
                Ok(agent_id) => {
                    // Leave the row in place: `start` reads it back as the
                    // resume checkpoint and only then replaces it. Deleting it
                    // here is what made "resume" restart the goal from
                    // iteration 0, contradicting boot's own comment that the
                    // loop picks up from the last checkpoint.
                    recovered.push((goal_id, agent_id));
                }
                Err(_) => {
                    warn!(goal_id = %goal_id, agent_id = %row.agent_id,
                          "Dropping stale goal run with unparseable agent_id");
                    // Nothing will ever resume this row, so it must not linger
                    // and be re-reported as stale at every subsequent boot.
                    let _ = store.delete_run(&goal_id.to_string());
                }
            }
        }
        recovered
    }

    /// Legacy recovery — kept for tests. Use recover_stale_runs instead.
    pub fn recover_stale_runs_demote_for_tests(&self, stale_timeout: Duration) -> Vec<GoalId> {
        let Some(store) = self.store.as_ref() else {
            return Vec::new();
        };
        if stale_timeout.is_zero() {
            return Vec::new();
        }
        let rows = match store.load_all_runs() {
            Ok(rows) => rows,
            Err(e) => {
                warn!("Failed to load persisted goal runs: {e}");
                return Vec::new();
            }
        };
        let now = Utc::now();
        let stale_secs = stale_timeout.as_secs() as i64;
        let mut recovered: Vec<GoalId> = Vec::new();
        for row in rows {
            if row.phase != GoalRunPhase::Running.to_string() {
                continue;
            }
            let Ok(goal_id) = row.goal_id.parse::<GoalId>() else {
                continue;
            };
            let started_at = match chrono::DateTime::parse_from_rfc3339(&row.started_at) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };
            let age = now.signed_duration_since(started_at).num_seconds();
            if age < 0 || age < stale_secs {
                continue;
            }
            let r = GoalRunRow {
                phase: GoalRunPhase::Stopped.to_string(),
                last_error: Some("Interrupted by daemon restart".to_string()),
                updated_at: now.to_rfc3339(),
                ..row
            };
            let _ = store.save_run(&r);
            recovered.push(goal_id);
        }
        recovered
    }
}

/// Where a run loop picks up from, and how fast it ticks.
///
/// Bundled rather than passed as three more positional parameters to an
/// already 13-argument function, where a transposed `u32` would compile.
struct RunSeed {
    /// Iteration to start counting from. Non-zero when resuming.
    iteration: u32,
    /// Learnings carried over from a paused or interrupted segment.
    learnings: Vec<String>,
    /// Pause between iterations, already clamped.
    tick_interval: Duration,
}

/// The run loop body. Extracted as a free function so tests can drive it with a
/// fake `send_message` and an in-memory substrate.
#[allow(clippy::too_many_arguments)]
async fn run_loop<F, Fut, S, Sfut, L, E, Efut>(
    goal_id: GoalId,
    agent_id: AgentId,
    max_iterations: u32,
    substrate: Arc<MemorySubstrate>,
    send_message: F,
    spawn_sub_agent: S,
    on_learnings_captured: L,
    evaluate_goal: E,
    loop_engineering: bool,
    state: Arc<Mutex<GoalRunState>>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<bool>,
    store: Option<GoalRunStore>,
    seed: RunSeed,
) where
    F: Fn(AgentId, String) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<String, String>> + Send,
    S: Fn(String) -> Sfut + Send + Sync,
    Sfut: std::future::Future<Output = Option<AgentId>> + Send,
    L: Fn(Vec<String>) + Send + Sync,
    E: Fn(String, String) -> Efut + Send + Sync,
    Efut: std::future::Future<Output = Result<bool, String>> + Send,
{
    let RunSeed {
        iteration: seed_iteration,
        learnings: seed_learnings,
        tick_interval,
    } = seed;
    let mut iteration: u32 = seed_iteration;
    // Error/rate-limit streaks deliberately restart at zero on a resume: they
    // are a circuit breaker on the *provider's current condition*, not run
    // history. Carrying a streak across a pause would trip the breaker on the
    // first failure of a resume that happens hours later, against a provider
    // that has long since recovered.
    let mut rate_limit_streak: u32 = 0;
    let mut error_streak: u32 = 0;
    // Learnings captured across iterations via `GOAL_LEARNED:` markers.
    // Seeded from the previous segment on a resume, and checkpointed to shared
    // memory on pause as well as on exit so a pause never discards them.
    let mut accumulated_learnings: Vec<String> = seed_learnings;
    let mut interrupted_by_shutdown = false;
    let final_phase = loop {
        if stop.load(Ordering::SeqCst) {
            break GoalRunPhase::Stopped;
        }
        // Checked before shutdown: a pause that lands as the daemon goes down
        // must still checkpoint, and a paused row is not auto-resumed at the
        // next boot the way a shutdown-interrupted `Running` row is.
        if pause.load(Ordering::SeqCst) {
            break GoalRunPhase::Paused;
        }
        if *shutdown_rx.borrow() {
            interrupted_by_shutdown = true;
            break GoalRunPhase::Stopped;
        }

        let goal = match load_goal(&substrate, goal_id) {
            Some(g) => g,
            None => {
                warn!(goal_id = %goal_id, "Goal vanished from store; ending run");
                break GoalRunPhase::Finished;
            }
        };
        if matches!(goal.status, GoalStatus::Completed | GoalStatus::Cancelled)
            || goal.progress >= 100
        {
            break GoalRunPhase::Finished;
        }
        if iteration >= max_iterations {
            break GoalRunPhase::MaxIterationsReached;
        }

        let has_verifier = loop_engineering && {
            let s = state.lock().await;
            s.verify_agent_id.is_some()
        };
        let prompt = build_goal_prompt(
            &goal,
            iteration,
            max_iterations,
            has_verifier,
            &accumulated_learnings,
        );
        debug!(goal_id = %goal_id, iteration, "Goal run: sending tick");

        match send_message(agent_id, prompt).await {
            Ok(reply) => {
                rate_limit_streak = 0;
                error_streak = 0; // reset error streak on success
                let parsed = parse_tick(&reply);
                // Collect learnings for self-evolution (loop_engineering only).
                if loop_engineering && !parsed.learnings.is_empty() {
                    accumulated_learnings.extend(parsed.learnings.clone());
                    info!(goal_id = %goal_id, learnings = ?parsed.learnings,
                          "Goal run: captured learnings");
                }

                // Verifier loop: only active when loop_engineering is on
                // and a verifier agent is configured.
                // against this iteration's output before accepting progress.
                let verify_id = if loop_engineering {
                    let s = state.lock().await;
                    s.verify_agent_id
                } else {
                    None
                };
                let max_retries = {
                    let s = state.lock().await;
                    s.verify_max_retries.max(1)
                };
                let mut passes_verification = true;
                if let Some(vid) = verify_id {
                    let mut retries = 0;
                    let mut current_verifier = vid;
                    loop {
                        // Auto-spawn a fresh reviewer after 2 rejections
                        // to get an independent second opinion.
                        if retries == 2 {
                            if let Some(new_id) = spawn_sub_agent("reviewer".to_string()).await {
                                info!(goal_id = %goal_id, old_verifier = %current_verifier,
                                      new_reviewer = %new_id,
                                      "Auto-spawned fresh reviewer sub-agent");
                                current_verifier = new_id;
                            }
                        }
                        let verdict_prompt = format!(
                            "Verify this output against the goal. Reply with exactly one line:\n\
                             VERDICT: PASS|FAIL|NEEDS_REWORK\n\
                             REASON: <one sentence>\n\n\
                             Goal: {title}\n\
                             Output:\n{reply}",
                            title = goal.title,
                        );
                        match send_message(current_verifier, verdict_prompt).await {
                            Ok(verdict) => {
                                let upper = verdict.to_ascii_uppercase();
                                if upper.contains("VERDICT: PASS") || upper.contains("VERDICT:PASS")
                                {
                                    info!(goal_id = %goal_id, iteration, retries,
                                          "Verifier: PASS");
                                    break; // passes
                                }
                                retries += 1;
                                if retries >= max_retries {
                                    warn!(goal_id = %goal_id, iteration, retries,
                                          verdict = %verdict.trim(),
                                          "Verifier: max retries exceeded");
                                    passes_verification = false;
                                    break;
                                }
                                info!(goal_id = %goal_id, iteration, retries,
                                      verdict = %verdict.trim(),
                                      "Verifier: retrying");
                                // Loop back — will call send_message(agent_id) again
                            }
                            Err(e) => {
                                warn!(goal_id = %goal_id, verifier = %vid,
                                      error = %e, "Verifier call failed");
                                retries += 1;
                                if retries >= max_retries {
                                    passes_verification = false;
                                    break;
                                }
                            }
                        }
                    }
                }

                // Evaluator model: a separate cheap model judges whether the
                // goal condition is met (Claude Code /goal pattern). This
                // replaces blind trust in the agent's self-reported GOAL_DONE.
                let evaluator_done = if passes_verification {
                    match evaluate_goal(goal.description.clone(), reply.clone()).await {
                        Ok(true) => {
                            info!(goal_id = %goal_id, iteration, "Evaluator: goal condition met");
                            true
                        }
                        Ok(false) => false,
                        Err(e) => {
                            warn!(goal_id = %goal_id, error = %e, "Evaluator call failed; trusting agent markers");
                            parsed.done // fall back to agent markers
                        }
                    }
                } else {
                    false
                };

                // Completion is the evaluator's verdict OR the agent's own
                // marker, gated on verification. The evaluator has to be
                // consulted *before* the goal document is patched: patching on
                // `parsed.done` alone stranded a goal in `InProgress` forever
                // whenever the agent finished the work but never emitted
                // `GOAL_DONE` — the evaluator said "met", the loop broke out,
                // and nothing ever wrote `Completed` back.
                let done = passes_verification && (parsed.done || evaluator_done);
                let new_status = if done {
                    Some(GoalStatus::Completed)
                } else {
                    Some(GoalStatus::InProgress)
                };
                let new_progress = if done { Some(100) } else { parsed.progress };
                patch_goal(&substrate, goal_id, new_progress, new_status);

                // Release before persist_run: state()'s try_lock returns None (→ running:false) while held.
                let snapshot = {
                    let mut s = state.lock().await;
                    s.iteration = iteration + 1;
                    if let Some(p) = new_progress {
                        s.last_progress = p;
                    }
                    s.last_error = None;
                    s.updated_at = Utc::now();
                    s.clone()
                };
                // Mirror the post-iteration state to the durable store so a
                // crash before the next tick still leaves a recoverable row.
                persist_run(&store, &snapshot);

                if done {
                    break GoalRunPhase::Finished;
                }
                if parsed.blocked {
                    info!(goal_id = %goal_id, "Goal run: agent reported blocked; ending run");
                    break GoalRunPhase::Stopped;
                }
            }
            Err(e) => {
                match classify_tick_error(&e) {
                    TickOutcome::RateLimited => {
                        rate_limit_streak = rate_limit_streak.saturating_add(1);
                        warn!(
                            goal_id = %goal_id,
                            consecutive_rate_limits = rate_limit_streak,
                            "Goal run: tick failed on provider rate-limit",
                        );
                    }
                    TickOutcome::Ok => {
                        rate_limit_streak = 0;
                        error_streak = error_streak.saturating_add(1);
                    }
                }
                // Same lock discipline as success path: release before persist_run.
                let snapshot = {
                    let mut s = state.lock().await;
                    s.last_error = Some(e);
                    s.updated_at = Utc::now();
                    s.clone()
                };
                persist_run(&store, &snapshot);
                if rate_limit_streak >= MAX_RATE_LIMIT_STREAK {
                    break GoalRunPhase::RateLimited;
                }
                if error_streak >= MAX_ERROR_STREAK {
                    warn!(goal_id = %goal_id, consecutive_errors = error_streak,
                          "Goal run: stopping after too many consecutive errors");
                    break GoalRunPhase::Stopped;
                }
            }
        }

        iteration += 1;

        tokio::select! {
            _ = tokio::time::sleep(tick_interval) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    interrupted_by_shutdown = true;
                    break GoalRunPhase::Stopped;
                }
            }
        }
    };

    let snapshot = {
        let mut s = state.lock().await;
        s.phase = final_phase;
        s.updated_at = Utc::now();
        s.clone()
    };

    if final_phase == GoalRunPhase::Paused {
        // A pause is a checkpoint, not an ending. Write everything the
        // successor loop needs — iteration, progress, and the learnings that
        // would otherwise die with this task's stack — as one KV entry.
        persist_pause_checkpoint(&substrate, goal_id, &snapshot, &accumulated_learnings);
        // The `goal_runs` mirror tracks *active* runs, so the paused run leaves
        // it, exactly as a cancelled one does. Leaving a `Running` row behind
        // would make boot recovery auto-resume a goal the operator deliberately
        // suspended.
        delete_persisted_run(&store, goal_id);
        info!(
            goal_id = %goal_id,
            iteration = snapshot.iteration,
            last_progress = snapshot.last_progress,
            learnings = accumulated_learnings.len(),
            "Goal run paused — state checkpointed for resume"
        );
        // No `on_learnings_captured`: that hook mints a skill from what the run
        // discovered, which is a conclusion the run has not reached yet.
        return;
    }

    // Any other exit settles the run, so a checkpoint from an earlier pause of
    // the same goal must not survive to seed a later fresh start.
    clear_pause_checkpoint(&substrate, goal_id);

    // A run that reaches a natural terminal phase (completed, capped, rate-
    // limited, agent-blocked, or an operator cancel) is settled — drop its
    // durable row so it is never resurfaced as "stale" at the next boot. A
    // shutdown-interrupted run is the exception: leave its last `Running` row
    // in place so boot recovery resumes it, exactly as workflow runs do.
    if !interrupted_by_shutdown {
        delete_persisted_run(&store, goal_id);
    }
    // Persist captured learnings to shared memory (loop_engineering only).
    if loop_engineering && !accumulated_learnings.is_empty() {
        persist_goal_learnings(&substrate, goal_id, &accumulated_learnings);
        // Caller hook: push learnings into proactive memory so the agent
        // recalls them in future conversations, and trigger auto_evolve
        // to refine prompts from what was discovered.
        on_learnings_captured(accumulated_learnings.clone());
    }
    info!(goal_id = %goal_id, phase = %final_phase, "Goal run ended");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tick_extracts_progress_done_blocked() {
        let p = parse_tick("working...\nGOAL_PROGRESS: 60\nmore text");
        assert_eq!(p.progress, Some(60));
        assert!(!p.done);

        let d = parse_tick("all set\ngoal_done");
        assert!(d.done);

        let b = parse_tick("stuck\nGOAL_BLOCKED: need a key");
        assert!(b.blocked);

        // Last progress wins; >100 clamps.
        let m = parse_tick("GOAL_PROGRESS: 30\nGOAL_PROGRESS: 250");
        assert_eq!(m.progress, Some(100));

        // No markers → all default.
        assert_eq!(parse_tick("just a normal reply"), ParsedTick::default());
    }

    #[test]
    fn parse_tick_captures_learnings_preserving_original_case() {
        // The marker is matched case-insensitively, but the captured text is
        // persisted to memory and fed into auto-generated skills verbatim, so
        // it must come back exactly as the agent wrote it. Slicing the
        // uppercased copy used for matching shouted every stored learning.
        let p = parse_tick("GOAL_LEARNED: Use ripgrep, not grep, on this repo");
        assert_eq!(p.learnings, vec!["Use ripgrep, not grep, on this repo"]);

        // Lowercase marker still registers, payload still verbatim.
        let lower = parse_tick("goal_learned: Prefer BTreeMap for prompt ordering");
        assert_eq!(lower.learnings, vec!["Prefer BTreeMap for prompt ordering"]);

        // Non-ASCII payloads survive the byte-offset slice intact.
        let unicode = parse_tick("GOAL_LEARNED: El daemon escucha en el puerto 4545");
        assert_eq!(
            unicode.learnings,
            vec!["El daemon escucha en el puerto 4545"]
        );

        // Multiple learnings accumulate in order; blank payloads are dropped.
        let many = parse_tick("GOAL_LEARNED: first\nGOAL_LEARNED:   \nGOAL_LEARNED: Second Thing");
        assert_eq!(many.learnings, vec!["first", "Second Thing"]);
    }

    #[test]
    fn parse_tick_requires_marker_token_boundary() {
        // Substrings that merely start with a control marker must NOT trip it.
        assert!(!parse_tick("GOAL_DONENESS: not yet").done);
        assert!(!parse_tick("GOAL_DONE_CRITERIA: ship it").done);
        assert!(!parse_tick("GOAL_COMPLETENESS: 40%").done);
        assert!(!parse_tick("GOAL_BLOCKEDNESS is low").blocked);

        // Bare and boundary-delimited forms still register, including the
        // trailing punctuation the model commonly appends.
        assert!(parse_tick("GOAL_DONE").done);
        assert!(parse_tick("GOAL_DONE now").done);
        assert!(parse_tick("GOAL_DONE.").done);
        assert!(parse_tick("GOAL_DONE!").done);
        assert!(parse_tick("GOAL_DONE - shipped the report").done);
        assert!(parse_tick("GOAL_COMPLETE").done);
        assert!(parse_tick("GOAL_BLOCKED").blocked);
        assert!(parse_tick("GOAL_BLOCKED! waiting on a key").blocked);
        assert!(parse_tick("GOAL_BLOCKED: need a key").blocked);
    }

    fn seed_goal(substrate: &MemorySubstrate, goal: &Goal) {
        substrate
            .structured_set(
                goals_storage_agent_id(),
                GOALS_STORAGE_KEY,
                serde_json::json!([serde_json::to_value(goal).unwrap()]),
            )
            .unwrap();
    }

    fn test_goal(agent_id: AgentId) -> Goal {
        Goal {
            id: GoalId::new(),
            title: "Write a report".into(),
            description: String::new(),
            parent_id: None,
            status: GoalStatus::InProgress,
            progress: 0,
            agent_id: Some(agent_id),
            loop_engineering: false,
            verify_agent_id: None,
            evaluator_model: None,
            tick_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// A fresh-run seed at the production cadence, for the pre-existing tests
    /// that predate resume and configurable ticks.
    fn test_seed() -> RunSeed {
        RunSeed {
            iteration: 0,
            learnings: Vec::new(),
            tick_interval: Duration::from_secs(
                librefang_types::goal::DEFAULT_GOAL_TICK_INTERVAL_SECS as u64,
            ),
        }
    }

    /// A seed that ticks as fast as the loop allows, so a test exercising
    /// multi-iteration behaviour does not spend seconds asleep.
    fn fast_seed(iteration: u32, learnings: Vec<String>) -> RunSeed {
        RunSeed {
            iteration,
            learnings,
            tick_interval: Duration::from_millis(1),
        }
    }

    #[tokio::test]
    async fn run_loop_stops_and_completes_on_goal_done() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (_tx, rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations: 10,
            last_progress: 0,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }));

        // Agent reports done on the first turn.
        let send = |_a: AgentId, _p: String| async move { Ok("done\nGOAL_DONE".to_string()) };

        run_loop(
            goal_id,
            agent_id,
            10,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(false)), // stop
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            None,
            test_seed(),
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Finished);
        let stored = load_goal(&substrate, goal_id).unwrap();
        assert_eq!(stored.status, GoalStatus::Completed);
        assert_eq!(stored.progress, 100);
    }

    #[tokio::test]
    async fn run_loop_honors_max_iterations() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (_tx, rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations: 2,
            last_progress: 0,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }));

        // Agent never finishes — always reports partial progress.
        let send = |_a: AgentId, _p: String| async move { Ok("GOAL_PROGRESS: 10".to_string()) };

        run_loop(
            goal_id,
            agent_id,
            2,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(false)), // stop
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            None,
            test_seed(),
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::MaxIterationsReached);
        assert_eq!(s.iteration, 2);
        // Goal stays in progress, not completed.
        let stored = load_goal(&substrate, goal_id).unwrap();
        assert_eq!(stored.status, GoalStatus::InProgress);
    }

    fn mk_state(
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: u32,
    ) -> Arc<Mutex<GoalRunState>> {
        Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations,
            last_progress: 0,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    #[tokio::test]
    async fn run_loop_stops_when_agent_reports_blocked() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 10);

        let send = |_a: AgentId, _p: String| async move {
            Ok("stuck\nGOAL_BLOCKED: need a key".to_string())
        };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(false)), // stop
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            None,
            test_seed(),
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Stopped);
        // Blocked must NOT mark the goal completed.
        assert_eq!(
            load_goal(&substrate, goal.id).unwrap().status,
            GoalStatus::InProgress
        );
    }

    #[tokio::test]
    async fn run_loop_stops_immediately_when_stop_flag_preset() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 10);

        // Operator stop is observed at the top of the loop before any tick.
        let send = |_a: AgentId, _p: String| async move {
            panic!("send_message must not be called once the stop flag is set");
            #[allow(unreachable_code)]
            Ok(String::new())
        };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(true)),  // stop, preset
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            None,
            test_seed(),
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Stopped);
        assert_eq!(s.iteration, 0, "no tick should run");
    }

    #[tokio::test]
    async fn run_loop_stops_immediately_on_shutdown_signal() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        // Shutdown already signalled.
        let (_tx, rx) = watch::channel(true);
        let state = mk_state(goal.id, agent_id, 10);

        let send = |_a: AgentId, _p: String| async move {
            panic!("send_message must not be called during shutdown");
            #[allow(unreachable_code)]
            Ok(String::new())
        };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(false)), // stop
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            None,
            test_seed(),
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Stopped);
    }

    #[tokio::test(start_paused = true)]
    async fn run_loop_breaks_after_consecutive_rate_limits() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 100);

        // Every tick fails with the rate-limit marker; the circuit breaker must
        // trip at MAX_RATE_LIMIT_STREAK rather than burning all 100 iterations.
        // start_paused auto-advances the inter-tick sleeps so this is instant.
        let send = |_a: AgentId, _p: String| async move {
            Err(format!(
                "provider quota exhausted {}",
                librefang_channels::message_journal::RATE_LIMIT_DEFER_MARKER
            ))
        };
        run_loop(
            goal.id,
            agent_id,
            100,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(false)), // stop
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            None,
            test_seed(),
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::RateLimited);
        assert!(
            s.iteration < 100,
            "must trip the breaker, not run to the cap"
        );
    }

    // --- Persistence + boot recovery (#5744 follow-up) ---

    /// Build a goal-run store sharing the substrate's SQLite pool. The
    /// substrate has already run migrations, so the `goal_runs` table exists.
    fn store_from(substrate: &MemorySubstrate) -> GoalRunStore {
        GoalRunStore::new(substrate.pool())
    }

    #[tokio::test(start_paused = true)]
    async fn run_loop_persists_state_across_iterations() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let store = store_from(&substrate);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 3);

        // Capture the persisted row after the second iteration, before the run
        // reaches the cap and deletes the row. A oneshot fires from inside the
        // fake send_message on the third call.
        let counter = Arc::new(AtomicU64::new(0));
        let probe_store = store.clone();
        let probe_id = goal.id.to_string();
        let captured: Arc<Mutex<Option<GoalRunRow>>> = Arc::new(Mutex::new(None));
        let probe_captured = captured.clone();
        let send = move |_a: AgentId, _p: String| {
            let counter = counter.clone();
            let probe_store = probe_store.clone();
            let probe_id = probe_id.clone();
            let probe_captured = probe_captured.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                // On the third call (n == 2), two iterations have already
                // persisted; snapshot the row before the loop ends.
                if n == 2 {
                    let row = probe_store.get_run(&probe_id).unwrap();
                    *probe_captured.lock().await = row;
                }
                Ok("GOAL_PROGRESS: 40".to_string())
            }
        };
        run_loop(
            goal.id,
            agent_id,
            3,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(false)), // stop
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            Some(store.clone()),
            test_seed(),
        )
        .await;

        let row = captured
            .lock()
            .await
            .clone()
            .expect("a Running row must have been persisted mid-run");
        assert_eq!(row.phase, GoalRunPhase::Running.to_string());
        assert_eq!(row.goal_id, goal.id.to_string());
        assert!(
            row.iteration >= 2,
            "iterations must accumulate in the store"
        );
        assert_eq!(row.last_progress, 40);
    }

    #[tokio::test]
    async fn completed_run_is_deleted_from_store() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let store = store_from(&substrate);
        // Pre-seed a Running row as `start()` would.
        store
            .save_run(&row_from_state(&GoalRunState {
                goal_id: goal.id,
                agent_id,
                phase: GoalRunPhase::Running,
                iteration: 0,
                max_iterations: 10,
                last_progress: 0,
                last_error: None,
                verify_agent_id: None,
                verify_max_retries: 0,
                evaluator_model: None,
                started_at: Utc::now(),
                updated_at: Utc::now(),
            }))
            .unwrap();
        assert!(store.get_run(&goal.id.to_string()).unwrap().is_some());

        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 10);
        let send = |_a: AgentId, _p: String| async move { Ok("done\nGOAL_DONE".to_string()) };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) }, // evaluate_goal: trust agent markers
            false,                      // loop_engineering
            state.clone(),
            Arc::new(AtomicBool::new(false)), // stop
            Arc::new(AtomicBool::new(false)), // pause
            rx,
            Some(store.clone()),
            test_seed(),
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Finished);
        assert!(
            store.get_run(&goal.id.to_string()).unwrap().is_none(),
            "a completed run must be removed from the durable store"
        );
    }

    #[tokio::test]
    async fn start_replaces_terminal_row_with_a_fresh_started_at() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();
        let stale_started = Utc::now() - chrono::Duration::days(1);
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: agent_id.to_string(),
                phase: GoalRunPhase::Stopped.to_string(),
                iteration: 5,
                max_iterations: 25,
                last_progress: 50,
                last_error: Some("Interrupted by daemon restart".to_string()),
                started_at: stale_started.to_rfc3339(),
                updated_at: stale_started.to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());
        runner.start(
            goal_id,
            agent_id,
            25,
            substrate,
            |_agent_id, _message| async move {
                std::future::pending::<Result<String, String>>().await
            },
            |_name| async move { None::<AgentId> },
            |_learnings| {},
            |_goal, _output| async move { Ok(true) },
            false,
            None,
            None,
            None,
            None,
        );

        let row = store.get_run(&goal_id.to_string()).unwrap().unwrap();
        let started_at = chrono::DateTime::parse_from_rfc3339(&row.started_at)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(row.phase, GoalRunPhase::Running.to_string());
        assert!(
            started_at > stale_started,
            "a new run must not inherit the predecessor's started_at"
        );
        assert_eq!(
            row.iteration, 0,
            "a terminal predecessor is not a resume checkpoint — its iteration \
             count must not carry into the new run"
        );

        assert!(runner.stop(goal_id));
    }

    #[test]
    fn recover_stale_run_returns_pair_for_auto_resume() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();

        // A Running row whose process died an hour ago.
        let stale_started = Utc::now() - chrono::Duration::seconds(3600);
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: agent_id.to_string(),
                phase: GoalRunPhase::Running.to_string(),
                iteration: 5,
                max_iterations: 25,
                last_progress: 50,
                last_error: None,
                started_at: stale_started.to_rfc3339(),
                updated_at: stale_started.to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());

        // 10-minute staleness window → the hour-old run returns a pair.
        let recovered = runner.recover_stale_runs(Duration::from_secs(600));
        assert_eq!(recovered, vec![(goal_id, agent_id)]);

        // The row survives recovery: `start` reads it as the resume checkpoint
        // and only then replaces it. Deleting it here is what made auto-resume
        // silently restart the goal from iteration 0.
        let row = store.get_run(&goal_id.to_string()).unwrap().unwrap();
        assert_eq!(row.iteration, 5);
        assert_eq!(row.last_progress, 50);
    }

    // -----------------------------------------------------------------------
    // Pause vs cancel
    // -----------------------------------------------------------------------

    /// Pausing must keep the durable row, carrying the iteration count the run
    /// reached. Cancelling deletes it. That difference is the entire point of
    /// having two verbs: before this, "stop" was the only option and it made
    /// resuming impossible.
    #[tokio::test]
    async fn pause_checkpoints_the_row_that_cancel_would_delete() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 100);

        // Let two turns land, then ask for a pause.
        let pause = Arc::new(AtomicBool::new(false));
        let turns = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let send = {
            let pause = pause.clone();
            let turns = turns.clone();
            move |_a: AgentId, _p: String| {
                let pause = pause.clone();
                let turns = turns.clone();
                async move {
                    if turns.fetch_add(1, Ordering::SeqCst) >= 1 {
                        pause.store(true, Ordering::SeqCst);
                    }
                    Ok("GOAL_PROGRESS: 40".to_string())
                }
            }
        };

        run_loop(
            goal.id,
            agent_id,
            100,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) },
            false,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            pause,
            rx,
            Some(store.clone()),
            fast_seed(0, Vec::new()),
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Paused);

        let checkpoint = load_pause_checkpoint(&substrate, goal.id)
            .expect("a paused run must leave a resume checkpoint");
        assert_eq!(
            checkpoint.iteration, 2,
            "both completed turns must be counted"
        );
        assert_eq!(checkpoint.last_progress, 40);

        // The `goal_runs` mirror tracks active runs only, so a paused run
        // leaves it — otherwise boot recovery would auto-resume a goal the
        // operator deliberately suspended.
        assert!(store.get_run(&goal.id.to_string()).unwrap().is_none());
    }

    /// Learnings live in a local on the loop task's stack. Without an explicit
    /// checkpoint on pause they are silently discarded and the resumed run
    /// pays to rediscover them.
    #[tokio::test]
    async fn pause_checkpoints_accumulated_learnings() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 100);

        let pause = Arc::new(AtomicBool::new(false));
        let send = {
            let pause = pause.clone();
            move |_a: AgentId, _p: String| {
                let pause = pause.clone();
                async move {
                    pause.store(true, Ordering::SeqCst);
                    Ok("GOAL_LEARNED: the retry budget is per-host".to_string())
                }
            }
        };

        run_loop(
            goal.id,
            agent_id,
            100,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| panic!("pausing must not mint a skill — the run has not concluded"),
            |_, _| async { Ok(false) },
            true, // loop_engineering — required for learnings capture
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            pause,
            rx,
            Some(store.clone()),
            fast_seed(0, Vec::new()),
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Paused);
        let checkpoint = load_pause_checkpoint(&substrate, goal.id)
            .expect("pausing must leave a resume checkpoint");
        assert_eq!(
            checkpoint.learnings,
            vec!["the retry budget is per-host".to_string()],
            "learnings live on the loop task's stack — a pause that does not \
             checkpoint them makes the resumed run rediscover them at full cost"
        );
    }

    /// The resume half: a paused checkpoint must seed the successor loop's
    /// iteration counter and learnings, not be quietly ignored.
    #[tokio::test]
    async fn start_resumes_from_a_paused_checkpoint() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);

        let paused_snapshot = GoalRunState {
            goal_id: goal.id,
            agent_id,
            phase: GoalRunPhase::Paused,
            iteration: 7,
            max_iterations: 25,
            last_progress: 65,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        };
        persist_pause_checkpoint(
            &substrate,
            goal.id,
            &paused_snapshot,
            &["cache the token".to_string()],
        );

        let resume = read_resume_point(&Some(store.clone()), &substrate, goal.id)
            .expect("a pause checkpoint is a resume point");
        assert_eq!(resume.iteration, 7);
        assert_eq!(resume.last_progress, 65);
        assert_eq!(resume.learnings, vec!["cache the token".to_string()]);

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());
        runner.start(
            goal.id,
            agent_id,
            25,
            substrate,
            |_a, _m| async move { std::future::pending::<Result<String, String>>().await },
            |_n| async move { None::<AgentId> },
            |_l| {},
            |_g, _o| async move { Ok(false) },
            false,
            None,
            None,
            None,
            None,
        );

        let live = runner.state(goal.id).expect("run registered");
        assert_eq!(
            live.iteration, 7,
            "resuming a paused goal must continue from its checkpoint, not restart it"
        );
        assert_eq!(live.last_progress, 65);
        assert!(runner.stop(goal.id));
    }

    /// A cancelled run leaves no checkpoint, so starting the goal again is a
    /// genuine fresh start. This is the behaviour `stop` is *for*, and the
    /// contrast that makes pause meaningful.
    #[test]
    fn cancel_discards_the_checkpoint_so_the_next_start_is_fresh() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);

        let paused_snapshot = GoalRunState {
            goal_id: goal.id,
            agent_id,
            phase: GoalRunPhase::Paused,
            iteration: 7,
            max_iterations: 25,
            last_progress: 65,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        };
        persist_pause_checkpoint(&substrate, goal.id, &paused_snapshot, &[]);

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());

        // Cancel reaches the checkpoint even with no live loop and no
        // `goal_runs` row — the exact shape of a goal paused before a restart.
        assert!(
            runner.stop(goal.id),
            "cancelling a paused goal must report that it discarded something"
        );
        assert!(
            read_resume_point(&Some(store), &substrate, goal.id).is_none(),
            "cancel must discard the checkpoint, or the next start silently resumes"
        );
    }

    /// A paused run's loop task exits and self-cleans its registry slot, so
    /// without the store fallback `GET /api/goals/{id}/run` would report the
    /// goal as having no run at all — an operator could pause work and lose
    /// every trace of it.
    #[test]
    fn paused_run_stays_observable_after_its_task_exits() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();

        let paused_snapshot = GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Paused,
            iteration: 3,
            max_iterations: 25,
            last_progress: 30,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        };
        persist_pause_checkpoint(&substrate, goal_id, &paused_snapshot, &[]);

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store, substrate.clone());

        // Nothing in the in-memory registry — exactly the post-pause state.
        let observed = runner
            .state(goal_id)
            .expect("a paused run must remain visible");
        assert_eq!(observed.phase, GoalRunPhase::Paused);
        assert_eq!(observed.iteration, 3);
        assert_eq!(observed.agent_id, agent_id);
    }

    /// `pause()` on a goal with no live run reports false rather than
    /// fabricating a paused checkpoint out of nothing.
    #[test]
    fn pause_on_an_idle_goal_reports_false() {
        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new(rx);
        assert!(!runner.pause(GoalId::new()));
    }

    // -----------------------------------------------------------------------
    // Configurable cadence
    // -----------------------------------------------------------------------

    /// The loop must actually sleep for the configured interval. Asserts only
    /// the lower bound — a sleep can overrun on a loaded machine but never
    /// finishes early, so this cannot flake.
    #[tokio::test]
    async fn run_loop_waits_the_configured_tick_interval_between_iterations() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 2);

        let tick = Duration::from_millis(250);
        let send = |_a: AgentId, _p: String| async move { Ok("GOAL_PROGRESS: 5".to_string()) };

        let began = std::time::Instant::now();
        run_loop(
            goal.id,
            agent_id,
            2,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) },
            false,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
            RunSeed {
                iteration: 0,
                learnings: Vec::new(),
                tick_interval: tick,
            },
        )
        .await;
        let elapsed = began.elapsed();

        assert_eq!(state.lock().await.phase, GoalRunPhase::MaxIterationsReached);
        // Two iterations run, each followed by one sleep.
        assert!(
            elapsed >= tick * 2,
            "expected at least {:?} of tick sleeps, took {elapsed:?}",
            tick * 2
        );
    }

    /// The seed's iteration is where counting starts, so a resumed run
    /// consumes the remainder of its cap instead of the whole cap again.
    #[tokio::test]
    async fn a_resumed_run_counts_toward_the_same_iteration_cap() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 5);

        let turns = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let send = {
            let turns = turns.clone();
            move |_a: AgentId, _p: String| {
                let turns = turns.clone();
                async move {
                    turns.fetch_add(1, Ordering::SeqCst);
                    Ok("GOAL_PROGRESS: 80".to_string())
                }
            }
        };

        run_loop(
            goal.id,
            agent_id,
            5,
            substrate.clone(),
            send,
            |_: String| async { None },
            |_| {},
            |_, _| async { Ok(false) },
            false,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
            fast_seed(3, Vec::new()),
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::MaxIterationsReached);
        assert_eq!(
            turns.load(Ordering::SeqCst),
            2,
            "resuming at 3 of a 5-iteration cap must leave 2 turns, not 5"
        );
    }

    /// A row whose `agent_id` cannot be parsed will never be resumed by
    /// anyone, so recovery must drop it rather than leave it to be re-reported
    /// as stale at every subsequent boot.
    #[test]
    fn recover_stale_run_drops_row_with_unparseable_agent_id() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let goal_id = GoalId::new();

        let stale_started = Utc::now() - chrono::Duration::seconds(3600);
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: "not-a-uuid".to_string(),
                phase: GoalRunPhase::Running.to_string(),
                iteration: 5,
                max_iterations: 25,
                last_progress: 50,
                last_error: None,
                started_at: stale_started.to_rfc3339(),
                updated_at: stale_started.to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());

        let recovered = runner.recover_stale_runs(Duration::from_secs(600));
        assert!(recovered.is_empty());
        assert!(store.get_run(&goal_id.to_string()).unwrap().is_none());
    }

    #[test]
    fn recover_skips_fresh_running_run() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();

        // A Running row that started just now — not stale.
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: agent_id.to_string(),
                phase: GoalRunPhase::Running.to_string(),
                iteration: 1,
                max_iterations: 25,
                last_progress: 10,
                last_error: None,
                started_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());
        let recovered = runner.recover_stale_runs(Duration::from_secs(600));
        assert!(recovered.is_empty(), "a fresh run must not be recovered");

        // Row stays Running, untouched.
        let row = store.get_run(&goal_id.to_string()).unwrap().unwrap();
        assert_eq!(row.phase, GoalRunPhase::Running.to_string());
        assert!(row.last_error.is_none());
    }

    // --- Concurrent-start atomicity (finding #8) ---

    /// Two `start()` calls racing on the same goal must never leave a second,
    /// orphaned loop running. Before the `start_lock` fix the non-atomic
    /// stop→spawn→insert let both racing calls pass their (no-op) stop while the
    /// slot was empty, spawn two loops, and have the second `insert` overwrite
    /// the first's handle — orphaning the first loop, which `stop()` could then
    /// never reach (it only aborts the currently-mapped generation) and which
    /// kept issuing agent turns invisibly.
    ///
    /// Detection: each turn registers its loop as "live" (an RAII guard that
    /// decrements on task abort) and then parks. After the racing starts settle,
    /// `stop()` cancels the single mapped run; if an orphan slipped through it is
    /// not in the map, so `stop()` cannot abort it and `live` never returns to
    /// zero. We do NOT assert a peak of one concurrent loop: `JoinHandle::abort`
    /// is asynchronous, so during a legitimate replace the outgoing loop can
    /// still be parked (live) when the incoming one registers — a transient the
    /// fix does not (and need not) eliminate. The load-bearing invariant is that
    /// no loop survives `stop()`. Repeated over many rounds because the race is
    /// timing-dependent; without the fix it manifests within a few rounds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_starts_never_leave_an_orphan_loop() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (_tx, rx) = watch::channel(false);
        let runner = Arc::new(GoalRunner::new(rx));

        for round in 0..30 {
            // Fresh counter + gate per round so state never leaks between them.
            let live = Arc::new(AtomicU64::new(0));
            let gate = Arc::new(tokio::sync::Notify::new());

            // Each turn registers the loop as live and then blocks forever on
            // `gate` (simulating a long agent turn). The RAII `Dec` guard is
            // held across the await, so an aborted loop still decrements `live`.
            let send = {
                let live = live.clone();
                let gate = gate.clone();
                move |_a: AgentId, _p: String| {
                    let live = live.clone();
                    let gate = gate.clone();
                    async move {
                        struct Dec(Arc<AtomicU64>);
                        impl Drop for Dec {
                            fn drop(&mut self) {
                                self.0.fetch_sub(1, Ordering::SeqCst);
                            }
                        }
                        live.fetch_add(1, Ordering::SeqCst);
                        let _dec = Dec(live.clone());
                        gate.notified().await;
                        Ok::<String, String>("GOAL_PROGRESS: 1".to_string())
                    }
                }
            };

            // Two genuinely-parallel starts (spawned, since `start()` is
            // synchronous — `join!` alone would run them sequentially).
            let r1 = runner.clone();
            let r2 = runner.clone();
            let s1 = send.clone();
            let s2 = send.clone();
            let sub1 = substrate.clone();
            let sub2 = substrate.clone();
            let h1 = tokio::spawn(async move {
                r1.start(
                    goal_id,
                    agent_id,
                    100,
                    sub1,
                    s1,
                    |_: String| async { None },
                    |_| {},
                    |_, _| async { Ok(false) }, // evaluate_goal
                    false,                      // loop_engineering
                    None,
                    None,
                    None, // evaluator_model
                    None, // tick_interval_secs
                );
            });
            let h2 = tokio::spawn(async move {
                r2.start(
                    goal_id,
                    agent_id,
                    100,
                    sub2,
                    s2,
                    |_: String| async { None },
                    |_| {},
                    |_, _| async { Ok(false) }, // evaluate_goal
                    false,                      // loop_engineering
                    None,
                    None,
                    None, // evaluator_model
                    None, // tick_interval_secs
                );
            });
            let _ = tokio::join!(h1, h2);

            // Wait for at least one loop to reach `send_message`, then give a
            // possible second (orphan) loop time to reach it too.
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while live.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;

            // Stop the (single, mapped) run. If an orphan exists it is not in
            // the map, so `stop()` cannot reach it and `live` never returns to 0.
            runner.stop(goal_id);
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while live.load(Ordering::SeqCst) != 0 && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }

            assert_eq!(
                live.load(Ordering::SeqCst),
                0,
                "round {round}: an orphaned goal loop survived stop()"
            );
        }
    }
}
