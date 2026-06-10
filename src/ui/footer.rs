use crate::app::App;
use crate::locale::t;
use crate::theme::Theme;
use chrono::Timelike;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::truncate_str;

pub(crate) fn draw_footer(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    // Filter input mode: show filter bar instead of normal keybindings
    if app.filter_active {
        let visible_count = app.visible_indices().len();
        let count = format!(
            "{}/{} {}",
            visible_count,
            app.sessions.len(),
            t("footer.sessions")
        );
        let suffix = if area.width <= 80 {
            format!("_  {}", count)
        } else {
            format!(
                "_  {}  (Esc {}, Enter {})",
                count,
                t("footer.esc_clear")
                    .split(',')
                    .next()
                    .unwrap_or(&t("footer.esc_clear")),
                t("footer.esc_clear")
                    .split(',')
                    .nth(1)
                    .unwrap_or("keep")
                    .trim()
            )
        };
        let filter_w = (area.width as usize).saturating_sub(2 + suffix.chars().count());
        let spans = vec![
            Span::styled(" /", Style::default().fg(theme.hi_fg)),
            Span::styled(
                truncate_str(&app.filter_text, filter_w),
                Style::default().fg(theme.title),
            ),
            Span::styled(suffix, Style::default().fg(theme.inactive_fg)),
        ];
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let has_tmux = std::env::var("TMUX").is_ok();
    let compact = area.width <= 80;
    let ultra_compact = area.width <= 70;

    let mut spans = vec![
        Span::styled(" ↑↓", Style::default().fg(theme.hi_fg)),
        Span::styled(
            format!(" {} ", t("footer.select")),
            Style::default().fg(theme.main_fg),
        ),
    ];
    if has_tmux && !ultra_compact {
        spans.push(Span::styled("↵", Style::default().fg(theme.hi_fg)));
        spans.push(Span::styled(
            format!(" {} ", t("footer.jump")),
            Style::default().fg(theme.main_fg),
        ));
    }
    if compact {
        spans.push(Span::styled("←→", Style::default().fg(theme.hi_fg)));
        spans.push(Span::styled(" tabs ", Style::default().fg(theme.main_fg)));
    }
    if !ultra_compact {
        spans.push(Span::styled("x", Style::default().fg(theme.hi_fg)));
        spans.push(Span::styled(
            format!(" {} ", t("footer.kill")),
            Style::default().fg(theme.main_fg),
        ));
    }
    spans.push(Span::styled("/", Style::default().fg(theme.hi_fg)));
    spans.push(Span::styled(
        format!(" {} ", t("footer.filter")),
        Style::default().fg(theme.main_fg),
    ));
    if !ultra_compact {
        spans.push(Span::styled("v", Style::default().fg(theme.hi_fg)));
        spans.push(Span::styled(
            format!(" {} ", t("footer.view")),
            Style::default().fg(theme.main_fg),
        ));
        if !compact {
            spans.push(Span::styled("c", Style::default().fg(theme.hi_fg)));
            spans.push(Span::styled(
                format!(" {} ", t("footer.config")),
                Style::default().fg(theme.main_fg),
            ));
        }
        spans.push(Span::styled("?", Style::default().fg(theme.hi_fg)));
        spans.push(Span::styled(
            format!(" {} ", t("footer.help")),
            Style::default().fg(theme.main_fg),
        ));
    }
    spans.push(Span::styled("q", Style::default().fg(theme.hi_fg)));
    spans.push(Span::styled(
        format!(" {} ", t("footer.quit")),
        Style::default().fg(theme.main_fg),
    ));

    // Show active filter or transient status
    if !compact && !app.filter_text.is_empty() {
        spans.push(Span::styled(
            format!(" /{} ", app.filter_text),
            Style::default().fg(theme.status_fg),
        ));
    } else if !compact {
        let status_text = app
            .status_msg
            .as_ref()
            .filter(|(_, when)| when.elapsed().as_secs() < 3)
            .map(|(msg, _)| msg.as_str());
        if let Some(msg) = status_text {
            spans.push(Span::styled(
                format!(" {msg} "),
                Style::default().fg(theme.status_fg),
            ));
        } else {
            spans.push(Span::styled(
                t("footer.auto"),
                Style::default().fg(theme.inactive_fg),
            ));
        }
    }

    // Peak hours warning: US business hours = PT 5am–11am = UTC 12:00–18:00
    let peak_info = {
        let now = chrono::Utc::now();
        let hour = now.hour();
        if (12..18).contains(&hour) {
            let mins_left = (18 - hour) * 60 - now.minute();
            let h = mins_left / 60;
            let m = mins_left % 60;
            let peak_label = t("footer.peak_hours");
            let resets_in = t("footer.resets_in");
            Some(format!("⚡{} ({} {}h{:02}m)", peak_label, resets_in, h, m))
        } else {
            None
        }
    };
    if let Some(ref peak) = peak_info.filter(|_| !compact) {
        spans.push(Span::styled(
            format!(" {peak} "),
            Style::default().fg(theme.warning_fg),
        ));
    }

    let visible_count = app.visible_indices().len();
    let sessions_label = t("footer.sessions");
    let count_label = if visible_count < app.sessions.len() {
        format!(
            "{}/{} {}",
            visible_count,
            app.sessions.len(),
            sessions_label
        )
    } else {
        format!("{} {}", app.sessions.len(), sessions_label)
    };
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let count_w = count_label.chars().count();
    if used + count_w < area.width as usize {
        let remaining = (area.width as usize).saturating_sub(used + count_w);
        spans.push(Span::styled(
            format!("{:>width$}", count_label, width = remaining),
            Style::default().fg(theme.graph_text),
        ));
    }

    let mut lines: Vec<Line> = vec![Line::from(spans)];
    if let Some(events_line) = events_status_line(app, area.width as usize, theme) {
        lines.push(events_line);
    }

    f.render_widget(Paragraph::new(lines), area);
}

/// Build the optional second footer line that surfaces the events
/// publisher state. Returns None when the publisher is disabled
/// (strict-mode: nothing rendered, the user gets a status hint via `e`).
fn events_status_line<'a>(app: &App, max_width: usize, theme: &Theme) -> Option<Line<'a>> {
    let publisher = app.publisher();
    if !publisher.is_enabled() {
        return None;
    }
    let state = if publisher.is_paused() {
        "paused"
    } else {
        "on"
    };
    let conns = publisher.conn_count();
    let path_display = publisher
        .socket_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let hint = "[e]";
    // Reserve fixed spans + a couple of spaces around the path.
    let prefix = format!(" events: {state} • {conns} conns • ");
    let suffix = format!("     {hint}");
    let budget = max_width
        .saturating_sub(prefix.chars().count())
        .saturating_sub(suffix.chars().count());
    let path_str = if budget == 0 {
        String::new()
    } else {
        truncate_middle(&path_display, budget)
    };

    Some(Line::from(vec![
        Span::styled(prefix, Style::default().fg(theme.inactive_fg)),
        Span::styled(path_str, Style::default().fg(theme.main_fg)),
        Span::styled("     ", Style::default().fg(theme.inactive_fg)),
        Span::styled(hint.to_string(), Style::default().fg(theme.hi_fg)),
    ]))
}

/// Middle-truncate `s` to at most `max` chars by keeping a leading and
/// trailing slice and joining them with `…`. Returns `s` unchanged when
/// it already fits.
pub(crate) fn truncate_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if max == 0 {
        return String::new();
    }
    if n <= max {
        return s.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    // Tunable split: keep more of the tail (basename) than the head.
    let keep = max - 1; // budget for actual content chars
    let tail_share = ((keep * 2) / 3).max(1);
    let head_share = keep - tail_share;
    let head: String = s.chars().take(head_share).collect();
    let tail: String = s.chars().skip(n - tail_share).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::truncate_middle;

    #[test]
    fn truncate_middle_returns_original_when_short() {
        assert_eq!(truncate_middle("abc", 10), "abc");
        assert_eq!(truncate_middle("abcdefghij", 10), "abcdefghij");
    }

    #[test]
    fn truncate_middle_middle_truncates_when_too_long() {
        let s = "/var/folders/xy/zz/T/abtop.sock";
        let out = truncate_middle(s, 16);
        assert_eq!(out.chars().count(), 16);
        assert!(out.contains('…'));
        // First few chars preserved.
        assert!(out.starts_with("/var"));
        // Trailing basename preserved.
        assert!(out.ends_with("abtop.sock"));
    }

    #[test]
    fn truncate_middle_zero_max_is_empty() {
        assert_eq!(truncate_middle("hello", 0), "");
    }

    #[test]
    fn truncate_middle_unit_max_is_ellipsis() {
        assert_eq!(truncate_middle("hello", 1), "…");
    }
}
