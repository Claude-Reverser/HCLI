//! Bordered command palette and catalog-backed model inspector.
use super::hcli::fit;
use super::{InlineInteractiveState, PickerEntry};
use crate::provider::openrouter::ModelInfo;
use jcode_tui_style::theme::{
    accent_color, ai_text, border_color, dim_color, selection_bg_color, user_bg,
};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

pub(super) fn catalog() -> Vec<ModelInfo> {
    crate::provider::openrouter::load_disk_cache_entry_for_namespace("hcap")
        .filter(|cache| {
            crate::config::config()
                .providers
                .get("hcap")
                .is_some_and(|profile| {
                    cache.source_api_base.as_deref().is_some_and(|source| {
                        source.trim_end_matches('/') == profile.base_url.trim_end_matches('/')
                    })
                })
        })
        .map(|cache| cache.models)
        .unwrap_or_default()
}

fn popup(input: Rect, frame: Rect, height: u16) -> Rect {
    let available = input.y.saturating_sub(frame.y);
    let gap = u16::from(available > 22);
    let inset = u16::from(input.width >= 80);
    let height = height.min(available.saturating_sub(gap));
    Rect::new(
        input.x + inset,
        input.y.saturating_sub(height + gap),
        input.width.saturating_sub(inset * 2),
        height,
    )
}

fn text(frame: &mut Frame, area: Rect, value: &str, style: Style) {
    frame.render_widget(
        Paragraph::new(fit(value, area.width as usize)).style(style),
        area,
    );
}

fn rule(frame: &mut Frame, area: Rect) {
    let area = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    text(
        frame,
        area,
        &"─".repeat(area.width as usize),
        Style::default().fg(border_color()),
    );
}

fn shell(frame: &mut Frame, area: Rect, title: &str, hint: &str, subtitle: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color()))
        .style(Style::default().fg(ai_text()))
        .title(Line::styled(
            format!(" {title} "),
            Style::default().fg(accent_color()).bold(),
        ))
        .title_bottom(Line::styled(
            format!(" {} ", fit(hint, area.width.saturating_sub(4) as usize)),
            Style::default().fg(dim_color()),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // Fill only the interior so the rounded corners don't sit on square color tiles.
    frame.render_widget(
        Block::default().style(Style::default().bg(user_bg())),
        inner,
    );
    text(
        frame,
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1),
        subtitle,
        Style::default().fg(dim_color()),
    );
    if inner.height > 1 {
        rule(frame, Rect::new(inner.x, inner.y + 1, inner.width, 1));
    }
    Rect::new(
        inner.x + 1,
        inner.y.saturating_add(2),
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(2),
    )
}

pub(super) fn commands(
    frame: &mut Frame,
    input: Rect,
    suggestions: &[(String, &'static str)],
    selected: usize,
) {
    let area = popup(
        input,
        frame.area(),
        (suggestions.len().min(6) as u16 * 2 + 4).max(6),
    );
    if area.height < 5 || area.width < 8 {
        return;
    }
    let selected = selected.min(suggestions.len().saturating_sub(1));
    let body = shell(
        frame,
        area,
        "COMMANDS",
        "↑↓ navigate · Tab complete · Enter run · Esc close",
        &format!("Find an action   ·   {} matches", suggestions.len()),
    );
    let visible = ((body.height + 1) / 2) as usize;
    let start = selected.saturating_sub(visible.saturating_sub(1));
    let command_width = (body.width as usize / 3)
        .clamp(12, 28)
        .min(body.width.saturating_sub(4) as usize);
    for (row, (command, description)) in suggestions.iter().skip(start).take(visible).enumerate() {
        let y = body.y + row as u16 * 2;
        let active = row + start == selected;
        let style = Style::default()
            .fg(if active { accent_color() } else { ai_text() })
            .bg(if active {
                selection_bg_color()
            } else {
                user_bg()
            });
        let command = fit(command, command_width);
        let padding =
            command_width.saturating_sub(unicode_width::UnicodeWidthStr::width(command.as_str()));
        let line = format!(
            "{} {}{} │ {}",
            if active { "›" } else { " " },
            command,
            " ".repeat(padding),
            description
        );
        text(
            frame,
            Rect::new(body.x, y, body.width, 1),
            &line,
            if active { style.bold() } else { style },
        );
        if y + 1 < body.bottom() {
            rule(frame, Rect::new(body.x, y + 1, body.width, 1));
        }
    }
}

fn number(value: Option<u64>) -> String {
    value
        .map(|value| {
            let s = value.to_string();
            s.chars()
                .enumerate()
                .map(|(i, c)| {
                    if i > 0 && (s.len() - i) % 3 == 0 {
                        format!(",{c}")
                    } else {
                        c.to_string()
                    }
                })
                .collect()
        })
        .unwrap_or_else(|| "—".into())
}

fn price(value: Option<&str>) -> String {
    value
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| format!("${:.2}", v * 1_000_000.0))
        .unwrap_or_else(|| "—".into())
}

fn details(entry: &PickerEntry, models: &[ModelInfo]) -> Vec<String> {
    let route = entry.active_option();
    let is_hcap = route.is_some_and(|route| route.provider.eq_ignore_ascii_case("hcap"));
    let id = entry.name.strip_prefix("hcap:").unwrap_or(&entry.name);
    let model = is_hcap
        .then(|| models.iter().find(|model| model.id == id))
        .flatten();
    let mut rows = vec![
        id.to_owned(),
        format!(
            "{}{}",
            route
                .map(|r| r.provider.as_str())
                .unwrap_or("Unknown route"),
            if entry.is_current {
                " · active"
            } else if entry.is_default {
                " · default"
            } else {
                ""
            }
        ),
    ];
    if let Some(route) = route
        && !route.available
    {
        rows.push(format!("Unavailable: {}", route.detail));
    }
    let Some(model) = model else {
        rows.push("Catalog details unavailable".into());
        if let Some(route) = route
            && !route.detail.is_empty()
        {
            rows.push(route.detail.clone());
        }
        return rows;
    };
    let extra = model.extra.as_ref();
    let field = |name: &str| extra.and_then(|extra| extra.get(name));
    rows.extend([
        "─ CAPACITY & PRICING".into(),
        format!("Context      {}", number(model.context_length)),
        format!(
            "Max output   {}",
            number(field("max_output").and_then(|v| v.as_u64()))
        ),
        format!("Input / 1M   {}", price(model.pricing.prompt.as_deref())),
        format!(
            "Output / 1M  {}",
            price(model.pricing.completion.as_deref())
        ),
        format!(
            "Wallet       {}",
            field("wallet").and_then(|v| v.as_str()).unwrap_or("—")
        ),
        format!(
            "Catalog t/s  {}",
            field("tokens_per_second")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "not reported".into())
        ),
        "─ CAPABILITIES".into(),
    ]);
    let caps = field("capabilities");
    let flag = |name: &str| match caps.and_then(|c| c.get(name)).and_then(|v| v.as_bool()) {
        Some(true) => "✓",
        Some(false) => "×",
        None => "—",
    };
    rows.push(format!(
        "Stream {}  Tools {}  Vision {}",
        flag("streaming"),
        flag("tools"),
        flag("vision")
    ));
    rows.push(format!(
        "Variant: {}",
        caps.and_then(|c| c.get("unjailed_variant"))
            .and_then(|v| v.as_str())
            .unwrap_or("—")
    ));
    rows.push("─ CATALOG ACTIVITY".into());
    rows.push(format!(
        "Requests     {}",
        number(field("requests").and_then(|v| v.as_u64()))
    ));
    let last = field("last_used")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
        .and_then(|v| chrono::DateTime::from_timestamp(v as i64, 0))
        .map(|v| v.format("%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "—".into());
    rows.push(format!("Last used    {last}"));
    rows
}

pub(super) fn models(
    frame: &mut Frame,
    input: Rect,
    picker: &InlineInteractiveState,
    catalog: &[ModelInfo],
) {
    let area = popup(input, frame.area(), 22);
    if area.height < 5 || area.width < 8 {
        return;
    }
    let selected = picker.selected.min(picker.filtered.len().saturating_sub(1));
    let subtitle = format!(
        "{} models   ·   {} / {}   ·   {}",
        picker.filtered.len(),
        if picker.filtered.is_empty() {
            0
        } else {
            selected + 1
        },
        picker.filtered.len(),
        if picker.filter.is_empty() {
            "Type to filter"
        } else {
            &picker.filter
        }
    );
    let body = shell(
        frame,
        area,
        "MODEL LIBRARY",
        "↑↓ select · Enter use · Ctrl+O default · Ctrl+N favorite",
        &subtitle,
    );
    if picker.filtered.is_empty() {
        text(
            frame,
            body,
            "  No models match. Try another search.",
            Style::default().fg(dim_color()),
        );
        return;
    }
    let entry = &picker.entries[picker.filtered[selected]];
    let (list, inspector) = if body.width >= 58 {
        let left = (body.width * 45 / 100).min(body.width.saturating_sub(32));
        for y in body.y..body.bottom() {
            text(
                frame,
                Rect::new(body.x + left, y, 1, 1),
                "│",
                Style::default().fg(border_color()),
            );
        }
        (
            Rect::new(body.x, body.y, left, body.height),
            Rect::new(
                body.x + left + 2,
                body.y,
                body.width.saturating_sub(left + 3),
                body.height,
            ),
        )
    } else {
        // Keep a small scrollable list and a stacked inspector on narrow screens.
        let list_height = body.height.min(3);
        rule(
            frame,
            Rect::new(body.x, body.y + list_height, body.width, 1),
        );
        (
            Rect::new(body.x, body.y, body.width, list_height),
            Rect::new(
                body.x + 1,
                body.y + list_height + 1,
                body.width.saturating_sub(2),
                body.height.saturating_sub(list_height + 1),
            ),
        )
    };
    let visible = ((list.height + 1) / 2) as usize;
    let start = selected.saturating_sub(visible.saturating_sub(1));
    for (row, index) in picker.filtered.iter().skip(start).take(visible).enumerate() {
        let entry = &picker.entries[*index];
        let active = row + start == selected;
        let marker = if active {
            "›"
        } else if entry.is_current {
            "●"
        } else {
            " "
        };
        let tag = if entry.is_default {
            "  default"
        } else if entry.is_favorite {
            "  ★"
        } else {
            ""
        };
        let name = super::ui::header::header_model_display_name(
            &entry.name,
            entry
                .active_option()
                .map(|o| o.provider.as_str())
                .unwrap_or(""),
        );
        let style = Style::default()
            .fg(if active { accent_color() } else { ai_text() })
            .bg(if active {
                selection_bg_color()
            } else {
                user_bg()
            });
        let y = list.y + row as u16 * 2;
        text(
            frame,
            Rect::new(list.x, y, list.width, 1),
            &format!("{marker} {name}{tag}"),
            if active { style.bold() } else { style },
        );
        if y + 1 < list.bottom() {
            rule(frame, Rect::new(list.x, y + 1, list.width, 1));
        }
    }
    let mut detail_rows = details(entry, catalog);
    if detail_rows.len() > inspector.height as usize {
        detail_rows = detail_rows
            .into_iter()
            .enumerate()
            .filter(|(index, text)| *index != 1 && !text.starts_with('─'))
            .map(|(_, text)| text)
            .collect();
    }
    for (row, value) in detail_rows
        .iter()
        .take(inspector.height as usize)
        .enumerate()
    {
        let style = if row == 0 {
            Style::default().fg(accent_color()).bold()
        } else if value.starts_with('─') {
            Style::default().fg(accent_color())
        } else {
            Style::default().fg(ai_text())
        };
        text(
            frame,
            Rect::new(inspector.x, inspector.y + row as u16, inspector.width, 1),
            value,
            style,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::{PickerAction, PickerKind, PickerOption};
    fn picker() -> InlineInteractiveState {
        let entries = ["gpt-6-astra", "test-small-model"]
            .into_iter()
            .map(|name| PickerEntry {
                name: name.into(),
                options: vec![PickerOption {
                    provider: "hcap".into(),
                    api_method: "api-key".into(),
                    available: true,
                    detail: String::new(),
                    estimated_reference_cost_micros: None,
                }],
                action: PickerAction::Model,
                selected_option: 0,
                is_current: name == "gpt-6-astra",
                is_default: name == "gpt-6-astra",
                is_favorite: false,
                recommended: false,
                recommendation_rank: 0,
                usage_score: 0,
                old: false,
                created_date: None,
                effort: None,
            })
            .collect();
        InlineInteractiveState {
            kind: PickerKind::Model,
            entries,
            filtered: vec![0, 1],
            selected: 0,
            column: 0,
            filter: String::new(),
            preview: true,
        }
    }
    fn catalog_fixture() -> Vec<ModelInfo> {
        serde_json::from_value(serde_json::json!([
            {"id":"gpt-6-astra", "context_length":1075200, "pricing":{"prompt":"0.0000004","completion":"0.000002"}, "extra":{"max_output":131072, "wallet":"premium", "tokens_per_second":null,"requests":12,"last_used":1789248087,"capabilities":{"streaming":true,"tools":true,"vision":true,"unjailed_variant":"gpt-6-astra-UnJailed"}}},
            {"id":"test-small-model", "context_length":8192, "extra":{"max_output":1024,"wallet":"standard","tokens_per_second":40,"capabilities":{"streaming":true,"tools":false,"vision":false}}}
        ])).unwrap()
    }
    fn rendered(width: u16, height: u16, picker: &InlineInteractiveState) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                models(
                    frame,
                    Rect::new(0, height.saturating_sub(5), width, 3),
                    picker,
                    &catalog_fixture(),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn model_inspector_tracks_highlight_and_preserves_catalog_stats() {
        let mut state = picker();
        let screen = rendered(120, 40, &state);
        for expected in [
            "╭",
            "MODEL LIBRARY",
            "1,075,200",
            "131,072",
            "$0.40",
            "$2.00",
            "premium",
            "not reported",
            "Stream ✓",
            "Tools ✓",
            "Vision ✓",
            "gpt-6-astra-UnJailed",
            "CATALOG ACTIVITY",
            "Last used",
        ] {
            assert!(screen.contains(expected), "missing {expected}:\n{screen}");
        }
        state.selected = 1;
        let screen = rendered(120, 40, &state);
        assert!(screen.contains("8,192"));
        assert!(screen.contains("standard"));
        assert!(screen.contains("Tools ×"));
        assert!(!screen.contains("1,075,200"));
        state.entries[1].options[0].provider = "other-gateway".into();
        assert!(rendered(120, 40, &state).contains("Catalog details unavailable"));
    }
    #[test]
    fn inspector_adapts_to_small_terminals_and_empty_results() {
        let mut state = picker();
        for (width, height) in [(140, 40), (64, 24), (52, 24), (30, 15), (8, 8)] {
            rendered(width, height, &state);
        }
        assert!(rendered(52, 24, &state).contains("Last used"));
        state.filtered.clear();
        assert!(rendered(80, 24, &state).contains("No models match"));
    }
    #[test]
    fn command_dropdown_has_border_separators_and_scrolls_to_selection() {
        let suggestions = (0..20)
            .map(|i| (format!("/command-{i}"), "Command description"))
            .collect::<Vec<_>>();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| commands(frame, Rect::new(0, 19, 80, 3), &suggestions, 19))
            .unwrap();
        let b = terminal.backend().buffer();
        let screen = (0..24)
            .map(|y| (0..80).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("COMMANDS"));
        assert!(screen.contains("› /command-19"));
        assert!(screen.contains("│ Command description"));
        assert!(!screen.contains("/command-0"));
    }
}
