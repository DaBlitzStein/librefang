//! Goals screen: browse, create, manage, and run goals.

use crate::tui::{theme, widgets};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, ListItem, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct GoalInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub progress: u8,
    pub agent_id: Option<String>,
    pub loop_engineering: bool,
    pub verify_agent_id: Option<String>,
    pub evaluator_model: Option<String>,
    /// Seconds the runner waits between loop turns. `None` means the goal has
    /// no override and runs at [`DEFAULT_GOAL_TICK_INTERVAL_SECS`].
    pub tick_interval_secs: Option<u32>,
    pub run_phase: Option<String>,
    pub run_iteration: Option<u32>,
    pub run_max_iterations: Option<u32>,
}

impl GoalInfo {
    /// Whether the loop is actively driving the agent right now.
    ///
    /// Read off the live run state, never off `status`: a goal's stored status
    /// is flipped to `in_progress` when a run starts and is *not* reset when
    /// the run pauses or stops, so it cannot tell those three apart.
    pub fn is_running(&self) -> bool {
        self.run_phase.as_deref() == Some("running")
    }

    /// Whether the run is suspended at a checkpoint it can continue from.
    pub fn is_paused(&self) -> bool {
        self.run_phase.as_deref() == Some("paused")
    }

    /// Whether there is a run to act on — running or parked at a checkpoint.
    /// Every other phase is settled and has nothing left to stop.
    pub fn has_live_run(&self) -> bool {
        self.is_running() || self.is_paused()
    }
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct GoalsState {
    pub goals: Vec<GoalInfo>,
    pub filtered: Vec<usize>,
    pub list_state: ratatui::widgets::ListState,
    pub search_buf: String,
    pub search_mode: bool,
    pub loading: bool,
    pub tick: usize,
    pub detail_open: bool,
    pub selected_goal: Option<usize>,
    pub create_open: bool,
    pub create_step: usize,
    pub create_title: String,
    pub create_desc: String,
    pub create_agent_id: String,
    pub create_loop_engineering: bool,
    pub create_verify_agent_id: String,
    pub create_evaluator_model: String,
    /// Raw text of the create wizard's cadence field. Blank is legal and means
    /// "no override".
    pub create_tick_interval: String,
    /// Validation message for the step the wizard is on, cleared on every edit.
    pub create_error: String,
    /// The cadence editor for an existing goal (`c`), reachable from the list
    /// and from the detail pane.
    pub cadence_open: bool,
    pub cadence_buf: String,
    pub cadence_error: String,
    pub status_msg: String,
    pub confirm_delete: bool,
}

/// Fields the create wizard walks through, in order.
const CREATE_STEPS: usize = 7;
/// Index of the last field; Enter here submits rather than advancing.
const CREATE_LAST_STEP: usize = CREATE_STEPS - 1;
/// The wizard step that toggles auto-review instead of taking text.
const CREATE_TOGGLE_STEP: usize = 3;
/// The wizard step that takes the loop cadence.
const CREATE_CADENCE_STEP: usize = 6;

#[allow(dead_code)]
pub enum GoalsAction {
    Continue,
    Refresh,
    CreateGoal {
        title: String,
        description: String,
        agent_id: String,
        loop_engineering: bool,
        verify_agent_id: String,
        evaluator_model: String,
        tick_interval_secs: Option<u32>,
    },
    StartRun {
        goal_id: String,
    },
    /// Suspend a running loop, keeping its checkpoint. Distinct from
    /// [`GoalsAction::StopRun`], which discards it.
    PauseRun {
        goal_id: String,
    },
    /// Continue a paused loop from its checkpoint.
    ResumeRun {
        goal_id: String,
    },
    StopRun {
        goal_id: String,
    },
    /// Persist a goal's cadence override. `None` clears it back to the default.
    SetCadence {
        goal_id: String,
        tick_interval_secs: Option<u32>,
    },
    DeleteGoal {
        goal_id: String,
    },
    ShowDetail {
        goal_id: String,
    },
}

impl GoalsState {
    pub fn new() -> Self {
        Self {
            goals: Vec::new(),
            filtered: Vec::new(),
            list_state: ratatui::widgets::ListState::default(),
            search_buf: String::new(),
            search_mode: false,
            loading: false,
            tick: 0,
            detail_open: false,
            selected_goal: None,
            create_open: false,
            create_step: 0,
            create_title: String::new(),
            create_desc: String::new(),
            create_agent_id: String::new(),
            create_loop_engineering: false,
            create_verify_agent_id: String::new(),
            create_evaluator_model: String::new(),
            create_tick_interval: String::new(),
            create_error: String::new(),
            cadence_open: false,
            cadence_buf: String::new(),
            cadence_error: String::new(),
            status_msg: String::new(),
            confirm_delete: false,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn refilter(&mut self) {
        if self.search_buf.is_empty() {
            self.filtered = (0..self.goals.len()).collect();
        } else {
            let q = self.search_buf.to_lowercase();
            self.filtered = self
                .goals
                .iter()
                .enumerate()
                .filter(|(_, g)| {
                    g.title.to_lowercase().contains(&q)
                        || g.agent_id
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> GoalsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return GoalsAction::Continue;
        }

        if self.create_open {
            return self.handle_create_key(key);
        }

        // The cadence editor is checked before the detail pane: it opens over
        // either surface and Esc returns to whichever one was underneath.
        if self.cadence_open {
            return self.handle_cadence_key(key);
        }

        if self.detail_open {
            return self.handle_detail_key(key);
        }

        if self.search_mode {
            match key.code {
                KeyCode::Esc => {
                    self.search_mode = false;
                    self.search_buf.clear();
                    self.refilter();
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                }
                KeyCode::Backspace => {
                    self.search_buf.pop();
                    self.refilter();
                }
                KeyCode::Char(c) => {
                    self.search_buf.push(c);
                    self.refilter();
                }
                _ => {}
            }
            return GoalsAction::Continue;
        }

        if self.confirm_delete {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_delete = false;
                    if let Some(sel) = self.list_state.selected() {
                        if let Some(&idx) = self.filtered.get(sel) {
                            let id = self.goals[idx].id.clone();
                            return GoalsAction::DeleteGoal { goal_id: id };
                        }
                    }
                }
                _ => {
                    self.confirm_delete = false;
                }
            }
            return GoalsAction::Continue;
        }

        let total = self.filtered.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.list_state.select(Some(next));
            }
            KeyCode::Enter => {
                if let Some(sel) = self.list_state.selected() {
                    if let Some(&idx) = self.filtered.get(sel) {
                        self.selected_goal = Some(idx);
                        self.detail_open = true;
                        return GoalsAction::ShowDetail {
                            goal_id: self.goals[idx].id.clone(),
                        };
                    }
                }
            }
            KeyCode::Char('n') => {
                self.create_open = true;
                self.create_step = 0;
                self.create_title.clear();
                self.create_desc.clear();
                self.create_agent_id.clear();
                self.create_loop_engineering = false;
                self.create_verify_agent_id.clear();
                self.create_evaluator_model.clear();
                self.create_tick_interval.clear();
                self.create_error.clear();
            }
            KeyCode::Char('d') if self.list_state.selected().is_some() => {
                self.confirm_delete = true;
            }
            KeyCode::Char('s') | KeyCode::Char('p') | KeyCode::Char('x') | KeyCode::Char('c') => {
                let idx = self
                    .list_state
                    .selected()
                    .and_then(|sel| self.filtered.get(sel).copied());
                if let Some(idx) = idx {
                    return self.run_control(idx, key.code);
                }
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_buf.clear();
            }
            KeyCode::Char('r') => return GoalsAction::Refresh,
            _ => {}
        }
        GoalsAction::Continue
    }

    fn handle_detail_key(&mut self, key: KeyEvent) -> GoalsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.detail_open = false;
            }
            KeyCode::Char('s') | KeyCode::Char('p') | KeyCode::Char('x') | KeyCode::Char('c') => {
                if let Some(idx) = self.selected_goal {
                    if idx < self.goals.len() {
                        return self.run_control(idx, key.code);
                    }
                }
            }
            KeyCode::Char('r') => return GoalsAction::Refresh,
            _ => {}
        }
        GoalsAction::Continue
    }

    /// Map a run-control key onto the goal at `idx`.
    ///
    /// Start, pause, resume and stop each get their own key rather than one
    /// toggle: pause and stop are different outcomes for the same run (a
    /// checkpoint kept versus discarded), and a single key cannot offer both.
    /// A key that does not apply to the goal's current phase reports why
    /// instead of silently doing nothing.
    fn run_control(&mut self, idx: usize, code: KeyCode) -> GoalsAction {
        let g = &self.goals[idx];
        let goal_id = g.id.clone();
        match code {
            KeyCode::Char('s') if g.is_paused() => GoalsAction::ResumeRun { goal_id },
            KeyCode::Char('s') if g.is_running() => {
                self.status_msg = crate::i18n::t("tui-goals-already-running");
                GoalsAction::Continue
            }
            KeyCode::Char('s') => GoalsAction::StartRun { goal_id },
            KeyCode::Char('p') if g.is_running() => GoalsAction::PauseRun { goal_id },
            KeyCode::Char('p') => {
                self.status_msg = crate::i18n::t(if g.is_paused() {
                    "tui-goals-already-paused"
                } else {
                    "tui-goals-not-running"
                });
                GoalsAction::Continue
            }
            KeyCode::Char('x') if g.has_live_run() => GoalsAction::StopRun { goal_id },
            KeyCode::Char('x') => {
                self.status_msg = crate::i18n::t("tui-goals-nothing-to-stop");
                GoalsAction::Continue
            }
            KeyCode::Char('c') => {
                self.cadence_open = true;
                self.cadence_buf = g
                    .tick_interval_secs
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                self.cadence_error.clear();
                self.selected_goal = Some(idx);
                GoalsAction::Continue
            }
            _ => GoalsAction::Continue,
        }
    }

    /// Cadence editor for an existing goal: one field, Enter saves, Esc cancels.
    fn handle_cadence_key(&mut self, key: KeyEvent) -> GoalsAction {
        match key.code {
            KeyCode::Esc => {
                self.cadence_open = false;
                self.cadence_error.clear();
            }
            KeyCode::Enter => match parse_cadence_input(&self.cadence_buf) {
                Ok(tick_interval_secs) => {
                    let goal_id = self
                        .selected_goal
                        .and_then(|i| self.goals.get(i))
                        .map(|g| g.id.clone());
                    self.cadence_open = false;
                    self.cadence_error.clear();
                    if let Some(goal_id) = goal_id {
                        return GoalsAction::SetCadence {
                            goal_id,
                            tick_interval_secs,
                        };
                    }
                }
                Err(msg) => self.cadence_error = msg,
            },
            KeyCode::Backspace => {
                self.cadence_buf.pop();
                self.cadence_error.clear();
            }
            KeyCode::Char(c) => {
                self.cadence_buf.push(c);
                self.cadence_error.clear();
            }
            _ => {}
        }
        GoalsAction::Continue
    }

    fn handle_create_key(&mut self, key: KeyEvent) -> GoalsAction {
        match key.code {
            KeyCode::Esc => {
                self.create_error.clear();
                if self.create_step == 0 {
                    self.create_open = false;
                } else {
                    self.create_step -= 1;
                }
            }
            KeyCode::Enter => {
                // The cadence is the only field with a range to honour, and it
                // is checked before the request so a typo comes back on the
                // field the operator is looking at rather than as a 400.
                let cadence = match parse_cadence_input(&self.create_tick_interval) {
                    Ok(v) => v,
                    Err(msg) => {
                        self.create_error = msg;
                        self.create_step = CREATE_CADENCE_STEP;
                        return GoalsAction::Continue;
                    }
                };
                self.create_error.clear();
                if self.create_step >= CREATE_LAST_STEP {
                    let action = GoalsAction::CreateGoal {
                        title: self.create_title.clone(),
                        description: self.create_desc.clone(),
                        agent_id: self.create_agent_id.clone(),
                        loop_engineering: self.create_loop_engineering,
                        verify_agent_id: self.create_verify_agent_id.clone(),
                        evaluator_model: self.create_evaluator_model.clone(),
                        tick_interval_secs: cadence,
                    };
                    self.create_open = false;
                    return action;
                }
                self.create_step += 1;
            }
            KeyCode::Tab | KeyCode::Char(' ') if self.create_step == CREATE_TOGGLE_STEP => {
                self.create_loop_engineering = !self.create_loop_engineering;
            }
            KeyCode::Char(c) => {
                self.create_error.clear();
                match self.create_step {
                    0 => self.create_title.push(c),
                    1 => self.create_desc.push(c),
                    2 => self.create_agent_id.push(c),
                    CREATE_TOGGLE_STEP => {} // toggle, no text input
                    4 => self.create_verify_agent_id.push(c),
                    5 => self.create_evaluator_model.push(c),
                    CREATE_CADENCE_STEP => self.create_tick_interval.push(c),
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                self.create_error.clear();
                match self.create_step {
                    0 => {
                        self.create_title.pop();
                    }
                    1 => {
                        self.create_desc.pop();
                    }
                    2 => {
                        self.create_agent_id.pop();
                    }
                    CREATE_TOGGLE_STEP => {} // toggle, no text
                    4 => {
                        self.create_verify_agent_id.pop();
                    }
                    5 => {
                        self.create_evaluator_model.pop();
                    }
                    CREATE_CADENCE_STEP => {
                        self.create_tick_interval.pop();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        GoalsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let inner = widgets::render_screen_block(f, area, &crate::i18n::t("tui-goals-title"));

    if state.create_open {
        draw_create(f, inner, state);
    } else if state.cadence_open {
        draw_cadence(f, inner, state);
    } else if state.detail_open {
        draw_split(f, inner, state);
    } else {
        draw_list(f, inner, state);
    }
}

fn draw_split(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let chunks = Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(area);

    draw_list_panel(f, chunks[0], state);
    draw_detail(f, chunks[1], state);
}

fn draw_list_panel(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    // Header with count
    if state.search_mode {
        f.render_widget(widgets::search_input(&state.search_buf), chunks[0]);
    } else {
        let search_hint = if state.search_buf.is_empty() {
            String::new()
        } else {
            crate::i18n::t_args("tui-goals-filter", &[("query", &state.search_buf)])
        };
        f.render_widget(
            Paragraph::new(vec![Line::from(vec![
                Span::styled(
                    format!("  {} goals", state.filtered.len()),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
                Span::styled(search_hint, theme::dim_style()),
            ])]),
            chunks[0],
        );
    }

    // List
    if state.loading {
        let loading_text = crate::i18n::t("tui-goals-loading");
        f.render_widget(widgets::spinner(state.tick, &loading_text), chunks[1]);
    } else if state.filtered.is_empty() {
        let empty_text = crate::i18n::t("tui-goals-empty");
        f.render_widget(widgets::empty_state(&empty_text), chunks[1]);
    } else {
        let items: Vec<ListItem> = state
            .filtered
            .iter()
            .map(|&idx| {
                let g = &state.goals[idx];
                let (badge, badge_style) = goal_status_badge(&g.status);
                let title_display = widgets::truncate(&g.title, 22);
                let mut spans = vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(format!("{:<8}", badge), badge_style),
                    Span::styled(" ", Style::default()),
                    Span::styled(
                        format!("{:<22}", title_display),
                        Style::default().fg(theme::TEXT_PRIMARY),
                    ),
                    Span::styled(" ", Style::default()),
                    Span::styled(
                        format!("{:>3}%", g.progress),
                        Style::default().fg(theme::ACCENT_DIM),
                    ),
                ];
                // The status badge to the left is the goal's own lifecycle and
                // reads `ACTV` for a run that is paused or already stopped, so
                // the live run gets its own marker rather than sharing that one.
                if let Some((marker, marker_style)) = run_marker(g) {
                    spans.push(Span::styled("  ", Style::default()));
                    spans.push(Span::styled(marker, marker_style));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    // Hints
    f.render_widget(
        widgets::status_or_hint(&state.status_msg, &crate::i18n::t("tui-goals-hints")),
        chunks[2],
    );
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    draw_list_panel(f, area, state);
}

fn draw_detail(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let idx = match state.selected_goal {
        Some(i) if i < state.goals.len() => i,
        _ => {
            f.render_widget(
                widgets::empty_state(&crate::i18n::t("tui-goals-none-selected")),
                area,
            );
            return;
        }
    };
    let g = &state.goals[idx];

    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // separator
        Constraint::Min(3),    // body
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{2316} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                widgets::truncate(&g.title, 36),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // Body: description, agent, status, progress, run info
    let (badge, badge_style) = goal_status_badge(&g.status);
    let agent = g.agent_id.as_deref().unwrap_or("(none)");
    let loop_eng = if g.loop_engineering { "yes" } else { "no" };
    let verify_agent = g.verify_agent_id.as_deref().unwrap_or("(none)");

    let mut lines = vec![
        Line::from(vec![
            Span::styled("  Status: ", theme::dim_style()),
            Span::styled(badge, badge_style),
        ]),
        Line::from(vec![Span::styled("  Progress: ", theme::dim_style())]),
    ];

    // Progress bar
    let pct = g.progress.min(100);
    let _gauge = Gauge::default()
        .gauge_style(Style::default().fg(theme::ACCENT))
        .percent(pct as u16)
        .label(format!(" {pct}%"));
    // gauge doesn't fit as a Line, render after the text block
    // We'll render it below the Paragraph using a separate area

    lines.push(Line::from(vec![
        Span::styled("  Description: ", theme::dim_style()),
        Span::styled(
            widgets::truncate(&g.description, 40),
            Style::default().fg(theme::TEXT_SECONDARY),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Agent: ", theme::dim_style()),
        Span::styled(agent, Style::default().fg(theme::CYAN)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Auto-review: ", theme::dim_style()),
        Span::styled(loop_eng, Style::default().fg(theme::YELLOW)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Reviewer: ", theme::dim_style()),
        Span::styled(verify_agent, Style::default().fg(theme::TEXT_SECONDARY)),
    ]));
    if let Some(ref em) = g.evaluator_model {
        lines.push(Line::from(vec![
            Span::styled(crate::i18n::t("tui-goals-judge-label"), theme::dim_style()),
            Span::styled(em.as_str(), Style::default().fg(theme::CYAN)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(
            crate::i18n::t("tui-goals-cadence-label"),
            theme::dim_style(),
        ),
        Span::styled(
            cadence_display(g.tick_interval_secs),
            Style::default().fg(theme::TEXT_SECONDARY),
        ),
    ]));

    if let Some(ref phase) = g.run_phase {
        let iter = g.run_iteration.unwrap_or(0);
        let max_iter = g
            .run_max_iterations
            .map(|m| m.to_string())
            .unwrap_or_default();
        let phase_style = if g.is_running() {
            Style::default().fg(theme::GREEN)
        } else if g.is_paused() {
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD)
        } else if phase == "finished" {
            Style::default().fg(theme::ACCENT)
        } else {
            theme::dim_style()
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(crate::i18n::t("tui-goals-phase-label"), theme::dim_style()),
            Span::styled(run_phase_label(phase), phase_style),
        ]));
        if !max_iter.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Iteration: ", theme::dim_style()),
                Span::styled(
                    format!("{iter}/{max_iter}"),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ]));
        }
        // A paused run is a checkpoint, not an ending: say which iteration it
        // would pick up from, and that stopping is the one that throws it away.
        if g.is_paused() {
            let resume_line = if max_iter.is_empty() {
                crate::i18n::t_args(
                    "tui-goals-resume-from-simple",
                    &[("iteration", &iter.to_string())],
                )
            } else {
                crate::i18n::t_args(
                    "tui-goals-resume-from",
                    &[("iteration", &iter.to_string()), ("max", &max_iter)],
                )
            };
            lines.push(Line::from(vec![Span::styled(
                resume_line,
                Style::default().fg(theme::YELLOW),
            )]));
            lines.push(Line::from(vec![
                Span::styled("  \u{24d8} ", Style::default().fg(theme::ACCENT)),
                Span::styled(
                    crate::i18n::t("tui-goals-pause-explainer"),
                    theme::dim_style(),
                ),
            ]));
        }
    }

    let text_area = chunks[2];
    let (text_top, gauge_area) = {
        let ch = Layout::vertical([
            Constraint::Length(lines.len() as u16 + 1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(text_area);
        (ch[0], ch[1])
    };

    f.render_widget(Paragraph::new(lines), text_top);

    // Render progress gauge separately
    let pct = g.progress.min(100);
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(theme::ACCENT))
            .percent(pct as u16)
            .label(format!(" {}% ", pct)),
        gauge_area,
    );

    let run_hint = if g.is_running() {
        crate::i18n::t("tui-goals-hint-running")
    } else if g.is_paused() {
        crate::i18n::t("tui-goals-hint-paused")
    } else {
        crate::i18n::t("tui-goals-hint-start")
    };
    let hint = crate::i18n::t_args("tui-goals-detail-hints", &[("run_hint", &run_hint)]);
    f.render_widget(widgets::hint_bar(&hint), chunks[3]);
}

fn draw_create(f: &mut Frame, area: Rect, state: &GoalsState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // separator
        Constraint::Length(1), // step progress
        Constraint::Length(1), // spacer
        Constraint::Length(1), // field label
        Constraint::Length(1), // spacer
        Constraint::Length(1), // input
        Constraint::Min(0),
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{2316} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-goals-new-title"),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // Step progress indicator
    let progress: Vec<Span> = (0..CREATE_STEPS)
        .map(|i| {
            if i < state.create_step {
                Span::styled("\u{25cf} ", Style::default().fg(theme::GREEN))
            } else if i == state.create_step {
                Span::styled("\u{25cf} ", Style::default().fg(theme::ACCENT))
            } else {
                Span::styled("\u{25cb} ", Style::default().fg(theme::TEXT_TERTIARY))
            }
        })
        .collect();
    let mut step_line = vec![Span::raw("  ")];
    step_line.extend(progress);
    step_line.push(Span::styled(
        crate::i18n::t_args(
            "tui-goals-step",
            &[
                ("n", &(state.create_step + 1).to_string()),
                ("total", &CREATE_STEPS.to_string()),
            ],
        ),
        Style::default().fg(theme::TEXT_SECONDARY),
    ));
    f.render_widget(Paragraph::new(Line::from(step_line)), chunks[2]);

    let judge_label = crate::i18n::t("tui-goals-judge");
    let cadence_label = crate::i18n::t("tui-goals-cadence");
    let (label, value, hint): (&str, &str, String) = match state.create_step {
        0 => (
            "Title:",
            &state.create_title,
            crate::i18n::t("tui-goals-example"),
        ),
        1 => (
            "Description:",
            &state.create_desc,
            crate::i18n::t("tui-goals-prompt"),
        ),
        2 => (
            "Agent:",
            &state.create_agent_id,
            crate::i18n::t("tui-goals-agent-hint"),
        ),
        CREATE_TOGGLE_STEP => ("Auto-review:", "", crate::i18n::t("tui-goals-verify-hint")),
        4 => (
            "Reviewer:",
            &state.create_verify_agent_id,
            crate::i18n::t("tui-goals-reviewer-hint"),
        ),
        5 => (
            &judge_label,
            &state.create_evaluator_model,
            crate::i18n::t("tui-goals-model-hint"),
        ),
        CREATE_CADENCE_STEP => (&cadence_label, &state.create_tick_interval, cadence_hint()),
        _ => ("", &state.create_title, String::new()),
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {label}"),
            Style::default().fg(theme::TEXT_PRIMARY),
        )])),
        chunks[4],
    );

    // Render the user's typed value
    if state.create_step != CREATE_TOGGLE_STEP {
        let display = if value.is_empty() {
            Span::styled("  \u{258c}", Style::default().fg(theme::TEXT_TERTIARY))
        } else {
            Span::styled(
                format!("  {value}"),
                Style::default().fg(theme::TEXT_PRIMARY),
            )
        };
        f.render_widget(Paragraph::new(Line::from(vec![display])), chunks[5]);
    } else {
        // The toggle step has no text buffer — show Auto-review status
        let toggle_text = if state.create_loop_engineering {
            crate::i18n::t("tui-goals-review-on")
        } else {
            crate::i18n::t("tui-goals-review-off")
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {toggle_text}"),
                Style::default().fg(theme::TEXT_PRIMARY),
            )])),
            chunks[5],
        );
    }

    // Info hint with ⓘ icon
    if !hint.is_empty() {
        let hint_style = Style::default().fg(theme::TEXT_SECONDARY);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{24d8} ", Style::default().fg(theme::ACCENT)),
                Span::styled(hint, hint_style),
            ])),
            chunks[6],
        );
    }

    // Validation failure for the field in view, under its hint.
    if !state.create_error.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.create_error),
                Style::default().fg(theme::RED),
            )])),
            chunks[7],
        );
    }

    let hint_text = if state.create_step >= CREATE_LAST_STEP {
        crate::i18n::t("tui-goals-nav-submit")
    } else if state.create_step == CREATE_TOGGLE_STEP {
        crate::i18n::t("tui-goals-nav-toggle")
    } else {
        crate::i18n::t("tui-goals-nav-next")
    };
    f.render_widget(widgets::hint_bar(&hint_text), chunks[8]);
}

/// The cadence editor for an existing goal.
///
/// Deliberately one field rather than a full edit form: cadence is the goal
/// setting that has to change while a run is in flight, and the wizard already
/// owns the create-time path.
fn draw_cadence(f: &mut Frame, area: Rect, state: &GoalsState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // separator
        Constraint::Length(1), // goal being edited
        Constraint::Length(1), // spacer
        Constraint::Length(1), // field label
        Constraint::Length(1), // input
        Constraint::Length(1), // hint
        Constraint::Min(0),    // error
        Constraint::Length(1), // nav hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{2316} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-goals-cadence-title"),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );
    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    if let Some(g) = state.selected_goal.and_then(|i| state.goals.get(i)) {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    widgets::truncate(&g.title, 40),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ])),
            chunks[2],
        );
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {}", crate::i18n::t("tui-goals-cadence")),
            Style::default().fg(theme::TEXT_PRIMARY),
        )])),
        chunks[4],
    );

    let value = if state.cadence_buf.is_empty() {
        Span::styled("  \u{258c}", Style::default().fg(theme::TEXT_TERTIARY))
    } else {
        Span::styled(
            format!("  {}", state.cadence_buf),
            Style::default().fg(theme::TEXT_PRIMARY),
        )
    };
    f.render_widget(Paragraph::new(Line::from(vec![value])), chunks[5]);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{24d8} ", Style::default().fg(theme::ACCENT)),
            Span::styled(cadence_hint(), Style::default().fg(theme::TEXT_SECONDARY)),
        ])),
        chunks[6],
    );

    if !state.cadence_error.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.cadence_error),
                Style::default().fg(theme::RED),
            )])),
            chunks[7],
        );
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-goals-cadence-nav")),
        chunks[8],
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Parse the cadence field shared by the create wizard and the cadence editor.
///
/// Blank is legal and means "no override": the goal then ticks at the daemon's
/// own default, which is what every goal did before the cadence was
/// configurable. Anything else must be a whole number of seconds inside the
/// range the API enforces, checked here so a typo is reported on the field
/// rather than as a rejected request one round-trip later.
pub fn parse_cadence_input(raw: &str) -> Result<Option<u32>, String> {
    use librefang_types::goal::{MAX_GOAL_TICK_INTERVAL_SECS, MIN_GOAL_TICK_INTERVAL_SECS};
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    // A run of digits too long for u64 is still a number, just an absurd one:
    // reporting it as "not a number" would send the operator looking for a
    // typo in digits that are all perfectly valid. Saturating lets the range
    // check below give it the answer it deserves.
    let secs = match trimmed.parse::<u64>() {
        Ok(v) => v,
        Err(_) if trimmed.chars().all(|c| c.is_ascii_digit()) => u64::MAX,
        Err(_) => return Err(crate::i18n::t("tui-goals-cadence-not-a-number")),
    };
    let range = u64::from(MIN_GOAL_TICK_INTERVAL_SECS)..=u64::from(MAX_GOAL_TICK_INTERVAL_SECS);
    if !range.contains(&secs) {
        return Err(crate::i18n::t_args(
            "tui-goals-cadence-out-of-range",
            &[
                ("min", &MIN_GOAL_TICK_INTERVAL_SECS.to_string()),
                ("max", &MAX_GOAL_TICK_INTERVAL_SECS.to_string()),
            ],
        ));
    }
    Ok(Some(secs as u32))
}

/// Plain-language explanation of what the cadence field controls.
fn cadence_hint() -> String {
    use librefang_types::goal::{DEFAULT_GOAL_TICK_INTERVAL_SECS, MAX_GOAL_TICK_INTERVAL_SECS};
    crate::i18n::t_args(
        "tui-goals-cadence-hint",
        &[
            ("default", &DEFAULT_GOAL_TICK_INTERVAL_SECS.to_string()),
            ("max", &MAX_GOAL_TICK_INTERVAL_SECS.to_string()),
        ],
    )
}

/// Render a goal's cadence, naming the default when it has no override.
pub fn cadence_display(tick_interval_secs: Option<u32>) -> String {
    use librefang_types::goal::DEFAULT_GOAL_TICK_INTERVAL_SECS;
    match tick_interval_secs {
        Some(secs) => crate::i18n::t_args("tui-goals-cadence-secs", &[("secs", &secs.to_string())]),
        None => crate::i18n::t_args(
            "tui-goals-cadence-default",
            &[("secs", &DEFAULT_GOAL_TICK_INTERVAL_SECS.to_string())],
        ),
    }
}

/// Human-readable name for a run phase the daemon reports as a bare token.
///
/// An unknown phase falls through verbatim rather than being hidden: a daemon
/// newer than this binary should still show what it is doing.
pub fn run_phase_label(phase: &str) -> String {
    match phase {
        "running" => crate::i18n::t("tui-goals-run-running"),
        "paused" => crate::i18n::t("tui-goals-run-paused"),
        "finished" => crate::i18n::t("tui-goals-run-finished"),
        "max_iterations_reached" => crate::i18n::t("tui-goals-run-maxiter"),
        "rate_limited" => crate::i18n::t("tui-goals-run-ratelimited"),
        "stopped" => crate::i18n::t("tui-goals-run-stopped"),
        other => other.to_string(),
    }
}

/// The list-row marker for a goal with a live run, or `None` for one whose run
/// has settled (or never started) — those are already described by the status
/// badge and would only add noise.
fn run_marker(g: &GoalInfo) -> Option<(String, Style)> {
    /// Filled circle: the loop is turning.
    const GLYPH_RUNNING: &str = "\u{25cf}";
    /// Double bar, the universal pause glyph: parked, not finished.
    const GLYPH_PAUSED: &str = "\u{2016}";
    if g.is_running() {
        Some((
            format!(
                "{} {}",
                GLYPH_RUNNING,
                crate::i18n::t("tui-goals-run-running")
            ),
            Style::default().fg(theme::GREEN),
        ))
    } else if g.is_paused() {
        Some((
            format!(
                "{} {}",
                GLYPH_PAUSED,
                crate::i18n::t("tui-goals-run-paused")
            ),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        None
    }
}

/// Map a goal status string to a (badge_text, style) pair.
fn goal_status_badge(status: &str) -> (String, Style) {
    let lower = status.to_lowercase();
    if lower.contains("in_progress") || lower.contains("running") || lower.contains("active") {
        (
            crate::i18n::t("tui-goals-phase-actv"),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )
    } else if lower.contains("completed") || lower.contains("done") {
        (
            crate::i18n::t("tui-goals-phase-done"),
            Style::default().fg(theme::ACCENT_DIM),
        )
    } else if lower.contains("cancelled") || lower.contains("cancel") {
        (
            crate::i18n::t("tui-goals-phase-canc"),
            Style::default().fg(theme::TEXT_TERTIARY),
        )
    } else if lower.contains("failed") || lower.contains("error") {
        (
            crate::i18n::t("tui-goals-phase-fail"),
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        )
    } else {
        // pending / default
        (
            crate::i18n::t("tui-goals-phase-pend"),
            Style::default().fg(theme::YELLOW),
        )
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::goal::{
        DEFAULT_GOAL_TICK_INTERVAL_SECS, MAX_GOAL_TICK_INTERVAL_SECS, MIN_GOAL_TICK_INTERVAL_SECS,
    };

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    /// A goal in the given run phase, selected in the list.
    fn state_with(phase: Option<&str>, tick_interval_secs: Option<u32>) -> GoalsState {
        let mut s = GoalsState::new();
        s.goals = vec![GoalInfo {
            id: "goal-1".into(),
            title: "Ship it".into(),
            // A started run flips the stored status to `in_progress` and never
            // resets it, so every fixture here carries the status a paused and
            // a stopped goal are indistinguishable by.
            status: "in_progress".into(),
            tick_interval_secs,
            run_phase: phase.map(str::to_string),
            run_iteration: Some(7),
            run_max_iterations: Some(25),
            ..Default::default()
        }];
        s.refilter();
        s
    }

    fn goal_id_of(action: &GoalsAction) -> Option<&str> {
        match action {
            GoalsAction::StartRun { goal_id }
            | GoalsAction::PauseRun { goal_id }
            | GoalsAction::ResumeRun { goal_id }
            | GoalsAction::StopRun { goal_id }
            | GoalsAction::SetCadence { goal_id, .. } => Some(goal_id),
            _ => None,
        }
    }

    /// Pause and stop are different outcomes and must not share a key: one
    /// keeps the checkpoint, the other discards it.
    #[test]
    fn pause_and_stop_are_distinct_keys_on_a_running_goal() {
        let mut s = state_with(Some("running"), None);
        assert!(
            matches!(s.handle_key(key('p')), GoalsAction::PauseRun { .. }),
            "p must pause a running goal"
        );
        assert!(
            matches!(s.handle_key(key('x')), GoalsAction::StopRun { .. }),
            "x must stop a running goal"
        );
    }

    /// `s` on a running goal is not a hidden stop — it reports why it did
    /// nothing instead of silently discarding the run's checkpoint.
    #[test]
    fn start_key_on_a_running_goal_does_not_stop_it() {
        let mut s = state_with(Some("running"), None);
        assert!(matches!(s.handle_key(key('s')), GoalsAction::Continue));
        assert_eq!(s.status_msg, crate::i18n::t("tui-goals-already-running"));
        assert!(!s.status_msg.starts_with('['), "message key is missing");
    }

    /// A paused goal resumes from its checkpoint rather than starting over.
    #[test]
    fn start_key_resumes_a_paused_goal() {
        let mut s = state_with(Some("paused"), None);
        let action = s.handle_key(key('s'));
        assert!(
            matches!(action, GoalsAction::ResumeRun { .. }),
            "s on a paused goal must resume, not start from iteration 0"
        );
        assert_eq!(goal_id_of(&action), Some("goal-1"));
    }

    /// Stop stays available while paused: that is how an operator discards a
    /// checkpoint they have decided not to continue.
    #[test]
    fn stop_key_acts_on_a_paused_goal() {
        let mut s = state_with(Some("paused"), None);
        assert!(matches!(
            s.handle_key(key('x')),
            GoalsAction::StopRun { .. }
        ));
    }

    /// The pause state is read from the live run, not from the goal's own
    /// status: `in_progress` is set at start and never cleared, so it says
    /// "running" for a goal that is paused, stopped or finished.
    #[test]
    fn run_controls_read_the_run_phase_not_the_goal_status() {
        // Same `status: "in_progress"` in all three fixtures.
        let mut idle = state_with(None, None);
        assert!(
            matches!(idle.handle_key(key('s')), GoalsAction::StartRun { .. }),
            "a goal with no live run must start, despite status in_progress"
        );

        let paused = state_with(Some("paused"), None);
        assert!(paused.goals[0].is_paused());
        assert!(!paused.goals[0].is_running());

        let stopped = state_with(Some("stopped"), None);
        assert!(!stopped.goals[0].has_live_run());
    }

    /// A settled run has nothing to stop, and says so rather than firing a
    /// request that would report "stopped: false".
    #[test]
    fn stop_key_on_a_settled_run_reports_instead_of_firing() {
        let mut s = state_with(Some("finished"), None);
        assert!(matches!(s.handle_key(key('x')), GoalsAction::Continue));
        assert_eq!(s.status_msg, crate::i18n::t("tui-goals-nothing-to-stop"));
        assert!(!s.status_msg.starts_with('['), "message key is missing");
    }

    /// Pausing something that is not running explains itself too.
    #[test]
    fn pause_key_on_an_idle_goal_reports_instead_of_firing() {
        let mut s = state_with(None, None);
        assert!(matches!(s.handle_key(key('p')), GoalsAction::Continue));
        assert_eq!(s.status_msg, crate::i18n::t("tui-goals-not-running"));

        let mut already = state_with(Some("paused"), None);
        assert!(matches!(
            already.handle_key(key('p')),
            GoalsAction::Continue
        ));
        assert_eq!(
            already.status_msg,
            crate::i18n::t("tui-goals-already-paused")
        );
    }

    /// Only a live run gets a list marker; the status badge already covers the
    /// rest, and a second badge saying the same thing is noise.
    #[test]
    fn only_a_live_run_gets_a_list_marker() {
        assert!(run_marker(&state_with(Some("running"), None).goals[0]).is_some());
        assert!(run_marker(&state_with(Some("paused"), None).goals[0]).is_some());
        assert!(run_marker(&state_with(Some("stopped"), None).goals[0]).is_none());
        assert!(run_marker(&state_with(None, None).goals[0]).is_none());

        let running = run_marker(&state_with(Some("running"), None).goals[0]).unwrap();
        let paused = run_marker(&state_with(Some("paused"), None).goals[0]).unwrap();
        assert_ne!(
            running.0, paused.0,
            "paused must not render the same marker as running"
        );
    }

    /// Every phase the daemon can report renders as prose, and an unknown one
    /// falls through verbatim rather than vanishing.
    #[test]
    fn run_phase_labels_are_translated_and_unknown_phases_pass_through() {
        for phase in [
            "running",
            "paused",
            "finished",
            "max_iterations_reached",
            "rate_limited",
            "stopped",
        ] {
            let label = run_phase_label(phase);
            assert!(
                !label.starts_with('['),
                "phase {phase} has no locale key: {label}"
            );
            assert_ne!(label, phase, "phase {phase} was not humanised");
        }
        assert_eq!(run_phase_label("some_future_phase"), "some_future_phase");
    }

    #[test]
    fn parse_cadence_input_accepts_blank_as_no_override() {
        assert_eq!(parse_cadence_input(""), Ok(None));
        assert_eq!(parse_cadence_input("   "), Ok(None));
    }

    #[test]
    fn parse_cadence_input_accepts_the_documented_range() {
        assert_eq!(parse_cadence_input(" 30 "), Ok(Some(30)));
        assert_eq!(
            parse_cadence_input(&MIN_GOAL_TICK_INTERVAL_SECS.to_string()),
            Ok(Some(MIN_GOAL_TICK_INTERVAL_SECS))
        );
        assert_eq!(
            parse_cadence_input(&MAX_GOAL_TICK_INTERVAL_SECS.to_string()),
            Ok(Some(MAX_GOAL_TICK_INTERVAL_SECS))
        );
    }

    /// Out-of-range values are refused with the range spelled out, and an
    /// overflowing number is out of range rather than "not a number".
    #[test]
    fn parse_cadence_input_rejects_values_outside_the_range() {
        let over_max = (MAX_GOAL_TICK_INTERVAL_SECS + 1).to_string();
        for bad in ["0", over_max.as_str(), "99999999999999999999"] {
            let err = parse_cadence_input(bad).expect_err("out-of-range cadence must be refused");
            assert!(!err.starts_with('['), "message key is missing: {err}");
            assert!(
                err.contains(&MAX_GOAL_TICK_INTERVAL_SECS.to_string()),
                "{bad}: the message must name the range: {err}"
            );
        }
    }

    #[test]
    fn parse_cadence_input_rejects_text() {
        for bad in ["abc", "3.5", "-1", "30s"] {
            let err = parse_cadence_input(bad).expect_err("a non-number must be refused");
            assert!(!err.starts_with('['), "message key is missing: {err}");
        }
    }

    /// The editor opens pre-filled with the goal's own cadence, so the operator
    /// sees what they are changing.
    #[test]
    fn cadence_editor_prefills_from_the_selected_goal() {
        let mut s = state_with(Some("running"), Some(45));
        assert!(matches!(s.handle_key(key('c')), GoalsAction::Continue));
        assert!(s.cadence_open);
        assert_eq!(s.cadence_buf, "45");
    }

    /// A goal with no override opens blank, and saving it blank clears the
    /// override rather than persisting a number the operator never chose.
    #[test]
    fn cadence_editor_saves_a_blank_field_as_a_cleared_override() {
        let mut s = state_with(Some("running"), None);
        s.handle_key(key('c'));
        assert_eq!(s.cadence_buf, "");
        let action = s.handle_key(enter());
        match action {
            GoalsAction::SetCadence {
                goal_id,
                tick_interval_secs,
            } => {
                assert_eq!(goal_id, "goal-1");
                assert_eq!(tick_interval_secs, None, "blank must clear the override");
            }
            _ => panic!("Enter must save the cadence"),
        }
        assert!(!s.cadence_open, "saving closes the editor");
    }

    #[test]
    fn cadence_editor_saves_a_typed_value() {
        let mut s = state_with(None, None);
        s.handle_key(key('c'));
        for c in "90".chars() {
            s.handle_key(key(c));
        }
        assert!(matches!(
            s.handle_key(enter()),
            GoalsAction::SetCadence {
                tick_interval_secs: Some(90),
                ..
            }
        ));
    }

    /// An out-of-range cadence keeps the editor open with the reason, instead
    /// of firing a request the API would refuse.
    #[test]
    fn cadence_editor_refuses_an_out_of_range_value() {
        let mut s = state_with(None, None);
        s.handle_key(key('c'));
        s.handle_key(key('0'));
        assert!(matches!(s.handle_key(enter()), GoalsAction::Continue));
        assert!(s.cadence_open, "the editor stays open on a bad value");
        assert!(!s.cadence_error.is_empty());
        // Editing clears the complaint so it cannot outlive what it described.
        s.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(s.cadence_error.is_empty());
    }

    #[test]
    fn cadence_editor_escape_discards_the_edit() {
        let mut s = state_with(None, Some(45));
        s.handle_key(key('c'));
        s.handle_key(key('9'));
        assert!(matches!(
            s.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            GoalsAction::Continue
        ));
        assert!(!s.cadence_open);
    }

    /// Walk the wizard to the end and submit: the cadence typed on the last
    /// step reaches the create request.
    #[test]
    fn create_wizard_carries_the_cadence_to_the_request() {
        let mut s = GoalsState::new();
        s.handle_key(key('n'));
        for c in "Ship".chars() {
            s.handle_key(key(c));
        }
        // One Enter per remaining field, then the digits of the cadence.
        for _ in 0..CREATE_CADENCE_STEP {
            s.handle_key(enter());
        }
        assert_eq!(s.create_step, CREATE_CADENCE_STEP);
        for c in "600".chars() {
            s.handle_key(key(c));
        }
        match s.handle_key(enter()) {
            GoalsAction::CreateGoal {
                title,
                tick_interval_secs,
                ..
            } => {
                assert_eq!(title, "Ship");
                assert_eq!(tick_interval_secs, Some(600));
            }
            _ => panic!("the last step must submit"),
        }
        assert!(!s.create_open);
    }

    /// Leaving the cadence blank is valid and submits without an override.
    #[test]
    fn create_wizard_submits_without_a_cadence() {
        let mut s = GoalsState::new();
        s.handle_key(key('n'));
        s.handle_key(key('X'));
        for _ in 0..CREATE_CADENCE_STEP {
            s.handle_key(enter());
        }
        assert!(matches!(
            s.handle_key(enter()),
            GoalsAction::CreateGoal {
                tick_interval_secs: None,
                ..
            }
        ));
    }

    /// A bad cadence blocks the submit and puts the wizard back on the field
    /// that is wrong.
    #[test]
    fn create_wizard_refuses_an_out_of_range_cadence() {
        let mut s = GoalsState::new();
        s.handle_key(key('n'));
        s.handle_key(key('X'));
        for _ in 0..CREATE_CADENCE_STEP {
            s.handle_key(enter());
        }
        s.handle_key(key('0'));
        assert!(matches!(s.handle_key(enter()), GoalsAction::Continue));
        assert!(s.create_open, "the wizard stays open");
        assert_eq!(s.create_step, CREATE_CADENCE_STEP);
        assert!(!s.create_error.is_empty());
    }

    /// The step counter never claims a step past the last one: the wizard used
    /// to walk to a blank seventh screen that rendered "step 7/6".
    #[test]
    fn create_wizard_never_advances_past_the_last_step() {
        let mut s = GoalsState::new();
        s.handle_key(key('n'));
        s.handle_key(key('X'));
        for _ in 0..20 {
            if !s.create_open {
                break;
            }
            assert!(
                s.create_step < CREATE_STEPS,
                "step {} is past the {CREATE_STEPS}-step wizard",
                s.create_step
            );
            s.handle_key(enter());
        }
        assert!(!s.create_open, "the wizard must submit and close");
    }

    /// The cadence hint has to say what the field does in plain language, and
    /// name the default that a blank field falls back to.
    #[test]
    fn cadence_hint_names_the_default() {
        let hint = cadence_hint();
        assert!(!hint.starts_with('['), "hint key is missing: {hint}");
        assert!(
            hint.contains(&DEFAULT_GOAL_TICK_INTERVAL_SECS.to_string()),
            "the hint must name the default cadence: {hint}"
        );
    }

    /// A goal with no override reads as the default rather than as blank.
    #[test]
    fn cadence_display_names_the_default_when_unset() {
        let unset = cadence_display(None);
        assert!(!unset.starts_with('['), "key is missing: {unset}");
        assert!(unset.contains(&DEFAULT_GOAL_TICK_INTERVAL_SECS.to_string()));
        assert!(cadence_display(Some(30)).contains("30"));
    }

    /// Every run-control key works the same from the detail pane as from the
    /// list: they share one resolver, and this pins that they stay in step.
    #[test]
    fn detail_pane_run_controls_match_the_list() {
        let mut s = state_with(Some("running"), None);
        s.selected_goal = Some(0);
        s.detail_open = true;
        assert!(matches!(
            s.handle_key(key('p')),
            GoalsAction::PauseRun { .. }
        ));
        assert!(matches!(
            s.handle_key(key('x')),
            GoalsAction::StopRun { .. }
        ));

        let mut paused = state_with(Some("paused"), None);
        paused.selected_goal = Some(0);
        paused.detail_open = true;
        assert!(matches!(
            paused.handle_key(key('s')),
            GoalsAction::ResumeRun { .. }
        ));
        assert!(matches!(paused.handle_key(key('c')), GoalsAction::Continue));
        assert!(
            paused.cadence_open,
            "c opens the editor from the detail too"
        );
    }
}
