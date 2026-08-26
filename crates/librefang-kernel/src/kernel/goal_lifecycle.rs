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
use librefang_types::agent::{AgentId, AgentManifest, ModelConfig, SessionMode};
use librefang_types::goal::{GoalId, GoalRunState, DEFAULT_GOAL_MAX_ITERATIONS};

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
    ///
    /// Starting a goal that was paused (or interrupted by a crash) resumes it
    /// from its persisted checkpoint rather than restarting it; see
    /// [`crate::goal_runner::GoalRunner::start`].
    #[allow(clippy::too_many_arguments)] // 9-context-arg public API; grouping churns trait+callers
    pub fn goal_run_start(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
        loop_engineering: bool,
        verify_agent_id: Option<AgentId>,
        verify_max_retries: Option<u32>,
        evaluator_model: Option<String>,
        tick_interval_secs: Option<u32>,
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
                let sender = goal_tick_sender_context(aid, goal_id, SYSTEM_CHANNEL_AUTONOMOUS);
                match k.send_message_with_sender_context(aid, &msg, &sender).await {
                    Ok(r) => Ok(r.response),
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        // Sub-agent spawn closure for the loop. Gated on loop_engineering
        // inside the closure body (not via if/else) so both branches
        // return the same concrete type.
        let spawn_kernel = kernel.clone();
        let spawn_sub = move |task_name: String| {
            let k = spawn_kernel.clone();
            async move {
                if !loop_engineering {
                    return None;
                }
                let manifest = AgentManifest {
                    name: format!(
                        "goal-sub-{}",
                        uuid::Uuid::new_v4()
                            .to_string()
                            .split('-')
                            .next()
                            .unwrap_or("x")
                    ),
                    version: "0.1.0".into(),
                    description: format!("Auto-spawned sub-agent: {task_name}"),
                    author: "goal-runner".into(),
                    module: "builtin:chat".into(),
                    schedule: librefang_types::agent::ScheduleMode::Reactive,
                    session_mode: SessionMode::New,
                    model: ModelConfig {
                        // "default" resolves to the operator's actually-
                        // configured default provider/model (see messaging.rs's
                        // "default" sentinel handling), rather than requiring a
                        // specific provider's API key.
                        provider: "default".into(),
                        model: "default".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                };
                // spawn_agent is sync (not async).
                k.spawn_agent(manifest).ok()
            }
        };

        use librefang_skills::evolution::create_skill;

        // Clone before kernel is consumed by closures below.
        let goal_id_for_title = goal_id;
        let goal_title = {
            let substrate = self.substrate_ref();
            let arr = substrate
                .structured_get(
                    librefang_types::goal::goals_storage_agent_id(),
                    librefang_types::goal::GOALS_STORAGE_KEY,
                )
                .ok()
                .flatten()
                .unwrap_or(serde_json::Value::Array(vec![]));
            if let serde_json::Value::Array(arr) = arr {
                let target = goal_id_for_title.to_string();
                arr.into_iter()
                    .find(|g| g.get("id").and_then(|v| v.as_str()) == Some(target.as_str()))
                    .and_then(|g| g.get("title").and_then(|v| v.as_str()).map(String::from))
                    .unwrap_or_else(|| format!("Goal {goal_id_for_title}"))
            } else {
                format!("Goal {goal_id_for_title}")
            }
        };

        // Evaluator closure: asks a model (configurable or the agent itself)
        // whether the goal condition has been met, mirroring Claude Code's
        // /goal evaluator pattern. Never trust the agent's self-reported
        // GOAL_DONE alone.
        let eval_kernel = kernel.clone();
        let eval_model = evaluator_model.clone();
        let evaluate = move |goal_desc: String, agent_reply: String| {
            let k = eval_kernel.clone();
            let eval_model = eval_model.clone();
            async move {
                let model_hint = eval_model.as_deref().unwrap_or("agent-default");
                let prompt = format!(
                    "You are a goal evaluator (model: {model_hint}). Read the goal \
                     and the agent's latest output. Answer ONLY 'YES' if the goal \
                     is fully achieved, or 'NO' if more work is needed.\n\n\
                     GOAL: {goal_desc}\n\n\
                     AGENT OUTPUT:\n{agent_reply}\n\n\
                     Is the goal achieved? (YES/NO):"
                );
                if let Some(ref model_name) = eval_model {
                    // Use the configured evaluator model via one-shot LLM call.
                    match k.one_shot_llm_call(model_name, &prompt).await {
                        Ok(response) => Ok(evaluator_reply_is_yes(&response)),
                        Err(e) => Err(e),
                    }
                } else {
                    // Fallback: send to the agent itself for evaluation. Same
                    // per-goal scope as the tick above, so the evaluator turn
                    // lands in the goal's own session rather than a shared one.
                    let sender = goal_tick_sender_context(agent_id, goal_id, "goal-evaluator");
                    match k
                        .send_message_with_sender_context(agent_id, &prompt, &sender)
                        .await
                    {
                        Ok(r) => Ok(evaluator_reply_is_yes(&r.response)),
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };

        // Learnings callback: persist captured knowledge as an auto-created
        // skill so the agent self-evolves. Only when loop_engineering is on.
        let learnings_agent_id = agent_id;
        let skills_dir = self.home_dir().join("skills");
        let on_learnings = move |learnings: Vec<String>| {
            if !loop_engineering || learnings.is_empty() {
                return;
            }
            let body = format!(
                "## Learnings from goal run\n\n{}\n\n## Usage\n\
                 These patterns were discovered during autonomous execution of \
                 goal `{title}`. Apply them when solving similar tasks.",
                learnings
                    .iter()
                    .enumerate()
                    .map(|(i, l)| format!("{}. {l}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n"),
                title = goal_title,
            );
            let skill_name = format!(
                "goal-learned-{}",
                goal_title
                    .to_lowercase()
                    .replace(|c: char| !c.is_alphanumeric(), "-")
                    .trim_matches('-')
            );
            match create_skill(
                &skills_dir,
                &skill_name,
                &format!("Auto-discovered from goal: {goal_title}"),
                &body,
                vec!["goal-learned".into(), "auto-evolved".into()],
                Some("goal-runner"),
            ) {
                Ok(result) => {
                    tracing::info!(
                        agent = %learnings_agent_id,
                        goal_id = %goal_id,
                        count = learnings.len(),
                        skill = %result.skill_name,
                        "Goal runner: auto-created skill from captured learnings"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        agent = %learnings_agent_id,
                        goal_id = %goal_id,
                        error = %e,
                        "Goal runner: failed to auto-create skill from learnings"
                    );
                }
            }
        };

        self.workflows.goal_runner.start(
            goal_id,
            agent_id,
            max,
            substrate,
            send,
            spawn_sub,
            on_learnings,
            evaluate,
            loop_engineering,
            verify_agent_id,
            verify_max_retries,
            evaluator_model,
            tick_interval_secs,
        )
    }

    /// Cancel an active goal run, discarding its resume checkpoint.
    ///
    /// Terminal by design: the durable row is dropped, so starting the goal
    /// again begins from iteration 0. Use [`Self::goal_run_pause`] to suspend a
    /// run that should later continue where it left off.
    pub fn goal_run_stop(&self, goal_id: GoalId) -> bool {
        self.workflows.goal_runner.stop(goal_id)
    }

    /// Pause an active goal run, preserving its resume checkpoint.
    ///
    /// Returns whether a live run was signalled. The loop finishes its current
    /// turn, checkpoints iteration / progress / learnings, and exits in
    /// `Paused`; `goal_run_start` then resumes from there.
    pub fn goal_run_pause(&self, goal_id: GoalId) -> bool {
        self.workflows.goal_runner.pause(goal_id)
    }

    /// Snapshot the observable state of a goal's run, if one is active.
    pub fn goal_run_status(&self, goal_id: GoalId) -> Option<GoalRunState> {
        self.workflows.goal_runner.state(goal_id)
    }

    /// Recover goal runs interrupted by a prior crash or restart.
    ///
    /// Boot calls this once, mirroring the workflow stale-recovery sweep:
    /// persisted runs still in `Running` phase and older than `stale_timeout`
    /// are deleted from the durable store.
    /// Returns (goal_id, agent_id) pairs for stale runs to auto-resume.
    /// Caller must call `goal_run_start` for each returned pair.
    pub fn recover_stale_goal_runs(
        &self,
        stale_timeout: std::time::Duration,
    ) -> Vec<(GoalId, AgentId)> {
        self.workflows.goal_runner.recover_stale_runs(stale_timeout)
    }

    /// Load a persisted [`Goal`] by id from the shared goal store, if present.
    ///
    /// The autonomous-run config (`loop_engineering`, `verify_agent_id`,
    /// `evaluator_model`) lives on the `Goal`, not on the persisted run row, so
    /// auto-resume after a restart reads it back from here rather than
    /// defaulting to a plain loop.
    pub fn goal_by_id(&self, goal_id: GoalId) -> Option<librefang_types::goal::Goal> {
        let arr = self
            .substrate_ref()
            .structured_get(
                librefang_types::goal::goals_storage_agent_id(),
                librefang_types::goal::GOALS_STORAGE_KEY,
            )
            .ok()
            .flatten()?;
        let serde_json::Value::Array(arr) = arr else {
            return None;
        };
        let target = goal_id.to_string();
        arr.into_iter()
            .find(|g| g.get("id").and_then(|v| v.as_str()) == Some(target.as_str()))
            .and_then(|g| serde_json::from_value(g).ok())
    }
}

/// Build the [`SenderContext`] every goal-run turn is dispatched with.
///
/// Single source of truth for the two call sites in [`LibreFangKernel::goal_run_start`]
/// (the tick itself and the self-evaluation fallback) so they cannot drift on
/// the fields that decide which session the turn lands in.
///
/// ## Why `chat_id` carries the goal id
///
/// `send_message_full`'s channel branch derives the session as
/// `SessionId::for_sender_scope(agent, channel, chat_id)`, which collapses to
/// `for_channel(agent, "autonomous")` when `chat_id` is absent. Every goal of a
/// given agent therefore used to resolve to one single session: two goals
/// running at once interleaved their prompts into one conversation history,
/// and each read back the other's turns as its own context.
///
/// Scoping by goal id splits them without costing prompt-cache reuse. Cache
/// reuse depends on consecutive turns of *one* goal sharing a session prefix,
/// and they still do — the scope is a function of the goal, not of the tick, so
/// all of a goal's iterations land on the same id. What changes is only that a
/// *different* goal no longer lands on that same id. This is why the isolation
/// is unconditional rather than opt-in the way cron's `session_mode = "new"`
/// is: cron's flag trades cache reuse for per-*fire* isolation, a real
/// trade-off with a losing side. Per-*goal* isolation has no losing side, and
/// there is no coherent reason to want two unrelated goals sharing one history.
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

/// Parse an evaluator's free-text reply into a goal-achieved verdict.
///
/// The evaluator is prompted to answer "YES"/"NO", but models routinely wrap
/// the verdict in prose. Substring matching is wrong: "NOTHING left to do"
/// contains "NO", "ANNOUNCE" contains "NO", so `contains("NO")` misclassifies
/// them. Tokenize on non-alphabetic characters and look for a standalone
/// `YES`/`NO` word instead — achieved only when a `YES` token is present and no
/// `NO` token is.
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

#[cfg(test)]
mod goal_session_scope_tests {
    use super::*;
    use librefang_types::agent::SessionId;

    /// Reproduce the session id `send_message_full` derives for a goal tick.
    ///
    /// Mirrors the channel branch of `messaging.rs::send_message_full`
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
    /// landing on the same session, or the provider prompt cache is destroyed
    /// and the agent loses its own turn-to-turn context mid-run.
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

    /// The self-evaluation fallback turn is part of the same goal run, so it
    /// must share the tick's session — a different `display_name` must not
    /// fan out a second session.
    #[test]
    fn evaluator_fallback_shares_the_goal_tick_session() {
        let agent = AgentId::new();
        let goal = GoalId::new();

        let tick = goal_tick_sender_context(agent, goal, SYSTEM_CHANNEL_AUTONOMOUS);
        let evaluator = goal_tick_sender_context(agent, goal, "goal-evaluator");

        assert_eq!(
            derived_session_id(&tick, agent),
            derived_session_id(&evaluator, agent),
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

#[cfg(test)]
mod evaluator_parse_tests {
    use super::evaluator_reply_is_yes;

    #[test]
    fn plain_verdicts() {
        assert!(evaluator_reply_is_yes("YES"));
        assert!(evaluator_reply_is_yes("yes"));
        assert!(!evaluator_reply_is_yes("NO"));
        assert!(!evaluator_reply_is_yes("no"));
    }

    #[test]
    fn verdict_wrapped_in_prose() {
        assert!(evaluator_reply_is_yes("YES, the goal is fully achieved."));
        assert!(!evaluator_reply_is_yes("NO, more work is needed."));
        assert!(evaluator_reply_is_yes("The answer is: yes."));
    }

    #[test]
    fn words_containing_no_do_not_count_as_no() {
        // These all contain the substring "NO" but no standalone NO token.
        assert!(evaluator_reply_is_yes(
            "YES. There is nothing more to do; the agent cannot improve it."
        ));
        assert!(evaluator_reply_is_yes("YES — ready to announce."));
    }

    #[test]
    fn no_token_wins_over_yes() {
        // Conservative: an explicit NO anywhere blocks completion.
        assert!(!evaluator_reply_is_yes("Yes and no — NO, not done."));
    }

    #[test]
    fn no_verdict_defaults_to_not_achieved() {
        assert!(!evaluator_reply_is_yes("I am not sure."));
        assert!(!evaluator_reply_is_yes(""));
    }
}
