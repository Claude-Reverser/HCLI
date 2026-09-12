//! HCLI workspace chrome. Keep geometry independent of terminal color support.

use super::TuiState;
use jcode_tui_style::theme::{accent_color, ai_text, dim_color, warning_color};
use ratatui::prelude::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// UI-only generation-rate sample. Estimates never enter usage/cost accounting.
#[derive(Debug, Clone, Default)]
pub(crate) struct SpeedMeter {
    completed_tokens: f64,
    completed_estimated: bool,
    chars: usize,
    chars_at_report: usize,
    reported: Option<u64>,
    elapsed: std::time::Duration,
}

impl SpeedMeter {
    fn count(&self) -> (f64, bool) {
        let pending = self.chars.saturating_sub(self.chars_at_report);
        (
            self.completed_tokens + self.reported.unwrap_or(0) as f64 + pending as f64 / 4.0,
            self.completed_estimated || pending > 0,
        )
    }

    pub(crate) fn output(&mut self, text: &str, elapsed: std::time::Duration) {
        if !text.is_empty() {
            self.chars = self.chars.saturating_add(text.chars().count());
            self.elapsed = elapsed;
        }
    }

    pub(crate) fn usage(&mut self, tokens: u64) {
        // Gateways sometimes send input-only usage with a zero output counter.
        // That is not evidence that already streamed text contained no tokens.
        if tokens == 0 {
            return;
        }
        if self.reported != Some(tokens) {
            self.reported = Some(tokens);
            self.chars_at_report = self.chars;
        }
    }

    pub(crate) fn next_call(&mut self) {
        let (tokens, estimated) = self.count();
        self.completed_tokens = tokens;
        self.completed_estimated = estimated;
        self.chars = 0;
        self.chars_at_report = 0;
        self.reported = None;
    }

    pub(crate) fn sample(&self) -> Option<(f32, bool)> {
        let (tokens, estimated) = self.count();
        let seconds = self.elapsed.as_secs_f64();
        // Avoid a misleading spike from one nearly instantaneous chunk.
        (tokens > 0.0 && seconds >= 0.25).then(|| ((tokens / seconds) as f32, estimated))
    }
}

pub(crate) fn enabled() -> bool {
    std::env::var("HCLI_UI").is_ok_and(|value| value == "1")
}

/// Defaults form a restrained slate/cyan palette. User overrides are applied
/// afterwards, and the existing light-theme / ANSI-color adapters still run.
pub(crate) const COLORS: &[(&str, &str)] = &[
    ("accent", "#62cbd4"),
    ("user", "#81b8e2"),
    ("ai", "#62cbd4"),
    ("tool", "#91a0ae"),
    ("file_link", "#81b8e2"),
    ("dim", "#8998a5"),
    ("system", "#a6b5c2"),
    ("pending", "#8998a5"),
    ("user_text", "#e3edf3"),
    ("user_bg", "#192730"),
    ("ai_text", "#dce5eb"),
    ("header_icon", "#62cbd4"),
    ("header_name", "#62cbd4"),
    ("header_session", "#dce5eb"),
    ("success", "#85c6a2"),
    ("warning", "#dfbc7b"),
    ("error", "#eb8a8a"),
    ("info", "#81b8e2"),
    ("border", "#33434d"),
    ("selection_bg", "#223540"),
];

pub(super) fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut remaining = width - 1;
    let mut result = String::new();
    for ch in text.chars() {
        let cells = ch.width().unwrap_or(0);
        if cells > remaining {
            break;
        }
        remaining -= cells;
        result.push(ch);
    }
    result.push('…');
    result
}

pub(crate) fn header(app: &dyn TuiState, width: u16) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    // Transcript rendering adds a small content inset after header preparation.
    let width = width.saturating_sub(2) as usize;
    let accent = Style::default().fg(accent_color());
    let muted = Style::default().fg(dim_color());
    let text = Style::default().fg(ai_text());
    let provider = app.provider_name();
    let model = app.provider_model();
    let connecting = model.is_empty()
        || model == "remote"
        || model == "connected"
        || model.starts_with("connecting");
    let route = if provider.eq_ignore_ascii_case("hcap") {
        "hcap.ai"
    } else {
        &provider
    };
    let status = if connecting {
        "connecting"
    } else if app.is_processing() {
        "working"
    } else {
        "ready"
    };
    let mut brand = vec![Span::styled(fit("HCLI", width), accent.bold())];
    if width >= 28 {
        brand.push(Span::styled(
            format!("  /  {}", fit(route, width.saturating_sub(24))),
            muted,
        ));
        brand.push(Span::styled(format!("  ·  {status}"), muted));
    }
    let mut lines = vec![
        Line::from(brand),
        Line::from(Span::styled("─".repeat(width.min(64)), muted)),
    ];
    let model = super::ui::header::header_model_display_name(&model, &provider);
    lines.push(Line::from(Span::styled(fit(&model, width), text.bold())));
    if let Some(path) = app.working_dir() {
        let path = dirs::home_dir()
            .and_then(|home| {
                std::path::Path::new(&path)
                    .strip_prefix(home)
                    .ok()
                    .map(|rest| {
                        if rest.as_os_str().is_empty() {
                            "~".to_owned()
                        } else {
                            format!("~/{}", rest.display())
                        }
                    })
            })
            .unwrap_or(path);
        let workspace = match app.git_branch() {
            Some(branch) if width >= 48 => format!("{path}  ·  {branch}"),
            _ => path,
        };
        lines.push(Line::from(Span::styled(fit(&workspace, width), muted)));
    }
    if app.server_update_available() == Some(true) || app.client_update_available() {
        lines.push(Line::from(Span::styled(
            fit("Update available · /update", width),
            muted,
        )));
    }
    if let Some(version) = app.server_display_version()
        && version.trim() != jcode_build_meta::version().trim()
    {
        lines.push(Line::from(Span::styled(
            fit("Server build differs · restart to apply changes", width),
            Style::default().fg(warning_color()),
        )));
    }
    lines.push(Line::from(""));
    (lines, Vec::new())
}

pub(crate) fn welcome(width: u16) -> Vec<Line<'static>> {
    let width = width as usize;
    let muted = Style::default().fg(dim_color());
    let accent = Style::default().fg(accent_color());
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            fit("What are we building?", width),
            Style::default().fg(ai_text()).bold(),
        )),
        Line::from(Span::styled(
            fit(
                "Describe a change, ask a question, or explore this project.",
                width,
            ),
            muted,
        )),
        Line::from(""),
    ];
    for (command, label) in [
        ("/model", "Choose a model"),
        ("/help", "Commands & shortcuts"),
    ] {
        if width >= 32 {
            lines.push(Line::from(vec![
                Span::styled(format!("{command:<10}"), accent),
                Span::styled(fit(label, width - 10), muted),
            ]));
        } else {
            lines.push(Line::from(Span::styled(fit(command, width), accent)));
        }
    }
    lines.push(Line::from(""));
    lines
}

fn tokens(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.3}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn join_metrics(parts: &[String], width: usize) -> String {
    let mut text = String::new();
    for part in parts {
        let candidate = if text.is_empty() {
            part.clone()
        } else {
            format!("{text}  ·  {part}")
        };
        if candidate.width() <= width {
            text = candidate;
        } else if text.is_empty() {
            return fit(part, width);
        }
    }
    text
}

fn footer_lines(
    data: &super::info_widget::InfoWidgetData,
    session: Option<(u64, u64)>,
    model: Option<&crate::provider::openrouter::ModelInfo>,
    width: usize,
    speed_estimated: bool,
) -> Vec<Line<'static>> {
    let limit = model
        .and_then(|model| model.context_length)
        .or(data.context_limit.map(|limit| limit as u64));
    let used = data
        .observed_context_tokens
        .map(|value| (value, false))
        .or_else(|| {
            if data.context_info_stale {
                None
            } else {
                data.context_info
                    .as_ref()
                    .map(|info| (info.estimated_tokens() as u64, true))
            }
        });
    let context = match (used, limit.filter(|limit| *limit > 0)) {
        (Some((used, estimated)), Some(limit)) => format!(
            "{}{} / {} ctx ({:.1}%)",
            if estimated { "~" } else { "" },
            tokens(used),
            tokens(limit),
            used as f64 * 100.0 / limit as f64
        ),
        (_, Some(limit)) => format!("— / {} ctx", tokens(limit)),
        _ => "Context —".to_string(),
    };
    let speed = data
        .tokens_per_second
        .filter(|speed| speed.is_finite() && *speed >= 0.0)
        .map(|speed| format!("{}{speed:.1} tok/s", if speed_estimated { "~" } else { "" }))
        .unwrap_or_else(|| "— tok/s".to_string());
    let mut live = vec![context, speed];
    if let Some(effort) = &data.reasoning_effort {
        live.push(format!("Reasoning {effort}"));
    }
    let price = |value: Option<&str>| {
        value
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| format!("${:.2}", value * 1_000_000.0))
            .unwrap_or_else(|| "—".to_string())
    };
    let mut details = Vec::new();
    if let Some(model) = model {
        details.push(format!(
            "{} in / {} out per 1M",
            price(model.pricing.prompt.as_deref()),
            price(model.pricing.completion.as_deref())
        ));
    }
    if let Some((input, output)) = session {
        details.push(format!(
            "Session {} in / {} out",
            tokens(input),
            tokens(output)
        ));
    }
    if details.is_empty() {
        details.push("Session tokens —".to_string());
    }
    vec![
        Line::styled(
            join_metrics(&live, width),
            Style::default().fg(accent_color()),
        ),
        Line::styled(
            join_metrics(&details, width),
            Style::default().fg(dim_color()),
        ),
    ]
}

pub(crate) fn draw_footer(
    frame: &mut Frame,
    app: &dyn TuiState,
    data: &super::info_widget::InfoWidgetData,
    area: Rect,
) {
    // Read the existing background-refreshed catalog, never perform network I/O
    // from a render frame. Scope rates to hcap's configured API, not another
    // provider selling the same model ID.
    let catalog = if app.provider_name().eq_ignore_ascii_case("hcap") {
        crate::provider::openrouter::load_disk_cache_entry_for_namespace("hcap").filter(|cache| {
            crate::config::config()
                .providers
                .get("hcap")
                .is_some_and(|profile| {
                    cache.source_api_base.as_deref().is_some_and(|source| {
                        source.trim_end_matches('/') == profile.base_url.trim_end_matches('/')
                    })
                })
        })
    } else {
        None
    };
    let id = app.provider_model();
    let model = catalog
        .as_ref()
        .and_then(|cache| cache.models.iter().find(|model| model.id == id));
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    frame.render_widget(
        ratatui::widgets::Paragraph::new(footer_lines(
            data,
            app.total_session_tokens(),
            model,
            inner.width as usize,
            app.output_tps_is_estimated(),
        )),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_estimates_stream_then_keeps_final_usage_rate() {
        use std::time::Duration;
        let mut meter = SpeedMeter::default();
        meter.output("abcd", Duration::ZERO);
        assert_eq!(meter.sample(), None);
        meter.output("efghijkl", Duration::from_secs(1));
        assert_eq!(meter.sample(), Some((3.0, true)));
        meter.usage(0);
        assert_eq!(meter.sample(), Some((3.0, true)));
        meter.usage(6);
        assert_eq!(meter.sample(), Some((6.0, false)));
        // Rendering later or receiving the same final usage must not decay speed.
        meter.usage(6);
        assert_eq!(meter.sample(), Some((6.0, false)));
        meter.next_call();
        meter.output("abcd", Duration::from_secs(2));
        assert_eq!(meter.sample(), Some((3.5, true)));
        meter.usage(4);
        assert_eq!(meter.sample(), Some((5.0, false)));
        meter = SpeedMeter::default();
        assert_eq!(meter.sample(), None);
    }

    #[test]
    fn speed_keeps_unreported_calls_estimated_and_ignores_duplicate_usage() {
        use std::time::Duration;
        let mut meter = SpeedMeter::default();
        meter.output("abcd", Duration::from_secs(1));
        meter.next_call();
        meter.output("abcd", Duration::from_secs(2));
        meter.usage(10);
        assert_eq!(meter.sample(), Some((5.5, true)));
        meter.output("abcd", Duration::from_secs(3));
        meter.usage(10);
        assert_eq!(meter.sample(), Some((4.0, true)));
    }

    #[test]
    fn footer_shows_catalog_context_prices_and_measured_speed() {
        let mut data = super::super::info_widget::InfoWidgetData::default();
        data.observed_context_tokens = Some(10_752);
        data.tokens_per_second = Some(42.5);
        let model = crate::provider::openrouter::ModelInfo {
            extra: None,
            id: "gpt-6-astra".into(),
            name: String::new(),
            context_length: Some(1_075_200),
            created: None,
            pricing: crate::provider::openrouter::ModelPricing {
                prompt: Some("0.0000004".into()),
                completion: Some("0.000002".into()),
                ..Default::default()
            },
        };
        let lines = footer_lines(&data, Some((1200, 500)), Some(&model), 140, false);
        let text = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("1.075M ctx (1.0%)"), "{text}");
        assert!(text.contains("42.5 tok/s"), "{text}");
        assert!(text.contains("$0.40 in / $2.00 out per 1M"), "{text}");
        assert!(text.contains("Session 1.2k in / 500 out"), "{text}");
        assert!(
            footer_lines(&data, None, Some(&model), 64, true)[0]
                .to_string()
                .contains("~42.5 tok/s")
        );
        data.tokens_per_second = None;
        assert!(
            footer_lines(&data, None, Some(&model), 64, false)[0]
                .to_string()
                .contains("— tok/s")
        );
        for width in 0..150 {
            assert!(
                footer_lines(&data, Some((1200, 500)), Some(&model), width, false)
                    .iter()
                    .all(|line| line.width() <= width)
            );
        }
    }

    #[test]
    fn chrome_fits_narrow_terminals_and_wide_unicode() {
        for width in 0..80 {
            assert!(fit("~/项目/very-long-workspace", width).width() <= width);
            for line in welcome(width as u16) {
                assert!(line.width() <= width);
            }
        }
        assert_eq!(fit("Hello", 5), "Hello");
    }
}
