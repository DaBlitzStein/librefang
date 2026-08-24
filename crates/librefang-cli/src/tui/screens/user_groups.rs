//! User groups screen (#7745): config-declared groups and their members.
//!
//! Read-only, and deliberately so. Groups are declared in `config.toml` under
//! `[[user_groups]]` and their membership is resolved in memory by the kernel
//! rather than stored, so there is no record an edit key could write to. The
//! hint bar says as much, so an operator is not left hunting for an "add
//! member" binding that could not exist.

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
pub struct UserGroupInfo {
    /// Stable identifier — what ownership records point at.
    pub id: String,
    /// Display name, which may differ from the id after a rename.
    pub name: String,
    pub description: String,
    /// Member user names, already in a stable order.
    pub members: Vec<String>,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct UserGroupsState {
    pub groups: Vec<UserGroupInfo>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum UserGroupsAction {
    Continue,
    Refresh,
}

impl UserGroupsState {
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// The group the cursor is on, if any.
    pub fn selected(&self) -> Option<&UserGroupInfo> {
        self.list_state.selected().and_then(|i| self.groups.get(i))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> UserGroupsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return UserGroupsAction::Continue;
        }
        let total = self.groups.len();
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
            KeyCode::Char('r') => return UserGroupsAction::Refresh,
            _ => {}
        }
        UserGroupsAction::Continue
    }
}

impl Default for UserGroupsState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut UserGroupsState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("◍ {}", crate::i18n::t("tui-user-groups-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(3),    // list
        Constraint::Length(7), // members of the selected group
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // Header
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!("  {}", crate::i18n::t("tui-user-groups-heading")),
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  │ {}",
                        crate::i18n::t_args(
                            "tui-user-groups-count",
                            &[("count", &state.groups.len().to_string())]
                        )
                    ),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ]),
            Line::from(vec![
                Span::styled("  ", theme::table_header()),
                Span::styled(
                    format!("{:<20}", crate::i18n::t("tui-user-groups-header-id")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    format!("{:<24}", crate::i18n::t("tui-user-groups-header-name")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    crate::i18n::t("tui-user-groups-header-members"),
                    theme::table_header(),
                ),
            ]),
        ]),
        chunks[0],
    );

    // List
    if state.loading && state.groups.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-user-groups-loading")),
            chunks[1],
        );
    } else if state.groups.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-user-groups-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .groups
            .iter()
            .map(|g| {
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        format!("{:<20}", widgets::truncate(&g.id, 19)),
                        Style::default().fg(theme::PURPLE),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{:<24}", widgets::truncate(&g.name, 23)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{}", g.members.len()),
                        Style::default().fg(theme::GREEN),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    // Members of the selected group.
    let detail: Vec<Line> = match state.selected() {
        Some(group) => {
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    format!("  {}", crate::i18n::t("tui-user-groups-members-of")),
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}", group.name),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ])];
            if !group.description.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", group.description),
                    theme::dim_style(),
                )));
            }
            if group.members.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", crate::i18n::t("tui-user-groups-no-members")),
                    theme::dim_style(),
                )));
            } else {
                // Chunked so a large rota wraps instead of running off the
                // right edge of the pane.
                for row in group.members.chunks(4) {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", row.join("   ")),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    )));
                }
            }
            lines
        }
        None => vec![Line::from(Span::styled(
            format!("  {}", crate::i18n::t("tui-user-groups-select-hint")),
            theme::dim_style(),
        ))],
    };
    f.render_widget(Paragraph::new(detail), chunks[2]);

    // Hints
    let hints = if state.status_msg.is_empty() {
        crate::i18n::t("tui-user-groups-hints")
    } else {
        state.status_msg.clone()
    };
    f.render_widget(widgets::hint_bar(&format!("  {hints}")), chunks[3]);
}
