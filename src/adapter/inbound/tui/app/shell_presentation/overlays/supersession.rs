use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::super::super::NativeTuiApp;
use super::SupersessionOverlayView;

pub(crate) fn build_supersession_overlay_view(app: &NativeTuiApp) -> SupersessionOverlayView {
    let mode_label = if app.parallel_mode_enabled() {
        "parallel"
    } else {
        "normal"
    };
    let snapshot = app.parallel_mode_readiness_snapshot();
    let summary_lines = match snapshot {
        Some(snapshot) => {
            let mut lines = vec![
                Line::from(format!("mode: {mode_label}")),
                Line::from(format!("readiness: {}", snapshot.readiness_label())),
                Line::from(format!("workspace: {}", snapshot.workspace_path)),
            ];
            if let Some(alert) = snapshot.top_alert.as_deref() {
                lines.push(Line::from(format!("alert: {alert}")));
            }
            lines
        }
        None => vec![
            Line::from(format!("mode: {mode_label}")),
            Line::from("readiness: not checked yet"),
            Line::from("next action: run :parallel or :parallel on"),
        ],
    };
    let capability_lines = snapshot
        .map(|snapshot| {
            snapshot
                .capabilities
                .iter()
                .map(|capability| Line::from(capability.summary()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![Line::from("parallel readiness has not been inspected yet")]);
    let board_lines = vec![
        Line::from("control tower skeleton"),
        Line::from("capability panel: live"),
        Line::from("pool board: not reconciled yet"),
        Line::from("active agents: no agent sessions launched in this slice"),
        Line::from("merge queue: distributor not started in this slice"),
        Line::from("selected detail: placeholder until supervisor state lands"),
    ];
    let key_lines = vec![
        Line::from("r: rerun readiness    Ctrl+P: parallel off"),
        Line::from("Ctrl+O or Esc/Ctrl+C: close"),
    ];

    SupersessionOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Supersession Control Tower",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / prepare state"),
            ]),
            Line::from("Inspect readiness before any parallel agent is launched."),
        ],
        summary_lines,
        capability_lines,
        board_lines,
        key_lines,
    }
}
