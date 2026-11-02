use super::*;
use crossterm::event::{Event, MouseEvent, MouseEventKind};

impl App {
    /// Dispatch a crossterm event (keyboard, resize, mouse, etc.) to the active view.
    pub fn handle_crossterm_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                // Inspector text editors own literal Unicode. Navigation mode
                // still resolves macOS Option characters into global chords.
                let inspector_text_input = matches!(self.current_view, ViewId::PlanInspector(_))
                    && self
                        .plan_inspector
                        .as_ref()
                        .is_some_and(PlanInspectorView::text_input_active);
                let key = if inspector_text_input {
                    key
                } else {
                    crate::views::normalize_macos_option(key)
                };

                if self.record_panic_esc(key) {
                    return;
                }

                // Quit-confirm dialog takes priority: it captures every key.
                if self.quit_confirm_visible {
                    if is_quit_chord(key) {
                        self.confirm_quit();
                    } else {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                self.confirm_quit();
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                self.quit_confirm_visible = false;
                            }
                            _ => {}
                        }
                    }
                    self.dirty = true;
                    return;
                }

                if self.collision_modal.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            self.collision_modal = None;
                        }
                        KeyCode::Char('N') | KeyCode::Char('n') => {
                            self.collision_modal = None;
                            self.process_action(Action::NewSessionRequested);
                        }
                        KeyCode::Char('P') | KeyCode::Char('p') => {
                            self.collision_modal = None;
                            self.process_action(Action::RequestSessions);
                        }
                        KeyCode::Enter => {
                            let acp = self
                                .collision_modal
                                .as_ref()
                                .map(|state| state.acp_id.clone());
                            self.collision_modal = None;
                            if let (Some(session_id), Some(tx)) = (acp, self.user_input_tx.as_ref())
                            {
                                let _ = tx.try_send(UserInput::ResumeSession { session_id });
                            }
                        }
                        _ => {}
                    }
                    self.dirty = true;
                    return;
                }

                // Ctrl+C / Ctrl+Q are the global quit chords. They run BEFORE
                // the upgrade-modal handler so the modal's `_ => swallow` arm
                // never eats a quit chord. First press opens the confirmation
                // prompt; pressing it again while the prompt is visible
                // bypasses confirmation and exits immediately.
                if is_quit_chord(key) {
                    self.request_quit();
                    return;
                }

                // Plan C Tier 2 — upgrade modal sits between Quit/Collision and
                // Help in the priority chain: a denial CTA demands user
                // attention so it preempts informational overlays, but defers
                // to Quit/Collision (which are already-in-progress user-driven
                // flows the modal would otherwise interrupt).
                if self.upgrade_modal.is_some() {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            self.upgrade_modal = None;
                        }
                        KeyCode::Char('s') => {
                            self.upgrade_modal = None;
                            self.show_user_warning(
                                "Run `spur auth status` in a shell to view tiers and license state."
                                    .into(),
                            );
                        }
                        KeyCode::Char('l') => {
                            self.upgrade_modal = None;
                            self.show_user_warning(
                                "Run `spur auth login --key <KEY>` in a shell to activate a license."
                                    .into(),
                            );
                        }
                        _ => { /* swallow other keys while the modal is up */ }
                    }
                    self.dirty = true;
                    return;
                }

                // Help overlay intercepts ? (toggle) and Esc (close) before views.
                if self.help_visible {
                    if self.is_undo_key(key) {
                        self.flash_hint_short("close help to undo");
                        return;
                    }
                    match key.code {
                        KeyCode::Char('?') | KeyCode::Esc => {
                            self.help_visible = false;
                            return;
                        }
                        _ => return, // swallow all keys while help is visible
                    }
                }

                // Priority 2.5 — palette overlay.
                if self.palette_visible {
                    match self.palette_state.handle_key(key) {
                        Some(PaletteIntent::Dismiss) => {
                            self.palette_visible = false;
                            self.palette_state.reset();
                            self.dirty = true;
                        }
                        Some(PaletteIntent::Accept(result)) => {
                            self.palette_visible = false;
                            self.palette_state.reset();
                            if let Some(action) = self.result_to_action(result) {
                                self.process_action(action);
                            }
                            self.dirty = true;
                        }
                        Some(PaletteIntent::SubmitQuery(query)) => {
                            self.palette_visible = false;
                            self.palette_state.reset();
                            if let Some(action) = self.query_to_action(&query) {
                                self.process_action(action);
                            }
                            self.dirty = true;
                        }
                        None => {
                            self.dirty = true;
                        }
                    }
                    return;
                }

                if !self.dashboard_tab_empty_deprecation_shown
                    && self.current_view == ViewId::Dashboard
                    && key.code == KeyCode::Tab
                    && key.modifiers.is_empty()
                    && self.dashboard.is_empty_root_input()
                {
                    self.flash_hint_short(DASHBOARD_TAB_DEPRECATION_HINT);
                    self.dashboard_tab_empty_deprecation_shown = true;
                }

                // Global Ctrl+K opens palette. Plain `:` is a Dashboard Navigate
                // alias only, so Compose mode can still type the character.
                // NOTE: This global Ctrl+K intercept shadows InputBar::handle_emacs_input's
                // Ctrl+K kill-to-EOL (input_bar.rs:581). That Emacs binding stays in place
                // because (a) it is part of InputBar's component contract — tests/input_bar_editing.rs
                // ::ctrl_k_deletes_to_end_of_line covers it directly — and (b) any future
                // InputBar consumer not sitting behind this dispatcher will need it. The
                // shadowing is intentional, not dead code.
                let is_ctrl_k = key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('k'));
                let is_dashboard_colon_alias = key.code == KeyCode::Char(':')
                    && key.modifiers.is_empty()
                    && self.current_view == ViewId::Dashboard
                    && self.dashboard.mode() == DashboardMode::Navigate;
                if is_ctrl_k || is_dashboard_colon_alias {
                    self.open_palette();
                    return;
                }

                if key.modifiers.contains(KeyModifiers::ALT)
                    && matches!(key.code, KeyCode::Char('a'))
                {
                    self.process_action(Action::OpenInsights);
                    return;
                }

                if key.modifiers.contains(KeyModifiers::ALT)
                    && matches!(key.code, KeyCode::Char('g'))
                {
                    tracing::info!(
                        current_view = ?self.current_view,
                        has_session_detail = self.session_detail.is_some(),
                        "Alt+g pressed"
                    );
                    self.process_action(Action::InspectWorkers);
                    return;
                }

                // === All overlay/modal/help/global-shortcut owners run above this line. ===
                // === Tombstone undo is the residual key-owner: fires only when no       ===
                // === narrower visible context wants u/Ctrl+Z.                            ===
                if self.is_undo_key(key) && self.handle_undo() {
                    self.dirty = true;
                    return;
                }

                let ctx = crate::views::ViewContext {
                    lineage: &self.lineage,
                    plan_projection: &self.plan_projection,
                    synopsis: &self.synopsis,
                    brain_status: &self.brain_status,
                    flag_summary: self.flag_summary,
                    tombstone: None,
                    transient_hint_override: None,
                    notebook_ready: self.notebook_socket_nonce.is_some(),
                    theme: &self.theme,
                };
                let action = match self.current_view {
                    ViewId::Dashboard => self.dashboard.handle_key_with_worker_streams(
                        key,
                        &self.lineage,
                        &mut self.worker_streams,
                    ),
                    ViewId::SessionDetail(_) => {
                        if let Some(ref mut detail) = self.session_detail {
                            detail.handle_key(key, &ctx)
                        } else {
                            None
                        }
                    }
                    ViewId::SessionPicker => self
                        .session_picker
                        .as_mut()
                        .and_then(|p| p.handle_key(key, &ctx)),
                    ViewId::AgentConfigBrowser { .. } => self
                        .agent_config_browser
                        .as_mut()
                        .and_then(|view| view.handle_key(key, &ctx)),
                    ViewId::PlanInspector(_) => {
                        if let Some(view) = self.plan_inspector.as_mut() {
                            view.handle_key_with_worker_streams(key, &mut self.worker_streams, &ctx)
                        } else {
                            None
                        }
                    }
                    ViewId::PlanBrowser => self
                        .plan_browser
                        .as_mut()
                        .and_then(|view| view.handle_key(key, &ctx)),
                    ViewId::LoopBrowser => self
                        .loop_browser
                        .as_mut()
                        .and_then(|view| view.handle_key(key, &ctx)),
                    ViewId::ExploreBrowser => self
                        .explore_browser
                        .as_mut()
                        .and_then(|view| view.handle_key(key)),
                    ViewId::IssueBrowser => self
                        .issue_browser
                        .as_mut()
                        .and_then(|view| view.handle_key(key, &ctx)),
                    #[cfg(feature = "analytics")]
                    ViewId::Insights => {
                        if let Some(view) = self.insights_view.as_mut() {
                            view.handle_key(key, &ctx)
                        } else if self.insights_init.is_some() {
                            // Init still running. Allow Esc to bail back
                            // to Dashboard; the background task continues
                            // and its result lands on the next tick.
                            match key.code {
                                KeyCode::Esc => Some(Action::NavigateBack),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    }
                    #[cfg(not(feature = "analytics"))]
                    ViewId::Insights => None,
                    #[cfg(feature = "markdown")]
                    ViewId::MermaidOverlay(_) => {
                        if let Some(viewer) = self.mermaid_viewer.as_mut() {
                            match key.code {
                                KeyCode::Char('[') | KeyCode::Char(']') => {
                                    if let Some(detail) = self.session_detail.as_ref() {
                                        let entries: Vec<_> = detail
                                            .mermaid_registry
                                            .iter()
                                            .map(|(k, v)| (*k, v))
                                            .collect();
                                        viewer.cycle(&entries, key.code == KeyCode::Char(']'));
                                        self.dirty = true;
                                    }
                                    None
                                }
                                _ => viewer.handle_key(key, &ctx),
                            }
                        } else {
                            None
                        }
                    }
                };
                let should_dismiss_warning = matches!(key.code, KeyCode::Esc)
                    && self.user_warning.is_some()
                    // SessionPicker treats Esc as exit-to-Dashboard, not NavigateBack.
                    && matches!(
                        action,
                        Some(Action::NavigateBack)
                            | Some(Action::NavigateTo(ViewId::Dashboard))
                    );

                if should_dismiss_warning {
                    self.dismiss_user_warning();
                } else if let Some(action) = action {
                    self.process_action(action);
                }
                self.schedule_pending_agent_model_catalog_probes();
                self.dirty = true;
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
            }
            Event::Resize(_, _) => {
                #[cfg(feature = "markdown")]
                if let Some(detail) = self.session_detail.as_mut() {
                    detail.invalidate_inline_protocols();
                }
                self.dirty = true;
            }
            Event::Paste(text) => {
                if self.quit_confirm_visible
                    || self.collision_modal.is_some()
                    || self.upgrade_modal.is_some()
                    || self.help_visible
                    || self.palette_visible
                {
                    return;
                }

                // Normalize line endings once at the event boundary so every
                // view (dashboard, session_detail, session_picker) sees `\n`
                // separators regardless of clipboard origin.
                let normalized;
                let text: &str = if text.contains('\r') {
                    normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                    &normalized
                } else {
                    &text
                };

                match self.current_view {
                    ViewId::Dashboard => self.dashboard.handle_paste(text),
                    ViewId::SessionDetail(_) => {
                        if let Some(ref mut detail) = self.session_detail {
                            detail.handle_paste(text);
                        }
                    }
                    ViewId::SessionPicker => {
                        if let Some(ref mut picker) = self.session_picker {
                            picker.handle_paste(text);
                        }
                    }
                    ViewId::AgentConfigBrowser { .. } => {}
                    ViewId::PlanInspector(_) => {}
                    ViewId::PlanBrowser => {}
                    ViewId::LoopBrowser => {}
                    ViewId::ExploreBrowser => {}
                    ViewId::IssueBrowser => {}
                    ViewId::Insights => {}
                    #[cfg(feature = "markdown")]
                    ViewId::MermaidOverlay(_) => {}
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn record_panic_esc(&mut self, key: KeyEvent) -> bool {
        if key.code != KeyCode::Esc {
            return false;
        }

        let now = Instant::now();
        self.esc_chain.push_back(now);
        while self
            .esc_chain
            .front()
            .is_some_and(|instant| now.duration_since(*instant) > PANIC_RESET_ESC_WINDOW)
        {
            self.esc_chain.pop_front();
        }
        while self.esc_chain.len() > 3 {
            self.esc_chain.pop_front();
        }

        if self.esc_chain.len() == 3 {
            self.process_action(Action::PanicReset);
            return true;
        }

        false
    }

    /// `u` is the view-level undo key. Ctrl+Z is only claimed in Emacs mode;
    /// Vim users keep Ctrl+Z available to their terminal conventions.
    fn is_undo_key(&self, key: KeyEvent) -> bool {
        let bare_u = key.code == KeyCode::Char('u') && key.modifiers.is_empty();
        let emacs_ctrl_z = key.code == KeyCode::Char('z')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !matches!(self.edit_mode, EditMode::Vim(_));
        bare_u || emacs_ctrl_z
    }

    /// Undo handler for `u` and Emacs Ctrl+Z.
    ///
    /// Returns `true` when the app consumed or explicitly blocked the key.
    /// Returns `false` when a narrower owner, such as the composer or picker,
    /// should receive the key unchanged.
    pub(super) fn handle_undo(&mut self) -> bool {
        if self.input_bar_active_non_empty() {
            return false;
        }
        if self.picker_or_history_active() {
            return false;
        }
        if self.view_text_input_active() {
            return false;
        }
        if self.pending_permission.is_some() {
            return false;
        }
        if self.mermaid_render_picker_active() {
            return false;
        }

        let view = self.current_view.clone();
        let Some(tombstone) = self.tombstones.evict(&view) else {
            self.flash_hint_short("nothing to undo");
            return true;
        };

        match tombstone.kind {
            TombstoneKind::Reversible { inverse } => {
                self.flash_hint_short(format!("Undid: {}", tombstone.label));
                self.tombstone_undo_replay = true;
                self.process_action(inverse);
                self.tombstone_undo_replay = false;
            }
            TombstoneKind::QueuedRemote { pending: _ } => {
                self.flash_hint_short(format!("Cancelled: {}", tombstone.label));
            }
        }
        true
    }

    fn input_bar_active_non_empty(&self) -> bool {
        match &self.current_view {
            ViewId::Dashboard => self.dashboard.input_bar_active_non_empty(),
            ViewId::SessionDetail(_) => self
                .session_detail
                .as_ref()
                .is_some_and(SessionDetailView::input_bar_active_non_empty),
            _ => false,
        }
    }

    fn picker_or_history_active(&self) -> bool {
        match &self.current_view {
            ViewId::Dashboard => self.dashboard.completion_active(),
            ViewId::SessionDetail(_) => self
                .session_detail
                .as_ref()
                .is_some_and(SessionDetailView::completion_active),
            _ => false,
        }
    }

    fn view_text_input_active(&self) -> bool {
        match self.current_view {
            ViewId::PlanInspector(_) => self
                .plan_inspector
                .as_ref()
                .is_some_and(PlanInspectorView::modal_input_active),
            ViewId::SessionPicker => self.session_picker.as_ref().is_some_and(|picker| {
                picker.is_rename_active()
                    || picker.is_search_focused()
                    || picker.is_confirm_switch_visible()
            }),
            ViewId::AgentConfigBrowser { .. } => self
                .agent_config_browser
                .as_ref()
                .is_some_and(|view| view.editing_active()),
            ViewId::IssueBrowser => self
                .issue_browser
                .as_ref()
                .is_some_and(|v| v.is_filter_mode() || v.is_text_modal_active()),
            _ => false,
        }
    }

    fn mermaid_render_picker_active(&self) -> bool {
        #[cfg(feature = "markdown")]
        {
            matches!(self.current_view, ViewId::MermaidOverlay(_)) && self.mermaid_viewer.is_some()
        }
        #[cfg(not(feature = "markdown"))]
        {
            false
        }
    }

    pub(super) fn request_quit(&mut self) {
        self.quit_confirm_visible = true;
        self.dirty = true;
    }

    pub(super) fn confirm_quit(&mut self) {
        // Flush any unsent draft to disk before we exit so the next
        // `spur watch` restores the latest text.
        self.force_flush_active_draft();
        self.quit_confirm_visible = false;
        self.should_quit = true;
        self.dirty = true;
    }

    /// Handle mouse scroll events. Only scroll wheel is processed —
    /// clicks and drags are ignored to avoid tmux/terminal conflicts.
    fn handle_mouse_event(&mut self, event: MouseEvent) {
        let lines: usize = match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => 3,
            _ => return,
        };
        let is_up = matches!(event.kind, MouseEventKind::ScrollUp);

        match self.current_view {
            ViewId::Dashboard => {
                if self.dashboard.focused_node().is_some() {
                    if is_up {
                        self.dashboard.scroll_detail_up_by(lines);
                    } else {
                        self.dashboard.scroll_detail_down_by(lines);
                    }
                } else if is_up {
                    self.dashboard.scroll_activity_up_by(lines);
                } else {
                    self.dashboard.scroll_activity_down_by(lines);
                }
            }
            ViewId::SessionDetail(_) => {
                if let Some(ref mut detail) = self.session_detail {
                    if is_up {
                        detail.scroll_up_by(lines);
                    } else {
                        detail.scroll_down_by(lines);
                    }
                }
            }
            ViewId::SessionPicker => {
                // No mouse scroll in v1 picker.
            }
            ViewId::AgentConfigBrowser { .. } => {}
            ViewId::PlanInspector(_) => {}
            ViewId::PlanBrowser => {}
            ViewId::LoopBrowser => {}
            ViewId::ExploreBrowser => {}
            ViewId::IssueBrowser => {
                if let Some(ref mut browser) = self.issue_browser {
                    if browser.issue_detail_visible() {
                        if is_up {
                            browser.scroll_issue_detail_up_by(lines as u16);
                        } else {
                            browser.scroll_issue_detail_down_by(lines as u16);
                        }
                    } else {
                        let count = browser.tracked_issues().len();
                        if count > 0 {
                            if is_up {
                                browser.issues_panel_mut().select_prev(lines, count);
                            } else {
                                browser.issues_panel_mut().select_next(lines, count);
                            }
                        }
                    }
                }
            }
            ViewId::Insights => {}
            #[cfg(feature = "markdown")]
            ViewId::MermaidOverlay(_) => {}
        }
        self.dirty = true;
    }
}
