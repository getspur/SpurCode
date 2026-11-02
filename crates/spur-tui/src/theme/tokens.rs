use super::palette::{Palette, PaletteEntry};
use std::collections::HashMap;

const DARK_DEFAULT_BINDINGS: &[(&str, &str)] = &[
    ("status_bar.fg", "fg"),
    ("spinner.fg", "accent"),
    ("picker.selected.bg", "bg_selection"),
    ("picker.row.fg", "fg"),
    ("picker.hint.fg", "fg_subtle"),
    ("picker.match.fg", "highlight"),
    ("status_bar.tombstone.fg", "fg_subtle"),
    ("status_bar.issue_count.fg", "accent"),
    ("status_bar.separator.fg", "fg_subtle"),
    ("tool.family.thinking", "fg_subtle"),
    ("tool.family.edit", "warning"),
    ("tool.family.read", "accent"),
    ("tool.family.delete", "danger"),
    ("tool.family.move", "warning"),
    ("tool.family.search", "info"),
    ("tool.family.bash", "accent_alt"),
    ("tool.family.fetch", "info"),
    ("tool.family.switch_mode", "accent"),
    ("tool.family.task", "accent"),
    ("tool.family.mcp", "fg_subtle"),
    ("tool.family.unknown", "warning"),
    ("diff.add.fg", "diff_add"),
    ("diff.del.fg", "diff_del"),
    ("diff.context.fg", "fg_muted"),
    // status_bar metric & badge tokens (PR3 wave-1 migration).
    ("status_bar.review_pending.fg", "warning"),
    ("status_bar.cost.fg", "warning"),
    ("status_bar.mode.fg", "accent_alt"),
    ("status_bar.brand.fg", "accent"),
    ("status_bar.notebook.fg", "success"),
    ("status_bar.alert_critical.fg", "danger"),
    ("status_bar.alert_warning.fg", "warning"),
    ("status_bar.flag_on.fg", "success"),
    ("status_bar.flag_partial.fg", "warning"),
    ("status_bar.flag_off.fg", "danger"),
    ("status_bar.usage.fg", "info"),
    ("status_bar.analytics.fg", "info"),
    ("status_bar.effort.fg", "accent_alt"),
    // ReAct trace tokens (PR3 wave-1 migration).
    ("react_trace.timestamp.fg", "fg_subtle"),
    ("react_trace.think.fg", "fg_subtle"),
    ("react_trace.message.title.fg", "accent"),
    ("react_trace.message.body.fg", "fg"),
    ("react_trace.user_message.fg", "fg"),
    ("react_trace.user_message.bg", "bg_selection"),
    ("react_trace.user_message.accent.fg", "accent_alt"),
    ("react_trace.permission.fg", "warning"),
    ("react_trace.spinner.fg", "warning"),
    ("react_trace.outcome.success.fg", "success"),
    ("react_trace.outcome.error.fg", "danger"),
    ("react_trace.outcome.pending.fg", "warning"),
    ("react_trace.outcome.unknown.fg", "warning"),
    ("react_trace.diff.context.fg", "fg_subtle"),
    ("react_trace.delegate.fg", "accent"),
    ("react_trace.observe.fg", "success"),
    ("react_trace.command.fg", "accent_alt"),
    ("react_trace.partial_card.fg", "accent_alt"),
    ("react_trace.partial_card.hint.fg", "fg_subtle"),
    ("react_trace.title.claude.fg", "accent_alt"),
    ("react_trace.title.codex.fg", "warning"),
    ("react_trace.title.kiro.fg", "accent"),
    ("react_trace.title.generic.fg", "fg_subtle"),
    // session_picker tokens (PR4 wave-2 migration).
    ("session_picker.title.fg", "accent"),
    ("session_picker.spinner.fg", "accent"),
    ("session_picker.banner.label.fg", "accent"),
    ("session_picker.banner.timestamp.fg", "fg_subtle"),
    ("session_picker.banner.action.fg", "success"),
    ("session_picker.banner.muted.fg", "fg_subtle"),
    ("session_picker.banner.error.fg", "danger"),
    ("session_picker.banner.error_id.fg", "warning"),
    ("session_picker.search.label.fg", "fg_subtle"),
    ("session_picker.search.active.fg", "accent"),
    ("session_picker.search.inactive.fg", "fg_muted"),
    ("session_picker.new_row.fg", "success"),
    ("session_picker.row.separator.fg", "fg_subtle"),
    ("session_picker.row.archived.fg", "fg_subtle"),
    ("session_picker.row.title_selected.fg", "fg"),
    ("session_picker.row.muted.fg", "fg_subtle"),
    ("session_picker.row.cursor.fg", "accent"),
    ("session_picker.row.pinned.fg", "warning"),
    ("session_picker.row.draft.fg", "highlight"),
    ("session_picker.row.selected.bg", "bg_selection"),
    ("session_picker.preview.value.fg", "fg"),
    ("session_picker.preview.label.fg", "fg_subtle"),
    ("session_picker.preview.border.fg", "fg_subtle"),
    ("session_picker.preview.draft.fg", "warning"),
    ("session_picker.preview.intent.fg", "fg_muted"),
    ("session_picker.preview.placeholder.fg", "fg_subtle"),
    ("session_picker.preview.footer.fg", "fg_subtle"),
    ("session_picker.prompt.confirm.fg", "warning"),
    ("session_picker.prompt.rename.fg", "accent"),
    ("session_picker.error.title.fg", "accent"),
    ("session_picker.error.message.fg", "danger"),
    ("session_picker.error.hint.fg", "fg_subtle"),
    ("session_picker.footer_hint.fg", "fg_subtle"),
    // session_detail tokens (PR4 wave-2 migration).
    ("session_detail.auth_banner.fg", "fg"),
    ("session_detail.auth_banner.bg", "danger"),
    ("session_detail.error_banner.fg", "fg"),
    ("session_detail.error_banner.bg", "danger"),
    ("session_detail.cancel_modal.fg", "fg"),
    ("session_detail.cancel_modal.bg", "fg_subtle"),
    ("session_detail.breadcrumb.fg", "fg_subtle"),
    ("session_detail.agent_name.fg", "accent"),
    ("session_detail.role.fg", "fg_subtle"),
    ("session_detail.elapsed.fg", "fg_subtle"),
    ("session_detail.cost.fg", "warning"),
    ("session_detail.unsafe_fs.fg", "fg_on_warning"),
    ("session_detail.unsafe_fs.bg", "warning"),
    // plan_browser tokens (PR4 wave-2 migration).
    ("plan_browser.border.fg", "border"),
    ("plan_browser.empty.fg", "fg_subtle"),
    ("plan_browser.row.selected.fg", "warning"),
    ("plan_browser.notice.error.fg", "danger"),
    ("plan_browser.notice.warning.fg", "warning"),
    ("plan_browser.field.label.fg", "fg_subtle"),
    ("plan_browser.action_line.fg", "accent"),
    ("plan_browser.confirm.title.fg", "warning"),
    ("plan_browser.confirm.border.fg", "warning"),
    ("plan_browser.confirm.primary_key.fg", "success"),
    ("plan_browser.confirm.cancel_key.fg", "danger"),
    // plan_inspector tokens (PR4 wave-2 migration).
    ("plan_inspector.title.fg", "accent"),
    ("plan_inspector.gauge.fill.fg", "success"),
    ("plan_inspector.gauge.track.fg", "fg_subtle"),
    ("plan_inspector.label.fg", "fg_muted"),
    ("plan_inspector.footer_hint.fg", "fg_muted"),
    ("plan_inspector.status.running.fg", "warning"),
    ("plan_inspector.status.success.fg", "success"),
    ("plan_inspector.status.failure.fg", "danger"),
    ("plan_inspector.status.cancelled.fg", "accent_alt"),
    ("plan_inspector.status.unknown.fg", "fg_muted"),
    // plan_inspector redesign tokens (2026-05-11).
    ("plan_inspector.board.selection.fg", "accent"),
    ("plan_inspector.board.stage.active.fg", "accent"),
    ("plan_inspector.board.stage.inactive.fg", "fg_subtle"),
    ("plan_inspector.status.dispatched.fg", "info"),
    ("plan_inspector.status.awaiting_review.fg", "warning"),
    ("plan_inspector.status.approved.fg", "success"),
    ("plan_inspector.status.rejected.fg", "warning"),
    ("plan_inspector.status.failed.fg", "danger"),
    ("plan_inspector.status.blocked.fg", "fg_on_danger"),
    ("plan_inspector.status.blocked.bg", "danger"),
    ("plan_inspector.status.pending.fg", "fg_muted"),
    ("plan_inspector.status.ready.fg", "fg_muted"),
    ("plan_inspector.chip.retry.fg", "accent_alt"),
    ("plan_inspector.chip.live.fg", "info"),
    ("plan_inspector.chip.blocked.fg", "danger"),
    ("plan_inspector.chip.depends.fg", "fg_muted"),
    ("plan_inspector.detail.hero.fg", "fg"),
    ("plan_inspector.detail.hero.agent.fg", "accent"),
    ("plan_inspector.detail.blocked_label.fg", "danger"),
    ("plan_inspector.detail.blocked_value.fg", "danger"),
    ("plan_inspector.detail.blocked_banner.fg", "fg_on_danger"),
    ("plan_inspector.detail.blocked_banner.bg", "danger"),
    ("plan_inspector.detail.section.fg", "fg_muted"),
    ("plan_inspector.detail.label.fg", "accent"),
    ("plan_inspector.detail.edge.structural.fg", "fg_muted"),
    ("plan_inspector.detail.edge.blocked.fg", "danger"),
    ("plan_inspector.detail.edge.highlight.fg", "accent"),
    ("plan_inspector.header.next_action.fg", "fg"),
    ("plan_inspector.header.count.dispatched.fg", "info"),
    ("plan_inspector.header.count.awaiting_review.fg", "warning"),
    ("plan_inspector.header.count.approved.fg", "success"),
    ("plan_inspector.header.count.rejected.fg", "warning"),
    ("plan_inspector.header.count.failed.fg", "danger"),
    ("plan_inspector.header.count.blocked.fg", "danger"),
    ("plan_inspector.header.count.cancelled.fg", "accent_alt"),
    ("plan_inspector.header.count.pending.fg", "fg_muted"),
    ("plan_inspector.header.count.ready.fg", "success"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenMap(pub HashMap<String, String>);

impl TokenMap {
    pub fn dark_default() -> TokenMap {
        TokenMap::from_bindings(DARK_DEFAULT_BINDINGS)
    }

    pub fn light_default() -> TokenMap {
        Self::dark_default()
    }

    pub fn high_contrast_default() -> TokenMap {
        Self::dark_default()
    }

    pub fn resolve<'a>(&self, token: &str, palette: &'a Palette) -> Option<&'a PaletteEntry> {
        let palette_field = self.0.get(token)?;
        palette_entry(palette_field, palette)
    }

    fn from_bindings(bindings: &[(&str, &str)]) -> TokenMap {
        let mut token_map = HashMap::with_capacity(bindings.len());
        for &(token, palette_field) in bindings {
            token_map.insert(token.to_string(), palette_field.to_string());
        }
        TokenMap(token_map)
    }
}

fn palette_entry<'a>(field: &str, palette: &'a Palette) -> Option<&'a PaletteEntry> {
    match field {
        "bg" => Some(&palette.bg),
        "bg_panel" => Some(&palette.bg_panel),
        "bg_selection" => Some(&palette.bg_selection),
        "bg_overlay" => Some(&palette.bg_overlay),
        "fg" => Some(&palette.fg),
        "fg_muted" => Some(&palette.fg_muted),
        "fg_subtle" => Some(&palette.fg_subtle),
        "fg_on_accent" => Some(&palette.fg_on_accent),
        "fg_on_success" => Some(&palette.fg_on_success),
        "fg_on_warning" => Some(&palette.fg_on_warning),
        "fg_on_danger" => Some(&palette.fg_on_danger),
        "fg_on_info" => Some(&palette.fg_on_info),
        "fg_on_overlay" => Some(&palette.fg_on_overlay),
        "border" => Some(&palette.border),
        "border_focused" => Some(&palette.border_focused),
        "accent" => Some(&palette.accent),
        "accent_alt" => Some(&palette.accent_alt),
        "success" => Some(&palette.success),
        "warning" => Some(&palette.warning),
        "danger" => Some(&palette.danger),
        "info" => Some(&palette.info),
        "highlight" => Some(&palette.highlight),
        "diff_add" => Some(&palette.diff_add),
        "diff_del" => Some(&palette.diff_del),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::palette::Palette;
    use super::TokenMap;

    #[test]
    fn dark_default_contains_all_spec_sample_bindings() {
        assert_eq!(TokenMap::dark_default().0.len(), 164);
    }

    #[test]
    fn plan_inspector_redesign_tokens_are_bound() {
        let tokens = TokenMap::dark_default();
        let expected = [
            ("plan_inspector.board.selection.fg", "accent"),
            ("plan_inspector.board.stage.active.fg", "accent"),
            ("plan_inspector.board.stage.inactive.fg", "fg_subtle"),
            ("plan_inspector.status.dispatched.fg", "info"),
            ("plan_inspector.status.awaiting_review.fg", "warning"),
            ("plan_inspector.status.approved.fg", "success"),
            ("plan_inspector.status.rejected.fg", "warning"),
            ("plan_inspector.status.failed.fg", "danger"),
            ("plan_inspector.status.blocked.fg", "fg_on_danger"),
            ("plan_inspector.status.blocked.bg", "danger"),
            ("plan_inspector.status.cancelled.fg", "accent_alt"),
            ("plan_inspector.status.pending.fg", "fg_muted"),
            ("plan_inspector.status.ready.fg", "fg_muted"),
            ("plan_inspector.chip.retry.fg", "accent_alt"),
            ("plan_inspector.chip.live.fg", "info"),
            ("plan_inspector.chip.blocked.fg", "danger"),
            ("plan_inspector.chip.depends.fg", "fg_muted"),
            ("plan_inspector.detail.hero.fg", "fg"),
            ("plan_inspector.detail.hero.agent.fg", "accent"),
            ("plan_inspector.detail.blocked_label.fg", "danger"),
            ("plan_inspector.detail.blocked_value.fg", "danger"),
            ("plan_inspector.detail.blocked_banner.fg", "fg_on_danger"),
            ("plan_inspector.detail.blocked_banner.bg", "danger"),
            ("plan_inspector.detail.section.fg", "fg_muted"),
            ("plan_inspector.detail.label.fg", "accent"),
            ("plan_inspector.detail.edge.structural.fg", "fg_muted"),
            ("plan_inspector.detail.edge.blocked.fg", "danger"),
            ("plan_inspector.detail.edge.highlight.fg", "accent"),
            ("plan_inspector.header.next_action.fg", "fg"),
            ("plan_inspector.header.count.dispatched.fg", "info"),
            ("plan_inspector.header.count.awaiting_review.fg", "warning"),
            ("plan_inspector.header.count.approved.fg", "success"),
            ("plan_inspector.header.count.rejected.fg", "warning"),
            ("plan_inspector.header.count.failed.fg", "danger"),
            ("plan_inspector.header.count.blocked.fg", "danger"),
            ("plan_inspector.header.count.cancelled.fg", "accent_alt"),
            ("plan_inspector.header.count.pending.fg", "fg_muted"),
            ("plan_inspector.header.count.ready.fg", "success"),
        ];

        for (token, palette_role) in expected {
            assert_eq!(tokens.0.get(token).map(String::as_str), Some(palette_role));
        }
    }

    #[test]
    fn session_picker_review_tokens_are_bound() {
        let tokens = TokenMap::dark_default();
        let expected = [
            ("session_picker.preview.value.fg", "fg"),
            ("session_picker.preview.label.fg", "fg_subtle"),
            ("session_picker.preview.border.fg", "fg_subtle"),
            ("session_picker.row.draft.fg", "highlight"),
            ("session_picker.row.selected.bg", "bg_selection"),
        ];

        for (token, palette_role) in expected {
            assert_eq!(tokens.0.get(token).map(String::as_str), Some(palette_role));
        }
        assert_ne!(
            tokens.0.get("session_picker.row.draft.fg"),
            tokens.0.get("session_picker.row.pinned.fg")
        );
    }

    #[test]
    fn default_tokens_resolve_to_palette_entries() {
        let defaults = [
            (TokenMap::dark_default(), Palette::dark_default()),
            (TokenMap::light_default(), Palette::light_default()),
            (
                TokenMap::high_contrast_default(),
                Palette::high_contrast_default(),
            ),
        ];

        for (tokens, palette) in defaults {
            for token in tokens.0.keys() {
                assert!(tokens.resolve(token, &palette).is_some(), "{token}");
            }
        }
    }

    #[test]
    fn light_and_high_contrast_defaults_match_dark_bindings() {
        assert_eq!(TokenMap::light_default().0, TokenMap::dark_default().0);
        assert_eq!(
            TokenMap::high_contrast_default().0,
            TokenMap::dark_default().0
        );
    }

    #[test]
    fn unknown_token_returns_none() {
        let palette = Palette::dark_default();
        let tokens = TokenMap::dark_default();

        assert!(tokens.resolve("not.a.real.token", &palette).is_none());
    }

    #[test]
    fn migration_bindings_preserve_current_literal_colors() {
        let tokens = TokenMap::dark_default();

        assert_eq!(
            tokens.0.get("tool.family.edit"),
            Some(&"warning".to_string())
        );
        assert_eq!(
            tokens.0.get("tool.family.bash"),
            Some(&"accent_alt".to_string())
        );
        assert_eq!(
            tokens.0.get("status_bar.separator.fg"),
            Some(&"fg_subtle".to_string())
        );
        assert_eq!(
            tokens.0.get("picker.selected.bg"),
            Some(&"bg_selection".to_string())
        );
        assert_eq!(tokens.0.get("picker.row.fg"), Some(&"fg".to_string()));
        assert_eq!(
            tokens.0.get("picker.hint.fg"),
            Some(&"fg_subtle".to_string())
        );
    }
}
