use ratatui::{backend::TestBackend, style::Color, widgets::Block, Terminal};
use spur_acp::SessionId;
use spur_core::{ExecutorLineage, PlanProjectionStore, SessionSynopsisProjection};
use spur_tui::{
    app::BrainStatus,
    theme::{load_built_in, load_theme_from_str, resolve_token, ColorDepth},
    views::{plan_inspector::PlanInspectorView, View, ViewContext},
};

fn luminance(color: Color) -> f64 {
    let Color::Rgb(r, g, b) = color else {
        panic!("expected truecolor palette value")
    };
    let linear = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn contrast(foreground: Color, background: Color) -> f64 {
    let f = luminance(foreground);
    let b = luminance(background);
    (f.max(b) + 0.05) / (f.min(b) + 0.05)
}

#[test]
fn essential_inspector_text_meets_builtin_contrast_budget() {
    for name in ["dark", "light", "high-contrast"] {
        let theme = load_built_in(name).unwrap();
        for token in [
            "plan_inspector.label.fg",
            "plan_inspector.footer_hint.fg",
            "plan_inspector.status.unknown.fg",
            "plan_inspector.status.pending.fg",
            "plan_inspector.chip.depends.fg",
            "plan_inspector.detail.section.fg",
            "plan_inspector.detail.edge.structural.fg",
            "plan_inspector.header.count.pending.fg",
        ] {
            let foreground = resolve_token(&theme, token, ColorDepth::Truecolor);
            for (surface, background) in [
                ("bg", theme.palette.bg.rgb),
                ("bg_panel", theme.palette.bg_panel.rgb),
            ] {
                let ratio = contrast(foreground, background);
                println!(
                    "U5_CONTRAST {}",
                    serde_json::json!({"theme":name,"token":token,"surface":surface,"foreground_luminance":(luminance(foreground)*100_000.0).round() as u64,"background_luminance":(luminance(background)*100_000.0).round() as u64,"ratio":ratio})
                );
                assert!(
                    ratio >= 4.5,
                    "{name} {token} is too faint on {surface}: {ratio:.3}:1"
                );
            }
        }
        assert_eq!(
            resolve_token(
                &theme,
                "plan_inspector.board.stage.inactive.fg",
                ColorDepth::Truecolor
            ),
            theme.palette.fg_subtle.rgb,
            "decorative borders should retain their subtle role"
        );
    }
}

#[test]
fn footer_rendering_honors_the_resolved_theme_and_custom_override() {
    let custom = load_theme_from_str(
        "version: 1\nname: custom\nextends: dark\ntokens:\n  plan_inspector.footer_hint.fg: fg\n",
        |name| {
            (name == "dark")
                .then(|| serde_yml::from_str(include_str!("../themes/dark.yaml")).unwrap())
        },
    )
    .unwrap();
    for theme in [
        load_built_in("dark").unwrap(),
        load_built_in("light").unwrap(),
        custom,
    ] {
        let lineage = ExecutorLineage::new();
        let plans = PlanProjectionStore::default();
        let synopsis = SessionSynopsisProjection::new();
        let ctx = ViewContext {
            lineage: &lineage,
            plan_projection: &plans,
            synopsis: &synopsis,
            brain_status: &BrainStatus::Idle,
            flag_summary: None,
            tombstone: None,
            transient_hint_override: None,
            notebook_ready: false,
            theme: &theme,
        };
        let mut view = PlanInspectorView::new(SessionId("contrast-brain".into()));
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Block::default()
                        .style(ratatui::style::Style::default().bg(theme.palette.bg.rgb)),
                    frame.area(),
                );
                view.render(frame, frame.area(), &ctx);
            })
            .unwrap();
        let cell = &terminal.backend().buffer()[(0, 7)];
        assert_eq!(cell.symbol(), "E");
        assert_eq!(
            cell.fg,
            resolve_token(
                &theme,
                "plan_inspector.footer_hint.fg",
                ColorDepth::Truecolor
            )
        );
        assert!(
            contrast(cell.fg, cell.bg) >= 4.5,
            "rendered footer is too faint in {}",
            theme.name
        );
        if theme.name == "custom" {
            assert_eq!(cell.fg, theme.palette.fg.rgb);
        }
    }
}
