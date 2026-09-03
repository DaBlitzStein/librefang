//! Kernel-side wiring for the autonomous goal runner (#5744).
//!
//! Bridges the standalone [`crate::goal_runner::GoalRunner`] to the live agent
//! send path: each goal-run tick is an autonomous agent turn driven through
//! `send_message_with_sender_context` with the reserved `"autonomous"` channel
//! sentinel (same RBAC carve-out as the continuous / cron background loops).
//!
//! These are inherent helpers; the `KernelApi` trait methods (`start_goal_run`
//! etc.) delegate here so the HTTP layer can reach them through
//! `Arc<dyn KernelApi>`.

use librefang_channels::types::SenderContext;
use librefang_skills::evolution::{create_skill, load_installed_skill_from_disk, update_skill};
use librefang_skills::SkillError;
use librefang_types::agent::AgentId;
use librefang_types::goal::{
    goals_storage_agent_id, Goal, GoalId, GoalRunState, DEFAULT_GOAL_MAX_ITERATIONS,
    GOALS_STORAGE_KEY,
};

use super::{LibreFangKernel, SYSTEM_CHANNEL_AUTONOMOUS};
use crate::MemorySubsystemApi;

impl LibreFangKernel {
    /// Start an autonomous run that drives `agent_id` toward `goal_id`.
    ///
    /// Each tick is a full agent turn; the runner parses the agent's reply for
    /// `GOAL_PROGRESS:` / `GOAL_DONE` markers and updates the goal until it is
    /// complete, the iteration cap (`max_iterations`, default
    /// [`DEFAULT_GOAL_MAX_ITERATIONS`]) is reached, an operator stops it, or the
    /// kernel shuts down.
    #[allow(clippy::too_many_arguments)]
    pub fn goal_run_start(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
        loop_engineering: bool,
        verify_agent_id: Option<AgentId>,
        verify_max_retries: Option<u32>,
        evaluator_model: Option<String>,
    ) -> bool {
        let max = max_iterations.unwrap_or(DEFAULT_GOAL_MAX_ITERATIONS).max(1);
        let substrate = self.substrate_ref().clone();

        // The tick closure drives a real agent turn, which needs an owned
        // `Arc<LibreFangKernel>`. Upgrade the self-handle (set right after the
        // kernel is wrapped in `Arc` at boot).
        let kernel = match self.self_handle.get().and_then(|w| w.upgrade()) {
            Some(k) => k,
            None => {
                tracing::warn!(%goal_id, "Cannot start goal run: kernel self-handle unset");
                return false;
            }
        };

        let send_kernel = kernel.clone();
        let send = move |aid: AgentId, msg: String| {
            let k = send_kernel.clone();
            async move {
                // Trusted internal system path — reuse the autonomous-channel
                // sentinel so the RBAC resolver applies the system carve-out
                // (see background_lifecycle.rs).
                let sender = goal_tick_sender_context(aid, goal_id, SYSTEM_CHANNEL_AUTONOMOUS);
                match k.send_message_with_sender_context(aid, &msg, &sender).await {
                    Ok(r) => Ok(r.response),
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        // Completion judge. A one-shot call on a model of the operator's
        // choosing, deliberately NOT a turn on the goal's own agent: routing it
        // there would both bill a full agent turn and put the worker back in
        // charge of grading itself. With no model configured the runner never
        // calls this, so the arm returning `Ok(false)` exists only to give the
        // closure a single concrete type.
        let eval_kernel = kernel.clone();
        let eval_model = evaluator_model.clone();
        let evaluate = move |goal_description: String, output: String| {
            let k = eval_kernel.clone();
            let eval_model = eval_model.clone();
            async move {
                let Some(model) = eval_model else {
                    return Ok(false);
                };
                let prompt = format!(
                    "You are judging whether a goal has been achieved. Read the goal and \
                     the worker's latest output, then answer with the single word YES or \
                     NO — YES only if the goal is fully achieved, NO if any work remains.\
                     \n\nGOAL: {goal_description}\n\nLATEST OUTPUT:\n{output}\n\nAchieved?"
                );
                k.one_shot_llm_call(&model, &prompt)
                    .await
                    .map(|reply| evaluator_reply_is_yes(&reply))
            }
        };

        // Lessons the run captured become a skill, so the next goal starts from
        // them instead of rediscovering them. `create_skill` runs the same
        // prompt-injection scan as every other skill-creation path, which is
        // the boundary that matters here: the body is model-authored text.
        let skills_dir = self.home_dir().join("skills");
        let goal_title = self
            .goal_by_id(goal_id)
            .map(|g| g.title)
            .unwrap_or_else(|| format!("Goal {goal_id}"));
        let on_learnings = move |learnings: Vec<String>| {
            persist_learnings_as_skill(&skills_dir, goal_id, &goal_title, &learnings);
        };

        self.workflows.goal_runner.start(
            goal_id,
            agent_id,
            max,
            substrate,
            send,
            on_learnings,
            evaluate,
            loop_engineering,
            verify_agent_id,
            verify_max_retries,
            evaluator_model,
        );
        true
    }

    /// Load a persisted [`Goal`] by id from the shared goals document.
    pub fn goal_by_id(&self, goal_id: GoalId) -> Option<Goal> {
        let Ok(Some(serde_json::Value::Array(arr))) = self
            .substrate_ref()
            .structured_get(goals_storage_agent_id(), GOALS_STORAGE_KEY)
        else {
            return None;
        };
        let target = goal_id.to_string();
        arr.into_iter()
            .find(|g| g.get("id").and_then(|v| v.as_str()) == Some(target.as_str()))
            .and_then(|g| serde_json::from_value(g).ok())
    }

    /// Stop an active goal run. Returns whether a run was stopped.
    ///
    /// Terminal: discards any resume checkpoint, so starting the goal again
    /// begins from iteration 0. Use [`Self::goal_run_pause`] to suspend a run
    /// that should later continue where it left off.
    pub fn goal_run_stop(&self, goal_id: GoalId) -> bool {
        self.workflows.goal_runner.stop(goal_id)
    }

    /// Pause an active goal run, checkpointing its iteration count and
    /// progress. Returns whether a live run was signalled.
    ///
    /// The loop finishes the turn it is on before checkpointing and exiting
    /// in [`librefang_types::goal::GoalRunPhase::Paused`], so a `true` return
    /// means the pause was accepted, not that the run has already stopped —
    /// poll [`Self::goal_run_status`] for the phase to reach `Paused`.
    pub fn goal_run_pause(&self, goal_id: GoalId) -> bool {
        self.workflows.goal_runner.pause(goal_id)
    }

    /// Resume a previously-paused goal run from its checkpoint.
    ///
    /// Identical to [`Self::goal_run_start`] — `GoalRunner::start` auto-detects
    /// and resumes from a pause checkpoint when one exists, so this is the
    /// same start path. Callers that want to refuse a resume when there is no
    /// checkpoint (rather than silently starting a fresh run) should check
    /// [`Self::goal_run_status`] for [`librefang_types::goal::GoalRunPhase::Paused`]
    /// before calling.
    ///
    /// The loop-engineering configuration is read back off the goal document
    /// rather than taken from the caller, so a resumed run is verified exactly
    /// like the run it continues. Deriving it here rather than at the HTTP
    /// boundary is what stops a resume from silently dropping the verifier
    /// gate: every caller gets the operator's configuration without having to
    /// remember to forward it.
    pub fn goal_run_resume(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
    ) -> bool {
        let goal = self.goal_by_id(goal_id);
        let loop_engineering = goal.as_ref().is_some_and(|g| g.loop_engineering);
        let verify_agent_id = goal
            .as_ref()
            .and_then(|g| g.verify_agent_id)
            .filter(|_| loop_engineering);
        let evaluator_model = goal
            .as_ref()
            .and_then(|g| g.evaluator_model.clone())
            .filter(|_| loop_engineering);
        self.goal_run_start(
            goal_id,
            agent_id,
            max_iterations,
            loop_engineering,
            verify_agent_id,
            // The per-run retry budget is a property of the request that
            // started the run; a resume carries no body of its own, so it
            // takes the default.
            None,
            evaluator_model,
        )
    }

    /// Snapshot the observable state of a goal's run, if one is active.
    pub fn goal_run_status(&self, goal_id: GoalId) -> Option<GoalRunState> {
        self.workflows.goal_runner.state(goal_id)
    }

    /// Recover goal runs interrupted by a prior crash or restart.
    ///
    /// Boot calls this once, mirroring the workflow stale-recovery sweep:
    /// persisted runs still in `Running` phase and older than `stale_timeout`
    /// are demoted to `Stopped` ("Interrupted by daemon restart"). Runs are not
    /// auto-resumed — an in-flight LLM call cannot be replayed. Returns the
    /// recovered goal ids.
    pub fn recover_stale_goal_runs(&self, stale_timeout: std::time::Duration) -> Vec<GoalId> {
        self.workflows.goal_runner.recover_stale_runs(stale_timeout)
    }
}

/// Read an evaluator's free-text reply as achieved / not achieved.
///
/// The model is asked for a bare YES or NO and routinely wraps it in prose, so
/// the verdict has to be found rather than compared. Substring matching is the
/// wrong tool: "NOTHING left to do" and "ready to ANNOUNCE" both contain "no".
/// Split on non-letters and look for a standalone `YES` / `NO` token instead.
///
/// An explicit `NO` anywhere beats a `YES`, and a reply that reaches no verdict
/// at all is not achieved — the conservative direction, since the cost of a
/// false "not yet" is one more iteration and the cost of a false "done" is a
/// goal closed on unfinished work.
fn evaluator_reply_is_yes(reply: &str) -> bool {
    let mut saw_yes = false;
    let mut saw_no = false;
    for token in reply.split(|c: char| !c.is_ascii_alphabetic()) {
        match token.to_ascii_uppercase().as_str() {
            "YES" => saw_yes = true,
            "NO" => saw_no = true,
            _ => {}
        }
    }
    saw_yes && !saw_no
}

/// Turn a goal title into a skill-name slug.
///
/// `create_skill` accepts `[a-z0-9_-]` starting alphanumeric, up to 64 chars,
/// so a title in a non-Latin script or one made entirely of punctuation slugs
/// down to nothing. The goal id is appended in all cases: it keeps the name
/// unique across goals that share a title, and it is what the name falls back
/// to when the slug is empty.
fn learned_skill_name(goal_id: GoalId, goal_title: &str) -> String {
    const MAX_SLUG: usize = 24;
    let mut slug = String::with_capacity(MAX_SLUG);
    let mut last_was_dash = false;
    for c in goal_title.chars() {
        if slug.len() >= MAX_SLUG {
            break;
        }
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !slug.is_empty() && !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    // The id fragment is hex, so the name always starts alphanumeric even when
    // the slug contributes nothing.
    let id_fragment: String = goal_id
        .to_string()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if slug.is_empty() {
        format!("goal-learned-{id_fragment}")
    } else {
        format!("goal-learned-{slug}-{id_fragment}")
    }
}

/// Render a run's lessons as a skill body.
fn learned_skill_body(goal_title: &str, learnings: &[String]) -> String {
    let mut body =
        String::from("## What this is\n\nLessons captured while autonomously pursuing the goal \"");
    body.push_str(goal_title);
    body.push_str("\". They are the run's own findings, not vetted guidance — weigh them as such.\n\n## Lessons\n\n");
    for (i, l) in learnings.iter().enumerate() {
        body.push_str(&format!("{}. {l}\n", i + 1));
    }
    body
}

/// Write a run's lessons into a skill, creating it or refreshing an existing
/// one.
///
/// A goal that is run more than once accumulates lessons across runs, so the
/// second run must update the skill the first one wrote rather than fail on
/// the name already existing and silently drop everything it learned. The
/// durable record is the runner's own `goal_learnings_<id>` store entry; this
/// is the copy the agent can actually read.
fn persist_learnings_as_skill(
    skills_dir: &std::path::Path,
    goal_id: GoalId,
    goal_title: &str,
    learnings: &[String],
) {
    if learnings.is_empty() {
        return;
    }
    let name = learned_skill_name(goal_id, goal_title);
    let body = learned_skill_body(goal_title, learnings);
    let description = format!("Lessons captured while pursuing the goal: {goal_title}");
    let created = create_skill(
        skills_dir,
        &name,
        &description,
        &body,
        vec!["goal-learned".to_string()],
        Some("goal-runner"),
    );
    match created {
        Ok(_) => {
            tracing::info!(%goal_id, skill = %name, count = learnings.len(),
                           "Goal run: captured lessons into a new skill");
        }
        Err(SkillError::AlreadyInstalled(_)) => {
            match load_installed_skill_from_disk(skills_dir, &name).and_then(|skill| {
                update_skill(
                    &skill,
                    &body,
                    "Lessons from a later goal run",
                    Some("goal-runner"),
                )
            }) {
                Ok(_) => tracing::info!(%goal_id, skill = %name, count = learnings.len(),
                                        "Goal run: refreshed the captured-lessons skill"),
                Err(e) => tracing::warn!(%goal_id, skill = %name, error = %e,
                                         "Goal run: failed to refresh the captured-lessons skill"),
            }
        }
        Err(e) => {
            // Most likely the prompt-injection scan rejecting model-authored
            // text, which is the scan doing its job. The lessons are still in
            // the runner's durable store either way.
            tracing::warn!(%goal_id, skill = %name, error = %e,
                           "Goal run: failed to capture lessons into a skill");
        }
    }
}

/// Build the [`SenderContext`] a goal-run tick is dispatched with.
///
/// ## Why `chat_id` carries the goal id
///
/// `send_message_full`'s channel branch derives the session as
/// `SessionId::for_sender_scope(agent, channel, chat_id)`, which collapses to
/// `for_channel(agent, "autonomous")` when `chat_id` is absent. Every goal of
/// a given agent would then resolve to one single session: two goals running
/// concurrently would interleave their prompts into one conversation history,
/// and each would read back the other's turns as its own context.
///
/// Scoping by goal id splits them without costing prompt-cache reuse: cache
/// reuse depends on consecutive turns of *one* goal sharing a session prefix,
/// and they still do, since the scope is a function of the goal rather than
/// of the tick. What changes is only that a *different* goal no longer lands
/// on that same id.
fn goal_tick_sender_context(
    agent_id: AgentId,
    goal_id: GoalId,
    display_name: &str,
) -> SenderContext {
    SenderContext {
        channel: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
        user_id: agent_id.to_string(),
        chat_id: Some(goal_id.to_string()),
        display_name: display_name.to_string(),
        is_internal_system: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluator_verdict_reads_a_bare_yes_or_no() {
        assert!(evaluator_reply_is_yes("YES"));
        assert!(evaluator_reply_is_yes("yes"));
        assert!(!evaluator_reply_is_yes("NO"));
        assert!(!evaluator_reply_is_yes("no"));
    }

    #[test]
    fn evaluator_verdict_survives_the_prose_models_wrap_it_in() {
        assert!(evaluator_reply_is_yes("YES, the goal is fully achieved."));
        assert!(!evaluator_reply_is_yes("NO, more work is needed."));
        assert!(evaluator_reply_is_yes("The answer is: yes."));
    }

    /// Substring matching would read all of these as "NO".
    #[test]
    fn a_word_merely_containing_no_is_not_a_verdict() {
        assert!(evaluator_reply_is_yes(
            "YES. There is nothing more to do here."
        ));
        assert!(evaluator_reply_is_yes("YES - ready to announce."));
        assert!(evaluator_reply_is_yes("Yes, the report is now complete."));
    }

    /// Closing a goal on unfinished work costs more than one extra iteration,
    /// so ambiguity resolves to "not yet".
    #[test]
    fn an_ambiguous_or_absent_verdict_is_not_achieved() {
        assert!(!evaluator_reply_is_yes("Yes and no - NO, not done."));
        assert!(!evaluator_reply_is_yes("I am not sure."));
        assert!(!evaluator_reply_is_yes(""));
    }

    #[test]
    fn skill_name_slugs_the_goal_title() {
        let id = GoalId::new();
        let name = learned_skill_name(id, "Ship the Q3 Report!");
        assert!(
            name.starts_with("goal-learned-ship-the-q3-report-"),
            "got {name}"
        );
    }

    /// `create_skill` only accepts `[a-z0-9_-]` starting alphanumeric, and a
    /// title in a non-Latin script or made of punctuation slugs to nothing.
    #[test]
    fn skill_name_stays_valid_for_a_title_that_slugs_to_nothing() {
        for title in ["", "!!!", "目标", "---"] {
            let name = learned_skill_name(GoalId::new(), title);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{title:?} produced an invalid skill name: {name}"
            );
            assert!(!name.ends_with('-'), "{title:?} produced {name}");
            assert!(name.len() <= 64, "{title:?} produced {name}");
        }
    }

    #[test]
    fn skill_name_is_bounded_for_a_very_long_title() {
        let name = learned_skill_name(GoalId::new(), &"extremely wordy title ".repeat(20));
        assert!(name.len() <= 64, "got {} chars: {name}", name.len());
    }

    #[test]
    fn skill_name_distinguishes_goals_that_share_a_title() {
        let a = learned_skill_name(GoalId::new(), "Same title");
        let b = learned_skill_name(GoalId::new(), "Same title");
        assert_ne!(a, b);
    }

    #[test]
    fn skill_body_lists_every_lesson_under_the_goal_title() {
        let body = learned_skill_body(
            "Ship the report",
            &[
                "Back off before retrying".to_string(),
                "Cite sources".to_string(),
            ],
        );
        assert!(body.contains("Ship the report"));
        assert!(body.contains("1. Back off before retrying"));
        assert!(body.contains("2. Cite sources"));
    }
}

#[cfg(test)]
mod goal_session_scope_tests {
    use super::*;
    use librefang_types::agent::SessionId;

    /// Reproduce the session id `send_message_full` derives for a goal tick.
    /// Mirrors the channel branch of `messaging.rs::send_message_full_inner`
    /// verbatim — `resolve_scope_channel` then `SessionId::for_sender_scope`
    /// — so this asserts against the real derivation rather than a local
    /// re-statement of it.
    fn derived_session_id(ctx: &SenderContext, agent_id: AgentId) -> SessionId {
        let scope = LibreFangKernel::resolve_scope_channel(&ctx.channel, ctx.is_internal_system);
        SessionId::for_sender_scope(agent_id, &scope, ctx.chat_id.as_deref())
    }

    /// Two loop-mode goals driven by the SAME agent must not share a session.
    ///
    /// Before the fix every goal tick synthesized `chat_id: None`, collapsing
    /// to `SessionId::for_channel(agent, "autonomous")` — so two concurrent
    /// goal runs interleaved their prompts into one conversation history.
    #[test]
    fn two_goals_on_one_agent_do_not_share_a_session() {
        let agent = AgentId::new();
        let goal_a = GoalId::new();
        let goal_b = GoalId::new();

        let ctx_a = goal_tick_sender_context(agent, goal_a, SYSTEM_CHANNEL_AUTONOMOUS);
        let ctx_b = goal_tick_sender_context(agent, goal_b, SYSTEM_CHANNEL_AUTONOMOUS);

        assert_ne!(
            derived_session_id(&ctx_a, agent),
            derived_session_id(&ctx_b, agent),
            "two goals of the same agent resolved to one session — their \
             prompts interleave in a single conversation history"
        );
    }

    /// Isolation is per GOAL, not per tick: every tick of one goal must keep
    /// landing on the same session, or turn-to-turn context and the provider
    /// prompt cache are both destroyed mid-run.
    #[test]
    fn repeated_ticks_of_one_goal_share_its_session() {
        let agent = AgentId::new();
        let goal = GoalId::new();

        let first = goal_tick_sender_context(agent, goal, SYSTEM_CHANNEL_AUTONOMOUS);
        let second = goal_tick_sender_context(agent, goal, SYSTEM_CHANNEL_AUTONOMOUS);

        assert_eq!(
            derived_session_id(&first, agent),
            derived_session_id(&second, agent),
        );
    }

    /// The same goal id under two different agents stays separate — the agent
    /// dimension is still part of the key.
    #[test]
    fn one_goal_across_two_agents_does_not_share_a_session() {
        let goal = GoalId::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        let ctx_a = goal_tick_sender_context(agent_a, goal, SYSTEM_CHANNEL_AUTONOMOUS);
        let ctx_b = goal_tick_sender_context(agent_b, goal, SYSTEM_CHANNEL_AUTONOMOUS);

        assert_ne!(
            derived_session_id(&ctx_a, agent_a),
            derived_session_id(&ctx_b, agent_b),
        );
    }
}
