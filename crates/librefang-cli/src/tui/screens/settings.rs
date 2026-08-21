//! Settings screen: provider key management, model catalog, tools list.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct ProviderInfo {
    pub name: String,
    pub configured: bool,
    pub env_var: String,
    /// Whether this is a local provider (ollama, vllm, lmstudio).
    pub is_local: bool,
    /// Whether the local provider is reachable (only set for local providers).
    pub reachable: Option<bool>,
    /// Probe latency in milliseconds (only set for local providers).
    pub latency_ms: Option<u64>,
}

#[derive(Clone, Default)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub tier: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub cost_input: f64,
    pub cost_output: f64,
    /// True when `context_window` resolved to neither an override nor a
    /// known catalog value — the runtime falls back to a conservative 8192
    /// tokens in this case (#7774). Only ever set for text models.
    pub context_window_is_estimated: bool,
}

#[derive(Clone, Default)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Clone)]
pub struct TestResult {
    pub provider: String,
    pub success: bool,
    pub latency_ms: u64,
    pub message: String,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsSub {
    Providers,
    Models,
    Tools,
}

pub struct SettingsState {
    pub sub: SettingsSub,
    pub providers: Vec<ProviderInfo>,
    pub models: Vec<ModelInfo>,
    pub tools: Vec<ToolInfo>,
    pub provider_list: ListState,
    pub model_list: ListState,
    pub tool_list: ListState,
    pub input_buf: String,
    pub input_mode: bool,
    pub editing_provider: Option<String>,
    pub test_result: Option<TestResult>,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
    // ── Model override editor (#7774) ──────────────────────────────
    /// True while the context_window / max_output_tokens editor is open.
    pub model_edit_mode: bool,
    /// True while waiting for the `GET /api/models/overrides/{id}`
    /// round-trip that precedes opening the editor — the whole existing
    /// `ModelOverrides` entity has to be fetched first so saving doesn't
    /// clobber fields set from another surface (the API replaces the whole
    /// entity, it does not patch a single field).
    pub model_edit_loading: bool,
    /// Override key (`provider:model_id`) of the model being edited.
    pub model_edit_key: String,
    /// 0 = context_window field focused, 1 = max_output_tokens field focused.
    pub model_edit_field: usize,
    pub model_edit_ctx: String,
    pub model_edit_max_out: String,
    /// The overrides entity as fetched, so saving preserves every field the
    /// editor doesn't touch (temperature, capability overrides, …).
    pub model_edit_base: librefang_types::model_catalog::ModelOverrides,
}

pub enum SettingsAction {
    Continue,
    RefreshProviders,
    RefreshModels,
    RefreshTools,
    SaveProviderKey {
        name: String,
        key: String,
    },
    DeleteProviderKey(String),
    TestProvider(String),
    /// Fetch the current overrides for a model before opening the editor
    /// (#7774) — `model_key` is `provider:model_id`.
    FetchModelOverrides(String),
    /// Persist the merged overrides entity for a model.
    SaveModelOverrides {
        model_key: String,
        overrides: librefang_types::model_catalog::ModelOverrides,
    },
    /// Clear every override for a model (mirrors the dashboard's "Reset").
    ResetModelOverrides(String),
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            sub: SettingsSub::Providers,
            providers: Vec::new(),
            models: Vec::new(),
            tools: Vec::new(),
            provider_list: ListState::default(),
            model_list: ListState::default(),
            tool_list: ListState::default(),
            input_buf: String::new(),
            input_mode: false,
            editing_provider: None,
            test_result: None,
            loading: false,
            tick: 0,
            status_msg: String::new(),
            model_edit_mode: false,
            model_edit_loading: false,
            model_edit_key: String::new(),
            model_edit_field: 0,
            model_edit_ctx: String::new(),
            model_edit_max_out: String::new(),
            model_edit_base: librefang_types::model_catalog::ModelOverrides::default(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SettingsAction::Continue;
        }

        if self.model_edit_mode {
            return self.handle_model_edit(key);
        }

        if self.input_mode {
            return self.handle_input(key);
        }

        // Sub-tab switching
        if !self.input_mode {
            match key.code {
                KeyCode::Char('1') => {
                    self.sub = SettingsSub::Providers;
                    return SettingsAction::RefreshProviders;
                }
                KeyCode::Char('2') => {
                    self.sub = SettingsSub::Models;
                    return SettingsAction::RefreshModels;
                }
                KeyCode::Char('3') => {
                    self.sub = SettingsSub::Tools;
                    return SettingsAction::RefreshTools;
                }
                _ => {}
            }
        }

        match self.sub {
            SettingsSub::Providers => self.handle_providers(key),
            SettingsSub::Models => self.handle_models(key),
            SettingsSub::Tools => self.handle_tools(key),
        }
    }

    fn handle_input(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = false;
                self.editing_provider = None;
                self.input_buf.clear();
            }
            KeyCode::Enter => {
                self.input_mode = false;
                if let Some(name) = self.editing_provider.take() {
                    if !self.input_buf.is_empty() {
                        let api_key = self.input_buf.clone();
                        self.input_buf.clear();
                        return SettingsAction::SaveProviderKey { name, key: api_key };
                    }
                }
                self.input_buf.clear();
            }
            KeyCode::Backspace => {
                self.input_buf.pop();
            }
            KeyCode::Char(c) => {
                self.input_buf.push(c);
            }
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_providers(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.providers.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.provider_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.provider_list.select(Some(next));
                self.test_result = None;
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.provider_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.provider_list.select(Some(next));
                self.test_result = None;
            }
            KeyCode::Char('e') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        self.editing_provider = Some(self.providers[sel].name.clone());
                        self.input_mode = true;
                        self.input_buf.clear();
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        return SettingsAction::DeleteProviderKey(self.providers[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('t') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        self.test_result = None;
                        return SettingsAction::TestProvider(self.providers[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('r') => return SettingsAction::RefreshProviders,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_models(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.models.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.model_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.model_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.model_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.model_list.select(Some(next));
            }
            KeyCode::Char('e') => {
                if let Some(sel) = self.model_list.selected() {
                    if let Some(m) = self.models.get(sel) {
                        self.model_edit_loading = true;
                        return SettingsAction::FetchModelOverrides(format!(
                            "{}:{}",
                            m.provider, m.id
                        ));
                    }
                }
            }
            KeyCode::Char('x') => {
                if let Some(sel) = self.model_list.selected() {
                    if let Some(m) = self.models.get(sel) {
                        return SettingsAction::ResetModelOverrides(format!(
                            "{}:{}",
                            m.provider, m.id
                        ));
                    }
                }
            }
            KeyCode::Char('r') => return SettingsAction::RefreshModels,
            _ => {}
        }
        SettingsAction::Continue
    }

    /// Handles input while the context_window / max_output_tokens editor
    /// (#7774) is open. Tab switches the focused field; digits type into it;
    /// Enter merges the two fields into `model_edit_base` (preserving every
    /// other override untouched) and submits; Esc cancels without saving.
    fn handle_model_edit(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.model_edit_mode = false;
                self.model_edit_ctx.clear();
                self.model_edit_max_out.clear();
            }
            KeyCode::Tab | KeyCode::Up | KeyCode::Down | KeyCode::BackTab => {
                self.model_edit_field = 1 - self.model_edit_field;
            }
            KeyCode::Enter => {
                self.model_edit_mode = false;
                let mut overrides = self.model_edit_base.clone();
                overrides.context_window =
                    self.model_edit_ctx.parse::<u64>().ok().filter(|v| *v > 0);
                overrides.max_output_tokens = self
                    .model_edit_max_out
                    .parse::<u64>()
                    .ok()
                    .filter(|v| *v > 0);
                let model_key = std::mem::take(&mut self.model_edit_key);
                self.model_edit_ctx.clear();
                self.model_edit_max_out.clear();
                return SettingsAction::SaveModelOverrides {
                    model_key,
                    overrides,
                };
            }
            KeyCode::Backspace => {
                let buf = self.model_edit_active_buf();
                buf.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let buf = self.model_edit_active_buf();
                buf.push(c);
            }
            _ => {}
        }
        SettingsAction::Continue
    }

    fn model_edit_active_buf(&mut self) -> &mut String {
        if self.model_edit_field == 0 {
            &mut self.model_edit_ctx
        } else {
            &mut self.model_edit_max_out
        }
    }

    fn handle_tools(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.tools.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.tool_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.tool_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.tool_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.tool_list.select(Some(next));
            }
            KeyCode::Char('r') => return SettingsAction::RefreshTools,
            _ => {}
        }
        SettingsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("⚙ {}", crate::i18n::t("tui-settings-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(1), // sub-tab bar
        Constraint::Length(1), // separator
        Constraint::Min(3),    // content
        Constraint::Length(1), // hints
    ])
    .split(inner);

    draw_sub_tabs(f, chunks[0], state.sub);

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    match state.sub {
        SettingsSub::Providers => draw_providers(f, chunks[2], state),
        SettingsSub::Models => draw_models(f, chunks[2], state),
        SettingsSub::Tools => draw_tools(f, chunks[2], state),
    }

    // Hints
    let hint_text = match state.sub {
        SettingsSub::Providers if state.input_mode => crate::i18n::t("tui-settings-hints-input"),
        SettingsSub::Providers => crate::i18n::t("tui-settings-hints-providers"),
        SettingsSub::Models if state.model_edit_mode => {
            crate::i18n::t("tui-settings-hints-models-edit")
        }
        SettingsSub::Models => crate::i18n::t("tui-settings-hints-models"),
        SettingsSub::Tools => crate::i18n::t("tui-settings-hints-tools"),
    };
    f.render_widget(widgets::hint_bar(&hint_text), chunks[3]);
}

fn draw_sub_tabs(f: &mut Frame, area: Rect, active: SettingsSub) {
    let tabs = [
        (
            SettingsSub::Providers,
            crate::i18n::t("tui-settings-tab-providers"),
        ),
        (
            SettingsSub::Models,
            crate::i18n::t("tui-settings-tab-models"),
        ),
        (SettingsSub::Tools, crate::i18n::t("tui-settings-tab-tools")),
    ];
    let mut spans = vec![Span::raw("  ")];
    for (i, (sub, label)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(theme::BORDER)));
        }
        if *sub == active {
            spans.push(Span::styled(format!(" ● {label} "), theme::tab_active()));
        } else {
            spans.push(Span::styled(format!(" ○ {label} "), theme::tab_inactive()));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_providers(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(2), // input / test result
    ])
    .split(area);

    let provider_hdr = crate::i18n::t("tui-settings-providers-header-provider");
    let status_hdr = crate::i18n::t("tui-settings-providers-header-status");
    let env_hdr = crate::i18n::t("tui-settings-providers-header-env");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<20} {:<20} {}", provider_hdr, status_hdr, env_hdr),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.providers.is_empty() {
        f.render_widget(
            widgets::spinner(
                state.tick,
                &crate::i18n::t("tui-settings-providers-loading"),
            ),
            chunks[1],
        );
    } else if state.providers.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-providers-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .providers
            .iter()
            .map(|p| {
                let (badge, badge_style) = if p.is_local {
                    match p.reachable {
                        Some(true) => {
                            let ms = p.latency_ms.unwrap_or(0);
                            (
                                format!(
                                    "● {}",
                                    crate::i18n::t_args(
                                        "tui-settings-providers-status-online",
                                        &[("ms", &ms.to_string())]
                                    )
                                ),
                                Style::default().fg(theme::GREEN),
                            )
                        }
                        Some(false) => (
                            format!(
                                "● {}",
                                crate::i18n::t("tui-settings-providers-status-offline")
                            ),
                            Style::default().fg(theme::RED),
                        ),
                        None => (
                            format!(
                                "○ {}",
                                crate::i18n::t("tui-settings-providers-status-local")
                            ),
                            theme::dim_style(),
                        ),
                    }
                } else if p.configured {
                    (
                        format!(
                            "● {}",
                            crate::i18n::t("tui-settings-providers-status-configured")
                        ),
                        Style::default().fg(theme::GREEN),
                    )
                } else {
                    (
                        format!(
                            "○ {}",
                            crate::i18n::t("tui-settings-providers-status-notset")
                        ),
                        theme::dim_style(),
                    )
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<20}", p.name),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(format!(" {:<20}", badge), badge_style),
                    Span::styled(format!(" {}", p.env_var), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.provider_list);
    }

    // Input mode or test result
    if state.input_mode {
        let provider_name = state.editing_provider.as_deref().unwrap_or("?");
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    format!(
                        "  🔑 {}",
                        crate::i18n::t_args(
                            "tui-settings-providers-input-prompt",
                            &[("provider", provider_name)]
                        )
                    ),
                    Style::default().fg(theme::YELLOW),
                )]),
                Line::from(vec![
                    Span::raw("  ▸ "),
                    Span::styled(
                        "•".repeat(state.input_buf.len().min(40)),
                        theme::input_style(),
                    ),
                    Span::styled(
                        "█",
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::SLOW_BLINK),
                    ),
                ]),
            ]),
            chunks[2],
        );
    } else if let Some(result) = &state.test_result {
        let (icon, style) = if result.success {
            ("●", Style::default().fg(theme::GREEN))
        } else {
            ("●", Style::default().fg(theme::RED))
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(format!("  {icon} "), style),
                    Span::styled(format!("{}: {}", result.provider, result.message), style),
                ]),
                Line::from(vec![Span::styled(
                    if result.success {
                        format!(
                            "  {}",
                            crate::i18n::t_args(
                                "tui-settings-providers-latency",
                                &[("ms", &result.latency_ms.to_string())]
                            )
                        )
                    } else {
                        String::new()
                    },
                    theme::dim_style(),
                )]),
            ]),
            chunks[2],
        );
    } else if !state.status_msg.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::GREEN),
            )])),
            chunks[2],
        );
    }
}

fn draw_models(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    if state.model_edit_mode {
        draw_model_edit(f, area, state);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
    ])
    .split(area);

    let id_hdr = crate::i18n::t("tui-settings-models-header-id");
    let provider_hdr = crate::i18n::t("tui-settings-models-header-provider");
    let tier_hdr = crate::i18n::t("tui-settings-models-header-tier");
    let ctx_hdr = crate::i18n::t("tui-settings-models-header-context");
    let cost_hdr = crate::i18n::t("tui-settings-models-header-cost");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<28} {:<14} {:<10} {:<10} {}",
                id_hdr, provider_hdr, tier_hdr, ctx_hdr, cost_hdr
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.models.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-settings-models-loading")),
            chunks[1],
        );
    } else if state.models.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-models-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .models
            .iter()
            .map(|m| {
                let tier_style = match m.tier.as_str() {
                    "Frontier" => Style::default()
                        .fg(theme::PURPLE)
                        .add_modifier(Modifier::BOLD),
                    "Smart" => Style::default()
                        .fg(theme::BLUE)
                        .add_modifier(Modifier::BOLD),
                    "Balanced" => Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                    "Fast" => Style::default()
                        .fg(theme::YELLOW)
                        .add_modifier(Modifier::BOLD),
                    _ => theme::dim_style(),
                };
                let ctx = format_context(m.context_window);
                // #7774: flag a guessed context window (the runtime falls
                // back to a conservative 8192-token default in this case)
                // so the operator knows to set an override.
                let ctx_marker = if m.context_window_is_estimated {
                    format!("{ctx}*")
                } else {
                    ctx
                };
                let cost = format!("${:.2}/${:.2}", m.cost_input, m.cost_output);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<28}", widgets::truncate(&m.id, 27)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {:<14}", widgets::truncate(&m.provider, 13)),
                        theme::dim_style(),
                    ),
                    Span::styled(format!(" {:<10}", m.tier), tier_style),
                    Span::styled(
                        format!(" {:<11}", ctx_marker),
                        if m.context_window_is_estimated {
                            Style::default().fg(theme::RED)
                        } else {
                            Style::default().fg(theme::YELLOW)
                        },
                    ),
                    Span::styled(format!(" {cost}"), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.model_list);
    }
}

/// The context_window / max_output_tokens override editor (#7774) — opened
/// via `[e]` on a selected model in the Models sub-tab, once the existing
/// `ModelOverrides` entity has round-tripped through `GET
/// /api/models/overrides/{id}` so saving doesn't clobber other overrides.
fn draw_model_edit(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // spacer
        Constraint::Length(1), // context_window field
        Constraint::Length(1), // max_output_tokens field
        Constraint::Length(1), // spacer
        Constraint::Min(1),    // estimated-window notice
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {} — {}",
                crate::i18n::t("tui-settings-models-edit-title"),
                state.model_edit_key
            ),
            Style::default()
                .fg(theme::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )])),
        chunks[0],
    );

    if state.model_edit_loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-settings-models-loading")),
            chunks[2],
        );
        return;
    }

    // The catalog/effective value shown as a placeholder when the field is
    // empty (mirrors the dashboard's "Auto = catalog default" convention).
    let catalog_defaults = state
        .models
        .iter()
        .find(|m| format!("{}:{}", m.provider, m.id) == state.model_edit_key);

    draw_model_edit_field(
        f,
        chunks[2],
        &crate::i18n::t("tui-settings-models-edit-context-window"),
        &state.model_edit_ctx,
        state.model_edit_field == 0,
        catalog_defaults.map(|m| format_context(m.context_window)),
    );
    draw_model_edit_field(
        f,
        chunks[3],
        &crate::i18n::t("tui-settings-models-edit-max-output"),
        &state.model_edit_max_out,
        state.model_edit_field == 1,
        catalog_defaults.map(|m| format_context(m.max_output_tokens)),
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {}",
                crate::i18n::t("tui-settings-models-edit-empty-hint")
            ),
            theme::dim_style(),
        )])),
        chunks[5],
    );
}

fn draw_model_edit_field(
    f: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    catalog_default: Option<String>,
) {
    let label_style = if focused {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::dim_style()
    };
    let marker = if focused { "❯" } else { " " };
    let placeholder = catalog_default
        .map(|d| format!("— ({d})"))
        .unwrap_or_else(|| "—".to_string());
    let display = if value.is_empty() {
        placeholder
    } else {
        value.to_string()
    };
    let value_style = if focused {
        theme::input_style()
    } else {
        theme::dim_style()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {marker} {label:<24}"), label_style),
            Span::styled(display, value_style),
            if focused {
                Span::styled(
                    "\u{2588}",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::SLOW_BLINK),
                )
            } else {
                Span::raw("")
            },
        ])),
        area,
    );
}

fn draw_tools(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
    ])
    .split(area);

    let name_hdr = crate::i18n::t("tui-settings-tools-header-name");
    let desc_hdr = crate::i18n::t("tui-settings-tools-header-desc");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<24} {}", name_hdr, desc_hdr),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.tools.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-tools-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .tools
            .iter()
            .map(|t| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<24}", widgets::truncate(&t.name, 23)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&t.description, 50)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.tool_list);
    }
}

fn format_context(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        format!("{n}")
    }
}
