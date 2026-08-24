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
    Capabilities,
    Backups,
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
    // ── Capability routing (kernel-global `[capabilities]`) ────────────
    /// The kernel-global routing block as last fetched from `GET /api/config`.
    /// A capability with no entry here is inherited from the historical
    /// `[media]` selectors and then from env-var auto-detection, which is what
    /// the row renders as "auto".
    pub capability_routing: librefang_types::media::CapabilityRouting,
    pub capability_list: ListState,
    /// True while a row's `provider/model` value is being typed.
    pub capability_edit_mode: bool,
    pub capability_edit_buf: String,
    // ── Backups ────────────────────────────────────────────────────────
    pub backups: Vec<BackupEntry>,
    pub backup_list: ListState,
    /// Clone mode: restore everything except config.toml.
    pub backup_keep_config: bool,
    /// Components selected for the restore; empty = restore all.
    pub backup_components: Vec<String>,
    pub backup_msg: String,
}

/// One backup file, mirrored from `GET /api/backups`.
#[derive(Clone)]
pub struct BackupEntry {
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String,
}

#[derive(Debug)]
pub enum SettingsAction {
    Continue,
    RefreshProviders,
    RefreshBackups,
    CreateBackup,
    RestoreBackup {
        filename: String,
        keep_config: bool,
        components: Vec<String>,
    },
    DeleteBackup(String),
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
    /// Load the kernel-global `[capabilities]` block from `GET /api/config`.
    RefreshCapabilities,
    /// Persist one capability's `provider/model` spec.
    ///
    /// `spec` empty means "clear the nomination" — the capability falls back
    /// to the `[media]` selectors and then auto-detection, which is the same
    /// thing as never having set it.
    SaveCapabilityRouting {
        capability: librefang_types::media::MediaCapability,
        spec: String,
    },
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
            capability_routing: librefang_types::media::CapabilityRouting::default(),
            capability_list: ListState::default(),
            capability_edit_mode: false,
            capability_edit_buf: String::new(),
            backup_keep_config: false,
            backup_components: Vec::new(),
            backups: Vec::new(),
            backup_list: ListState::default(),
            backup_msg: String::new(),
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

        // Before sub-tab switching: while a capability value is being typed,
        // digits are part of a model id (`gpt-4o`), not a tab shortcut.
        if self.capability_edit_mode {
            return self.handle_capability_edit(key);
        }

        if self.input_mode {
            return self.handle_input(key);
        }

        // Sub-tab switching. Skips while the Backups tab is active: the
        // number keys 1-8 toggle restore components there.
        if !self.input_mode && self.sub != SettingsSub::Backups {
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
                KeyCode::Char('4') => {
                    self.sub = SettingsSub::Capabilities;
                    return SettingsAction::RefreshCapabilities;
                }
                KeyCode::Char('5') => {
                    self.sub = SettingsSub::Backups;
                    return SettingsAction::RefreshBackups;
                }
                _ => {}
            }
        }

        match self.sub {
            SettingsSub::Providers => self.handle_providers(key),
            SettingsSub::Models => self.handle_models(key),
            SettingsSub::Tools => self.handle_tools(key),
            SettingsSub::Capabilities => self.handle_capabilities(key),
            SettingsSub::Backups => self.handle_backups(key),
        }
    }

    /// The capabilities the tab lists, in the order they are rendered.
    /// Understanding first — those are the two that change how an inbound
    /// message is handled.
    pub const CAPABILITY_ROWS: [librefang_types::media::MediaCapability; 6] =
        librefang_types::media::CapabilityRouting::ALL;

    /// The `provider/model` spec currently stored for `capability`, or an
    /// empty string when nothing is nominated.
    pub fn capability_spec(&self, capability: librefang_types::media::MediaCapability) -> String {
        match self.capability_routing.get(capability) {
            Some(target) => match (&target.provider, &target.model) {
                (Some(p), Some(m)) => format!("{p}/{m}"),
                (Some(p), None) => p.clone(),
                // A model-only override inherits the provider; `/model` is how
                // that round-trips through a single text field.
                (None, Some(m)) => format!("/{m}"),
                (None, None) => String::new(),
            },
            None => String::new(),
        }
    }

    fn handle_capabilities(&mut self, key: KeyEvent) -> SettingsAction {
        let total = Self::CAPABILITY_ROWS.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.capability_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.capability_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.capability_list.selected().unwrap_or(0);
                self.capability_list.select(Some((i + 1) % total));
            }
            KeyCode::Enter => {
                let i = self.capability_list.selected().unwrap_or(0);
                // Seed the buffer with the current value so editing is a tweak
                // rather than a retype, and so pressing Enter twice is a no-op
                // instead of a silent clear.
                self.capability_edit_buf = self.capability_spec(Self::CAPABILITY_ROWS[i]);
                self.capability_edit_mode = true;
            }
            KeyCode::Char('r') => return SettingsAction::RefreshCapabilities,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_capability_edit(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.capability_edit_mode = false;
                self.capability_edit_buf.clear();
            }
            KeyCode::Enter => {
                self.capability_edit_mode = false;
                let i = self.capability_list.selected().unwrap_or(0);
                let capability = Self::CAPABILITY_ROWS[i];
                let spec = std::mem::take(&mut self.capability_edit_buf)
                    .trim()
                    .to_string();
                // Reflect the change locally so the row updates on the next
                // frame; the refresh that follows the save confirms it.
                let target = if spec.is_empty() {
                    None
                } else {
                    Some(librefang_types::media::CapabilityTarget::parse(&spec))
                };
                self.capability_routing.set(capability, target);
                return SettingsAction::SaveCapabilityRouting { capability, spec };
            }
            KeyCode::Backspace => {
                self.capability_edit_buf.pop();
            }
            KeyCode::Char(c) => self.capability_edit_buf.push(c),
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_backups(&mut self, key: KeyEvent) -> SettingsAction {
        const BACKUP_COMPONENTS: [&str; 8] = [
            "config",
            "cron_jobs",
            "hand_state",
            "custom_models",
            "agents",
            "skills",
            "workflows",
            "data",
        ];
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let total = self.backups.len();
                if total > 0 {
                    let i = self.backup_list.selected().unwrap_or(0);
                    self.backup_list
                        .select(Some(if i == 0 { total - 1 } else { i - 1 }));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let total = self.backups.len();
                if total > 0 {
                    let i = self.backup_list.selected().unwrap_or(0);
                    self.backup_list.select(Some((i + 1) % total));
                }
            }
            KeyCode::Char('c') => return SettingsAction::CreateBackup,
            KeyCode::Char('r') => {
                if let Some(i) = self.backup_list.selected() {
                    if let Some(b) = self.backups.get(i) {
                        return SettingsAction::RestoreBackup {
                            filename: b.filename.clone(),
                            keep_config: self.backup_keep_config,
                            components: self.backup_components.clone(),
                        };
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(i) = self.backup_list.selected() {
                    if let Some(b) = self.backups.get(i) {
                        return SettingsAction::DeleteBackup(b.filename.clone());
                    }
                }
            }
            KeyCode::Char(' ') => {
                self.backup_keep_config = !self.backup_keep_config;
            }
            KeyCode::Char(c) if ('1'..='8').contains(&c) => {
                let idx = (c as u8 - b'1') as usize;
                let comp = BACKUP_COMPONENTS[idx];
                if self.backup_components.iter().any(|x| x == comp) {
                    self.backup_components.retain(|x| x != comp);
                } else {
                    self.backup_components.push(comp.to_string());
                }
            }
            _ => {}
        }
        SettingsAction::Continue
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
        SettingsSub::Capabilities => draw_capabilities(f, chunks[2], state),
        SettingsSub::Backups => draw_backups(f, chunks[2], state),
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
        SettingsSub::Capabilities if state.capability_edit_mode => {
            crate::i18n::t("tui-settings-hints-capabilities-edit")
        }
        SettingsSub::Capabilities => crate::i18n::t("tui-settings-hints-capabilities"),
        SettingsSub::Backups => crate::i18n::t("tui-settings-hints-backups"),
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
        (
            SettingsSub::Capabilities,
            crate::i18n::t("tui-settings-tab-capabilities"),
        ),
        (
            SettingsSub::Backups,
            crate::i18n::t("tui-settings-tab-backups"),
        ),
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

fn draw_backups(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(area);

    let mut hints = vec![Span::styled(
        format!(
            "  {}: {}",
            crate::i18n::t("tui-settings-backup-keep-config"),
            if state.backup_keep_config {
                "on"
            } else {
                "off"
            },
        ),
        if state.backup_keep_config {
            theme::tab_active()
        } else {
            theme::tab_inactive()
        },
    )];
    let comps = [
        "config",
        "cron_jobs",
        "hand_state",
        "custom_models",
        "agents",
        "skills",
        "workflows",
        "data",
    ];
    for (idx, c) in comps.iter().enumerate() {
        let on = state.backup_components.iter().any(|x| x == c);
        hints.push(Span::styled(
            format!("  {}{}{}", idx + 1, c, if on { "+" } else { "-" }),
            if on {
                theme::tab_active()
            } else {
                theme::tab_inactive()
            },
        ));
    }
    f.render_widget(Paragraph::new(Line::from(hints)), chunks[0]);

    if state.backups.is_empty() {
        let msg = if state.backup_msg.is_empty() {
            crate::i18n::t("tui-settings-backup-empty")
        } else {
            state.backup_msg.clone()
        };
        f.render_widget(Paragraph::new(msg), chunks[1]);
    } else {
        let items: Vec<ListItem> = state
            .backups
            .iter()
            .map(|b| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {}", b.filename),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "  {}",
                            crate::i18n::t_args(
                                "tui-settings-backup-size-bytes",
                                &[("bytes", &b.size_bytes.to_string())],
                            )
                        ),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                    Span::styled(
                        format!("  {}", b.created_at),
                        Style::default().fg(theme::TEXT_TERTIARY),
                    ),
                ]))
            })
            .collect();
        f.render_stateful_widget(
            ratatui::widgets::List::new(items),
            chunks[1],
            &mut state.backup_list,
        );
    }
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

/// Human-readable label for one media capability.
///
/// Localised rather than printed as the raw serde spelling, per the TUI
/// convention that labels are human-friendly and the machine-readable name
/// belongs in the hint.
fn capability_label(capability: librefang_types::media::MediaCapability) -> String {
    use librefang_types::media::MediaCapability as C;
    let key = match capability {
        C::ImageUnderstanding => "tui-settings-capability-image-understanding",
        C::SpeechToText => "tui-settings-capability-speech-to-text",
        C::ImageGeneration => "tui-settings-capability-image-generation",
        C::TextToSpeech => "tui-settings-capability-text-to-speech",
        C::VideoGeneration => "tui-settings-capability-video-generation",
        C::MusicGeneration => "tui-settings-capability-music-generation",
        // `MediaCapability` is `#[non_exhaustive]`, so a capability added
        // upstream would not break this build. Fall back to its serde
        // spelling — an unglamorous label beats an empty row, and the row is
        // still editable.
        _ => return capability.to_string(),
    };
    crate::i18n::t(key)
}

fn draw_capabilities(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(2), // edit line + explainer
    ])
    .split(area);

    let cap_hdr = crate::i18n::t("tui-settings-capabilities-header-capability");
    let target_hdr = crate::i18n::t("tui-settings-capabilities-header-target");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<22} {}", cap_hdr, target_hdr),
            theme::table_header(),
        )])),
        chunks[0],
    );

    let auto = crate::i18n::t("tui-settings-capabilities-auto");
    let items: Vec<ListItem> = SettingsState::CAPABILITY_ROWS
        .iter()
        .map(|cap| {
            let spec = state.capability_spec(*cap);
            // An unset capability is not blank — it is "auto", and saying so is
            // the difference between "nothing is configured" and "nobody has
            // looked at this yet".
            let (text, style) = if spec.is_empty() {
                (auto.clone(), theme::dim_style())
            } else {
                (spec, Style::default().fg(theme::CYAN))
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {:<22}", widgets::truncate(&capability_label(*cap), 21)),
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(format!(" {}", widgets::truncate(&text, 48)), style),
            ]))
        })
        .collect();

    let list = widgets::themed_list(items);
    f.render_stateful_widget(list, chunks[1], &mut state.capability_list);

    let footer = if state.capability_edit_mode {
        Line::from(vec![
            Span::styled("  > ", Style::default().fg(theme::CYAN)),
            Span::raw(state.capability_edit_buf.clone()),
            Span::styled("_", Style::default().fg(theme::CYAN)),
        ])
    } else {
        Line::from(vec![Span::styled(
            format!(
                "  {}",
                crate::i18n::t("tui-settings-capabilities-explainer")
            ),
            theme::dim_style(),
        )])
    };
    f.render_widget(Paragraph::new(footer), chunks[2]);
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

#[cfg(test)]
mod backups_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn tab_5_opens_backups_and_requests_a_refresh() {
        let mut state = SettingsState::new();
        let action = state.handle_key(key(KeyCode::Char('5')));
        assert!(matches!(state.sub, SettingsSub::Backups));
        assert!(matches!(action, SettingsAction::RefreshBackups));
    }

    #[test]
    fn space_toggles_clone_mode() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Backups;
        state.handle_key(key(KeyCode::Char(' ')));
        assert!(state.backup_keep_config);
        state.handle_key(key(KeyCode::Char(' ')));
        assert!(!state.backup_keep_config);
    }

    #[test]
    fn number_keys_toggle_components() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Backups;
        state.handle_key(key(KeyCode::Char('1')));
        assert!(state.backup_components.iter().any(|c| c == "config"));
        state.handle_key(key(KeyCode::Char('1')));
        assert!(!state.backup_components.iter().any(|c| c == "config"));
    }

    #[test]
    fn restore_sends_the_selection_with_the_filename() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Backups;
        state.backups.push(BackupEntry {
            filename: "b.zip".to_string(),
            size_bytes: 10,
            created_at: String::new(),
        });
        state.backup_list.select(Some(0));
        state.handle_key(key(KeyCode::Char(' ')));
        state.handle_key(key(KeyCode::Char('5')));

        let action = state.handle_key(key(KeyCode::Char('r')));
        match action {
            SettingsAction::RestoreBackup {
                filename,
                keep_config,
                components,
            } => {
                assert_eq!(filename, "b.zip");
                assert!(keep_config);
                assert!(components.iter().any(|c| c == "agents"));
            }
            other => panic!("expected restore, got {other:?}"),
        }
    }

    #[test]
    fn create_is_one_keypress() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Backups;
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('c'))),
            SettingsAction::CreateBackup
        ));
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use librefang_types::media::{CapabilityRouting, MediaCapability};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(state: &mut SettingsState, s: &str) {
        for c in s.chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn tab_4_opens_capabilities_and_requests_a_refresh() {
        let mut state = SettingsState::new();
        let action = state.handle_key(key(KeyCode::Char('4')));
        assert!(matches!(state.sub, SettingsSub::Capabilities));
        assert!(matches!(action, SettingsAction::RefreshCapabilities));
    }

    #[test]
    fn an_unset_capability_renders_its_spec_as_empty() {
        let state = SettingsState::new();
        assert_eq!(
            state.capability_spec(MediaCapability::ImageUnderstanding),
            ""
        );
    }

    #[test]
    fn capability_spec_renders_each_target_shape() {
        let mut state = SettingsState::new();
        state.capability_routing =
            toml::from_str::<CapabilityRouting>("image_understanding = \"openai/gpt-4o\"\n")
                .unwrap();
        assert_eq!(
            state.capability_spec(MediaCapability::ImageUnderstanding),
            "openai/gpt-4o"
        );

        state.capability_routing =
            toml::from_str::<CapabilityRouting>("image_understanding = \"openai\"\n").unwrap();
        assert_eq!(
            state.capability_spec(MediaCapability::ImageUnderstanding),
            "openai"
        );

        // A model-only override inherits the provider; `/model` is how that
        // survives a round-trip through the single text field.
        state.capability_routing = toml::from_str::<CapabilityRouting>(
            "image_understanding = { model = \"gpt-4o-mini\" }\n",
        )
        .unwrap();
        assert_eq!(
            state.capability_spec(MediaCapability::ImageUnderstanding),
            "/gpt-4o-mini"
        );
    }

    #[test]
    fn enter_seeds_the_editor_with_the_current_value() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Capabilities;
        state.capability_routing =
            toml::from_str::<CapabilityRouting>("image_understanding = \"openai/gpt-4o\"\n")
                .unwrap();
        state.capability_list.select(Some(0));

        state.handle_key(key(KeyCode::Enter));
        assert!(state.capability_edit_mode);
        assert_eq!(state.capability_edit_buf, "openai/gpt-4o");
    }

    #[test]
    fn typing_a_spec_and_pressing_enter_saves_it() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Capabilities;
        state.capability_list.select(Some(0));
        state.handle_key(key(KeyCode::Enter));
        type_str(&mut state, "groq/llama");

        let action = state.handle_key(key(KeyCode::Enter));
        assert!(!state.capability_edit_mode);
        match action {
            SettingsAction::SaveCapabilityRouting { capability, spec } => {
                assert_eq!(capability, MediaCapability::ImageUnderstanding);
                assert_eq!(spec, "groq/llama");
            }
            other => panic!("expected a save, got {other:?}"),
        }
        // Reflected locally so the row updates on the next frame.
        assert_eq!(
            state.capability_spec(MediaCapability::ImageUnderstanding),
            "groq/llama"
        );
    }

    /// Digits are part of a model id (`gpt-4o`) while the editor is open, so
    /// they must not be swallowed as sub-tab shortcuts.
    #[test]
    fn digits_typed_into_the_editor_do_not_switch_tabs() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Capabilities;
        state.capability_list.select(Some(0));
        state.handle_key(key(KeyCode::Enter));
        type_str(&mut state, "openai/gpt-4o");

        assert!(matches!(state.sub, SettingsSub::Capabilities));
        assert_eq!(state.capability_edit_buf, "openai/gpt-4o");
    }

    #[test]
    fn esc_discards_the_edit_and_leaves_the_value_alone() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Capabilities;
        state.capability_routing =
            toml::from_str::<CapabilityRouting>("image_understanding = \"openai\"\n").unwrap();
        state.capability_list.select(Some(0));
        state.handle_key(key(KeyCode::Enter));
        type_str(&mut state, "-nonsense");

        state.handle_key(key(KeyCode::Esc));
        assert!(!state.capability_edit_mode);
        assert_eq!(
            state.capability_spec(MediaCapability::ImageUnderstanding),
            "openai"
        );
    }

    #[test]
    fn clearing_the_field_clears_the_nomination() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Capabilities;
        state.capability_routing =
            toml::from_str::<CapabilityRouting>("image_understanding = \"openai\"\n").unwrap();
        state.capability_list.select(Some(0));
        state.handle_key(key(KeyCode::Enter));
        for _ in 0.."openai".len() {
            state.handle_key(key(KeyCode::Backspace));
        }

        let action = state.handle_key(key(KeyCode::Enter));
        match action {
            SettingsAction::SaveCapabilityRouting { spec, .. } => assert_eq!(spec, ""),
            other => panic!("expected a save, got {other:?}"),
        }
        assert_eq!(
            state.capability_spec(MediaCapability::ImageUnderstanding),
            "",
            "an emptied field must fall back to auto-detection, not pin an empty provider"
        );
    }

    #[test]
    fn navigation_wraps_around_every_capability_row() {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Capabilities;
        state.capability_list.select(Some(0));

        state.handle_key(key(KeyCode::Up));
        assert_eq!(
            state.capability_list.selected(),
            Some(SettingsState::CAPABILITY_ROWS.len() - 1)
        );
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.capability_list.selected(), Some(0));
    }

    #[test]
    fn every_capability_row_has_a_localised_label() {
        for cap in SettingsState::CAPABILITY_ROWS {
            let label = capability_label(cap);
            assert!(!label.is_empty(), "no label for {cap}");
            // A missing Fluent key resolves to the key itself; that would ship
            // `tui-settings-capability-…` as a visible label.
            assert!(
                !label.starts_with("tui-settings-capability-"),
                "missing translation for {cap}: {label}"
            );
        }
    }
}
