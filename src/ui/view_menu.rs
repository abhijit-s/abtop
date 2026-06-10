use crate::app::App;
use crate::locale::t;
use crate::theme::Theme;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Items rendered in the `v` overlay. The `key` char is the hotkey accepted
/// while the overlay is open; `accessor` returns the current on/off state for
/// rendering the indicator.
pub(crate) struct ViewItem {
    pub key: char,
    pub label: &'static str,
    pub state: ViewState,
}

pub(crate) enum ViewState {
    On,
    Off,
    Action,
    /// Free-form right-hand label (used by the Events row).
    Custom(String),
}

pub(crate) fn items(app: &App) -> Vec<ViewItem> {
    use ViewState::*;
    let bool_state = |b: bool| if b { On } else { Off };
    vec![
        ViewItem {
            key: 'T',
            label: t("view.tree_view").leak(),
            state: bool_state(app.tree_view),
        },
        ViewItem {
            key: 'l',
            label: t("view.timeline").leak(),
            state: bool_state(app.show_timeline),
        },
        ViewItem {
            key: 'f',
            label: t("view.file_audit").leak(),
            state: bool_state(app.show_file_audit),
        },
        ViewItem {
            key: '1',
            label: t("view.context_panel").leak(),
            state: bool_state(app.show_context),
        },
        ViewItem {
            key: '2',
            label: t("view.quota_panel").leak(),
            state: bool_state(app.show_quota),
        },
        ViewItem {
            key: '3',
            label: t("view.tokens_panel").leak(),
            state: bool_state(app.show_tokens),
        },
        ViewItem {
            key: '4',
            label: t("view.projects_panel").leak(),
            state: bool_state(app.show_projects),
        },
        ViewItem {
            key: '5',
            label: t("view.ports_panel").leak(),
            state: bool_state(app.show_ports),
        },
        ViewItem {
            key: '6',
            label: t("view.sessions_panel").leak(),
            state: bool_state(app.show_sessions),
        },
        ViewItem {
            key: '7',
            label: t("view.mcp_servers_panel").leak(),
            state: bool_state(app.show_mcp),
        },
        ViewItem {
            key: 'M',
            label: t("view.mcp_session_hide").leak(),
            state: bool_state(app.mcp_suppress_sessions),
        },
        ViewItem {
            key: 't',
            label: t("view.cycle_theme").leak(),
            state: Action,
        },
        ViewItem {
            key: 'e',
            label: "Events",
            state: events_state(app),
        },
    ]
}

fn events_state(app: &App) -> ViewState {
    let publisher = app.publisher();
    if !publisher.is_enabled() {
        return ViewState::Custom("disabled (start with --events)".to_string());
    }
    if publisher.is_paused() {
        ViewState::Custom("paused".to_string())
    } else {
        ViewState::On
    }
}

pub(crate) fn draw_view_overlay(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let entries = items(app);
    let extra_detail_lines: u16 = if app.publisher().is_enabled() { 3 } else { 0 };
    let popup_w = 60u16.min(area.width.saturating_sub(4));
    let popup_h =
        (entries.len() as u16 + extra_detail_lines + 4).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_w)) / 2;
    let y = (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    f.render_widget(Clear, popup);

    let view_title = t("view.title");
    let block = Block::default()
        .style(Style::default().bg(theme.main_bg))
        .title(
            Line::from(vec![Span::styled(
                view_title.clone(),
                Style::default()
                    .fg(theme.title)
                    .add_modifier(Modifier::BOLD),
            )])
            .alignment(Alignment::Center),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.cpu_box));
    f.render_widget(block, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    let on_str = t("view.on");
    let off_str = t("view.off");
    let action_str = t("view.action");

    let mut lines: Vec<Line> = Vec::with_capacity(entries.len() + 6);
    for item in &entries {
        let (state_str, state_style) = match &item.state {
            ViewState::On => (on_str.clone(), Style::default().fg(theme.proc_misc)),
            ViewState::Off => (off_str.clone(), Style::default().fg(theme.inactive_fg)),
            ViewState::Action => (action_str.clone(), Style::default().fg(theme.session_id)),
            ViewState::Custom(label) => {
                let style = if label == "paused" {
                    Style::default().fg(theme.warning_fg)
                } else if label.starts_with("disabled") {
                    Style::default().fg(theme.inactive_fg)
                } else {
                    Style::default().fg(theme.proc_misc)
                };
                (label.clone(), style)
            }
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}  ", item.key),
                Style::default()
                    .fg(theme.hi_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<22}", item.label),
                Style::default().fg(theme.main_fg),
            ),
            Span::styled(state_str, state_style),
        ]));
    }

    // When the publisher is enabled, add a small details block for the
    // Events row: socket path, live connection count, drop counter.
    let publisher = app.publisher();
    if publisher.is_enabled() {
        let socket = publisher
            .socket_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let conns = publisher.conn_count();
        let dropped = publisher.dropped_total();
        let detail_style = Style::default().fg(theme.graph_text);
        let value_style = Style::default().fg(theme.main_fg);
        lines.push(Line::from(vec![
            Span::styled("      Socket:  ", detail_style),
            Span::styled(socket, value_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("      Conns:   ", detail_style),
            Span::styled(format!("{conns}"), value_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("      Dropped: ", detail_style),
            Span::styled(format!("{dropped}"), value_style),
        ]));
    }

    lines.push(Line::from(""));
    let key_toggle = t("view.key_toggle");
    lines.push(Line::from(Span::styled(
        key_toggle,
        Style::default().fg(theme.graph_text),
    )));

    f.render_widget(Paragraph::new(lines), inner);
}
