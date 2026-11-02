#[cfg(test)]
mod license_gate_refresh_tests {
    use super::super::super::*;

    fn pro_license_state_event() -> LicenseStateEvent {
        LicenseStateEvent {
            status: LicenseStatusEvent::Active,
            subject_kind: LicenseSubjectKind::User,
            plan: EventLicensePlan::Pro,
            features: spur_license::policy::PolicyResolver::embedded()
                .tier_features("pro")
                .expect("embedded policy must define pro tier features"),
            expires_at: None,
            binding_mode: LicenseBindingMode::NodeLocked,
            offline_ok: true,
            status_text: "Pro license active".into(),
        }
    }

    fn assert_pro_cost_tracking_enabled(app: &App) {
        spur_license::require_feature(
            &app.feature_gate,
            spur_license::FeatureKey::COST_PRO_PER_PROJECT_TRACKING,
        )
        .expect("Pro cost tracking should be enabled");
    }

    #[test]
    fn license_update_refreshes_feature_gate_snapshot() {
        let mut app = App::new_for_tests();
        assert!(spur_license::require_feature(
            &app.feature_gate,
            spur_license::FeatureKey::COST_PRO_PER_PROJECT_TRACKING,
        )
        .is_err());

        app.handle_spur_event(SpurEvent::now(SpurEventBody::LicenseUpdated {
            state: pro_license_state_event(),
        }));

        assert_pro_cost_tracking_enabled(&app);
    }

    #[test]
    fn seeded_license_state_hydrates_feature_gate_snapshot() {
        let app = App::new_with_license(
            None,
            false,
            std::sync::Arc::new(spur_acp::SpurConfig::default()),
            pro_license_state_event(),
            crate::landing::LandingDecision::ShowDashboard,
            None,
        );

        assert_pro_cost_tracking_enabled(&app);
    }
}

#[cfg(all(test, feature = "analytics"))]
mod insights_navigation_tests {
    use super::super::super::*;

    /// `InsightsView::new` spawns a tokio refresh task, so these tests need
    /// an active runtime. `ensure_insights_engine_and_view` would otherwise
    /// open a real `~/.spur/cache/cost.duckdb`; we pre-seed the App with an
    /// in-memory `AsyncEngine` so the constructor takes its fast path.
    fn boot_test_app() -> (tokio::runtime::Runtime, App) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut app = {
            let _guard = rt.enter();
            App::new_for_tests()
        };
        let in_memory = spur_context::AnalyticsEngine::open_in_memory().unwrap();
        in_memory.initialize().unwrap();
        in_memory.create_agent_views().unwrap();
        app.analytics_engine = Some(spur_context::AsyncEngine::new(in_memory));
        (rt, app)
    }

    #[test]
    fn alt_a_opens_insights_view() {
        let (rt, mut app) = boot_test_app();
        let _guard = rt.enter();

        app.handle_crossterm_event_for_test(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));

        assert_eq!(app.current_view(), &ViewId::Insights);
    }

    /// macOS Terminal/iTerm with default "Use Option as Meta key" OFF emits
    /// the Unicode char `å` for Option+A. The global Insights bypass must
    /// trigger AFTER `normalize_macos_option` runs at the app entry point.
    #[test]
    fn macos_option_a_opens_insights_view() {
        use crossterm::event::Event;

        let (rt, mut app) = boot_test_app();
        let _guard = rt.enter();

        app.handle_crossterm_event(Event::Key(KeyEvent::new(
            KeyCode::Char('å'),
            KeyModifiers::NONE,
        )));

        assert_eq!(app.current_view(), &ViewId::Insights);
    }
}

#[cfg(test)]
mod worker_stream_routing_tests {
    use super::super::super::*;
    use spur_acp::domain::events::{SpurEvent, SpurEventBody};
    use spur_acp::SessionId;
    use spur_acp::{ContentBlock, ContentChunk, SessionNotification, SessionUpdate, TextContent};

    fn msg_update(text: &str) -> SessionUpdate {
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            text,
        ))))
    }

    fn test_app() -> App {
        App::new_for_tests()
    }

    fn wrap_event(body: SpurEventBody) -> SpurEvent {
        SpurEvent::now(body)
    }

    #[test]
    fn worker_notification_populates_per_executor_trace() {
        let mut app = test_app();
        // Seed lineage with the executor first — routing drops orphan events.
        app.lineage
            .apply(&wrap_event(SpurEventBody::ExecutorSpawned {
                id: "exec-42".into(),
                parent_id: None,
                session_id: SessionId("abc".into()),
                agent: "claude".into(),
                role: spur_acp::Role::Executor,
                task_spec: String::new(),
            }));
        let notif = Box::new(SessionNotification::new(
            "abc",
            msg_update("hello from worker"),
        ));
        app.handle_spur_event(wrap_event(SpurEventBody::WorkerNotification {
            brain_session_id: SessionId("brain-1".into()),
            executor_id: "exec-42".into(),
            notification: notif,
        }));
        let trace = app
            .worker_streams()
            .get("exec-42")
            .expect("trace for spawned executor");
        assert_eq!(trace.entry_count(), 1);
    }

    #[tokio::test]
    async fn run_tui_replay_populates_synopsis_from_prior_ndjson() {
        use std::io::Write;

        // spur-tui does not depend on serial_test; this process-wide CWD
        // mutation can flake if another parallel test depends on CWD.
        // Share `theme::runtime::test_support::TEST_LOCK` so that any
        // `with_isolated_dirs` caller (e.g. theme threading + /theme
        // command tests) is serialized against this cwd swap.
        let _lock = crate::theme::runtime::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".spur/events").unwrap();

        // Write a fixture NDJSON file from a "prior" PID.
        let path = std::path::PathBuf::from(".spur/events/100-1000-0.ndjson");
        let mut f = std::fs::File::create(&path).unwrap();
        let ev = wrap_event(SpurEventBody::AgentNotification {
            session: spur_acp::SessionId("test-sess".into()),
            notification: Box::new(spur_acp::SessionNotification::new(
                spur_acp::AcpSessionId::new("test-sess"),
                spur_acp::SessionUpdate::UserMessageChunk(spur_acp::ContentChunk::new(
                    spur_acp::ContentBlock::Text(spur_acp::TextContent::new("hello replay")),
                )),
            )),
        });
        writeln!(f, "{}", serde_json::to_string(&ev).unwrap()).unwrap();
        let flush_ev = wrap_event(SpurEventBody::TurnComplete {
            session: spur_acp::SessionId("test-sess".into()),
        });
        writeln!(f, "{}", serde_json::to_string(&flush_ev).unwrap()).unwrap();
        drop(f);

        // Build an empty App via the existing test helper and run replay
        // against it directly, mirroring run_tui_with_license's wiring.
        let mut app = test_app();
        let cfg = spur_core::event_replay::ReplayConfig {
            events_dir: std::path::PathBuf::from(".spur/events"),
            replay_horizon: std::time::Duration::from_secs(86400 * 365),
            skip_pid: None, // include all PIDs in this test
            ..Default::default()
        };
        let stats = spur_core::event_replay::replay_events(&cfg, |ev| {
            app.lineage.apply(ev);
            app.plan_projection.apply(ev);
            app.synopsis.apply(ev);
        })
        .unwrap();

        assert_eq!(stats.events_applied, 2, "stats: {:?}", stats);
        let synopsis = app
            .synopsis
            .get(&spur_acp::SessionId("test-sess".into()))
            .expect("replay should populate synopsis for test-sess");
        assert_eq!(synopsis.last_user_msg.as_deref(), Some("hello replay"));

        std::env::set_current_dir(cwd).unwrap();
    }

    #[test]
    fn orphan_worker_notification_is_dropped() {
        let mut app = test_app();
        let notif = Box::new(SessionNotification::new("abc", msg_update("orphan")));
        app.handle_spur_event(wrap_event(SpurEventBody::WorkerNotification {
            brain_session_id: SessionId("brain-1".into()),
            executor_id: "orphan-exec".into(),
            notification: notif,
        }));
        assert!(
            app.worker_streams().get("orphan-exec").is_none(),
            "orphan events must not materialize a trace"
        );
    }

    #[test]
    fn seed_from_stream_buffer_on_rehydrate() {
        use spur_core::lineage::types::{WorkerStreamEntry, WorkerStreamKind};
        use std::time::SystemTime;

        let mut ws = crate::worker_streams::WorkerStreams::new();
        let entries = [
            WorkerStreamEntry {
                kind: WorkerStreamKind::Message,
                text: "restored".into(),
                occurred_at: SystemTime::now(),
            },
            WorkerStreamEntry {
                kind: WorkerStreamKind::Thought,
                text: "restored-2".into(),
                occurred_at: SystemTime::now(),
            },
        ];
        ws.seed_from_stream_buffer("restored-exec", "claude", entries.iter());
        let trace = ws.get("restored-exec").expect("seeded trace");
        assert_eq!(trace.entry_count(), 2);
    }

    #[test]
    fn executor_retry_started_resets_trace() {
        let mut app = test_app();
        app.lineage
            .apply(&wrap_event(SpurEventBody::ExecutorSpawned {
                id: "exec-r".into(),
                parent_id: None,
                session_id: SessionId("abc".into()),
                agent: "claude".into(),
                role: spur_acp::Role::Executor,
                task_spec: String::new(),
            }));
        app.handle_spur_event(wrap_event(SpurEventBody::WorkerNotification {
            brain_session_id: SessionId("brain-1".into()),
            executor_id: "exec-r".into(),
            notification: Box::new(SessionNotification::new("abc", msg_update("pre-retry"))),
        }));
        assert_eq!(app.worker_streams().get("exec-r").unwrap().entry_count(), 1);
        app.handle_spur_event(wrap_event(SpurEventBody::ExecutorRetryStarted {
            id: "exec-r".into(),
            attempt_n: 2,
            reason: "test retry".into(),
            new_session_id: SessionId("new-sess".into()),
        }));
        assert_eq!(
            app.worker_streams().get("exec-r").unwrap().entry_count(),
            0,
            "retry clears the per-executor trace"
        );
    }

    #[test]
    fn app_tick_drives_worker_streams_tick_all() {
        let mut app = test_app();
        app.lineage
            .apply(&wrap_event(SpurEventBody::ExecutorSpawned {
                id: "exec-tick".into(),
                session_id: spur_acp::SessionId("s".into()),
                parent_id: None,
                agent: "claude".into(),
                role: spur_acp::Role::Executor,
                task_spec: String::new(),
            }));
        app.handle_spur_event(wrap_event(SpurEventBody::WorkerNotification {
            brain_session_id: spur_acp::SessionId("brain-1".into()),
            executor_id: "exec-tick".into(),
            notification: Box::new(spur_acp::SessionNotification::new("s", msg_update("x"))),
        }));

        // Ticking must not panic and must leave the trace queryable.
        app.tick();
        app.tick();
        assert!(app.worker_streams().get("exec-tick").is_some());
    }
}

#[cfg(test)]
mod plan_projection_tests {
    use super::super::super::*;
    use spur_acp::{PlanSnapshot, PlanSnapshotCounts, PlanSnapshotTask};

    fn wrap(body: SpurEventBody) -> SpurEvent {
        SpurEvent::now(body)
    }

    fn spawn_brain(app: &mut App, session: &SessionId) {
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: session.clone(),
        }));
    }

    fn sample_plan_snapshot_event(session: &SessionId) -> SpurEvent {
        wrap(SpurEventBody::PlanSnapshotUpdated {
            session_id: session.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "p-1".into(),
                epic_id: None,
                status: "running".into(),
                progress: "0/1 done".into(),
                next_action:
                    "Use get_task_diff to review each awaiting task, then review_task to approve or reject."
                        .into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    pending: 1,
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "task-1".into(),
                    task_name: "task-1".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-1".into()),
                    issue_title: None,
                    status: "pending".into(),
                    attempt: 0,
                    max_attempts: 3,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                    unblocks: Vec::new(),
                    summary: None,
                    feedback: None,
                    error: None,
                    worker_branch: None,
                    delegation_id: None,
                    diff_summary: None,
                    mutation_id: None,
                    superseded_by: Vec::new(),
                    next_action: "wait".into(),
                }],
                owner_brain_session_id: None,
                owner_token: None,
                owner_acquired_at: None,
            }),
        })
    }

    #[test]
    fn navigate_to_plan_inspector_and_back_returns_to_session_detail() {
        let mut app = App::new_for_tests();
        let session = SessionId("brain-1".into());
        spawn_brain(&mut app, &session);

        app.process_action(Action::NavigateTo(ViewId::PlanInspector(session.clone())));
        assert!(matches!(app.current_view(), ViewId::PlanInspector(_)));

        app.process_action(Action::NavigateBack);
        assert!(matches!(app.current_view(), ViewId::SessionDetail(_)));
    }

    #[test]
    fn plan_snapshot_event_updates_plan_store() {
        let mut app = App::new_for_tests();
        let session = SessionId("brain-1".into());

        app.handle_spur_event(sample_plan_snapshot_event(&session));

        let plan = app
            .plan_projection()
            .current_for_session(&session)
            .expect("tracked plan");
        assert_eq!(plan.plan_id, "p-1");
        assert_eq!(
            plan.task("task-1").unwrap().issue_id.as_deref(),
            Some("bd-1")
        );
    }

    fn inspector_app_with_task(name: &str, status: &str) -> App {
        let mut app = App::new_for_tests();
        let session = SessionId("inspector-input-brain".into());
        spawn_brain(&mut app, &session);
        let mut event = sample_plan_snapshot_event(&session);
        if let SpurEventBody::PlanSnapshotUpdated { snapshot, .. } = &mut event.body {
            snapshot.tasks[0].task_name = name.into();
            snapshot.tasks[0].status = status.into();
            snapshot.counts = if status == "approved" {
                PlanSnapshotCounts {
                    approved: 1,
                    ..Default::default()
                }
            } else {
                PlanSnapshotCounts {
                    failed: 1,
                    ..Default::default()
                }
            };
        }
        app.handle_spur_event(event);
        app.process_action(Action::NavigateTo(ViewId::PlanInspector(session)));
        app.transient_hint = None;
        app
    }

    fn inspector_app_key(app: &mut App, code: KeyCode) {
        app.handle_crossterm_event(crossterm::event::Event::Key(KeyEvent::new(
            code,
            KeyModifiers::NONE,
        )));
    }

    fn render_inspector_app(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let mut row = String::new();
                let mut x = 0;
                while x < width {
                    let symbol = buffer[(x, y)].symbol();
                    row.push_str(symbol);
                    x += ratatui::text::Span::raw(symbol).width().max(1) as u16;
                }
                row
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn inspector_finder_app_dispatch_preserves_bare_u() {
        let mut app = inspector_app_with_task("buffer", "approved");
        inspector_app_key(&mut app, KeyCode::Char('/'));
        for ch in "buffer".chars() {
            inspector_app_key(&mut app, KeyCode::Char(ch));
        }
        let text = render_inspector_app(&mut app, 40, 8);
        assert!(
            text.contains("/ buffer"),
            "global undo consumed finder input:\n{text}"
        );
        assert!(
            app.transient_hint.is_none(),
            "query triggered a global hint/action"
        );
    }

    #[tokio::test]
    async fn inspector_app_dispatch_preserves_literal_unicode_in_text_modals() {
        let query = "façade å © ø ß u";
        for retry in [false, true] {
            let mut app = inspector_app_with_task(query, "failed");
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            app.user_input_tx = Some(tx);
            inspector_app_key(&mut app, KeyCode::Char(if retry { 'R' } else { '/' }));
            for ch in query.chars() {
                inspector_app_key(&mut app, KeyCode::Char(ch));
                assert!(
                    matches!(app.current_view(), ViewId::PlanInspector(_)),
                    "literal Unicode navigated away from inspector"
                );
            }
            if retry {
                inspector_app_key(&mut app, KeyCode::Enter);
                assert!(
                    matches!(rx.try_recv(), Ok(UserInput::RetryPlanTask { append_prompt: Some(prompt), .. }) if prompt == query),
                    "retry text was normalized or consumed before submission"
                );
            } else {
                let text = render_inspector_app(&mut app, 80, 24);
                assert!(
                    text.contains(&format!("/ {query}")) && !text.contains("No matching tasks"),
                    "literal query was changed:\n{text}"
                );
                assert!(
                    rx.try_recv().is_err(),
                    "finder text dispatched backend input"
                );
            }
        }
    }

    #[test]
    fn inspector_finder_preserves_global_quit_and_palette_priority() {
        let mut app = inspector_app_with_task("buffer", "approved");
        inspector_app_key(&mut app, KeyCode::Char('/'));
        app.handle_crossterm_event(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.palette_visible);
        inspector_app_key(&mut app, KeyCode::Esc);
        assert!(render_inspector_app(&mut app, 40, 8).contains("Find task"));
        app.handle_crossterm_event(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.quit_confirm_visible);
    }

    #[test]
    fn inspector_attention_feedback_is_visible_through_app_render() {
        for width in [40, 80] {
            for with_error in [false, true] {
                let mut app = inspector_app_with_task("done", "approved");
                if with_error {
                    app.handle_spur_event(wrap(SpurEventBody::PlanCommandError {
                        operation: "ResumePlan".into(),
                        plan_id: Some("p-1".into()),
                        error: "test command failure".into(),
                    }));
                }
                inspector_app_key(&mut app, KeyCode::Char(']'));
                let text = render_inspector_app(&mut app, width, 8);
                assert!(
                    text.contains("No tasks need attention"),
                    "App delivered an invisible navigation result at {width}x8:\n{text}"
                );
                assert!(
                    text.contains("▶ ✓ done"),
                    "notice displaced task selection:\n{text}"
                );
                let footer = text.lines().last().unwrap();
                assert!(
                    footer.contains("Tab") && footer.contains("Esc") && footer.contains("? help")
                );
                if with_error {
                    assert!(
                        text.contains("Error:"),
                        "notice erased persistent command failure"
                    );
                }
                let y = text
                    .lines()
                    .position(|line| line.contains("No tasks need attention"))
                    .unwrap();
                let cells =
                    ratatui::text::Span::raw(text.lines().nth(y).unwrap().trim_end()).width();
                println!(
                    "R2_GEOMETRY {}",
                    serde_json::json!({"width":width,"height":8,"with_error":with_error,"notice":{"x":0,"y":y,"width":cells,"height":1}})
                );
                inspector_app_key(&mut app, KeyCode::Char('?'));
                assert!(
                    render_inspector_app(&mut app, 100, 40).contains("No tasks need attention"),
                    "active notice missing from help"
                );
            }
        }
    }
}

#[cfg(test)]
mod brain_retired_tests {
    //! Second-order consumers of `SpurEventBody::BrainRetired` on the App
    //! side. Commit 1 wired the lineage projection; these tests cover the
    //! App-level state that must also react, namely:
    //!
    //! - `brain_name` must null out on retire so readbacks between `/clear`
    //!   and the next prompt are not stale (R5).
    //! - `metadata_store.last_active_*` must be cleared so `/clear` followed
    //!   by a process quit does NOT auto-resume the retired session on the
    //!   next `spur watch` launch (R7; the real user-visible bug).
    //!
    //! These tests exercise private fields, so they live in-module.
    use super::super::super::*;
    use spur_acp::domain::events::{BrainRetireReason, SpurEvent, SpurEventBody};
    use spur_acp::SessionId;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;
    use tokio::time::{timeout, Duration};

    fn wrap(body: SpurEventBody) -> SpurEvent {
        SpurEvent::now(body)
    }

    /// Construct an `App` with a live `user_input_tx` so tests that go
    /// through `Action::ClearSession` (which requires `tx.try_send` to
    /// succeed for the send-first reset gate) can observe the reset.
    /// Returns the receiver so the channel stays open for the test's
    /// lifetime.
    fn app_with_user_input_tx() -> (App, tokio::sync::mpsc::Receiver<UserInput>) {
        let (tx, rx) = tokio::sync::mpsc::channel::<UserInput>(8);
        (App::new(Some(tx), false), rx)
    }

    struct SocketCleanup {
        socket_path: std::path::PathBuf,
        _tempdir: tempfile::TempDir,
    }

    impl Drop for SocketCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    fn bind_notebook_listener(label: &str) -> (String, SocketCleanup, UnixListener) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let tempdir = tempfile::Builder::new()
            .prefix("nb")
            .tempdir_in("/tmp")
            .expect("notebook socket tempdir");
        let nonce = tempdir
            .path()
            .join(format!("n-{label}-{}-{nanos}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let socket_path = spur_core::notebook::control_socket_path(&nonce);
        let listener = UnixListener::bind(&socket_path).expect("bind notebook control socket");
        (
            nonce,
            SocketCleanup {
                socket_path,
                _tempdir: tempdir,
            },
            listener,
        )
    }

    async fn read_notebook_frame(stream: &mut tokio::net::UnixStream) -> serde_json::Value {
        let mut len = [0_u8; 4];
        stream.read_exact(&mut len).await.expect("frame length");
        let len = u32::from_be_bytes(len) as usize;
        let mut bytes = vec![0_u8; len];
        stream.read_exact(&mut bytes).await.expect("frame body");
        serde_json::from_slice(&bytes).expect("request json")
    }

    async fn write_notebook_frame(stream: &mut tokio::net::UnixStream, value: serde_json::Value) {
        let bytes = serde_json::to_vec(&value).expect("response json");
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .expect("write frame length");
        stream.write_all(&bytes).await.expect("write frame body");
        stream.flush().await.expect("flush frame");
    }

    fn effort_config_option() -> spur_acp::SessionConfigOption {
        use spur_acp::{SessionConfigId, SessionConfigOption, SessionConfigSelectOption};

        SessionConfigOption::select(
            SessionConfigId::new("reasoning_effort".to_string()),
            "effort".to_string(),
            "medium".to_string(),
            vec![SessionConfigSelectOption::new(
                "medium".to_string(),
                "Medium".to_string(),
            )],
        )
    }

    fn caps_without_config_options() -> std::sync::Arc<spur_acp::SpurAgentCaps> {
        let init = spur_acp::InitializeResponse::new(spur_acp::ProtocolVersion::LATEST);
        let new = spur_acp::NewSessionResponse::new(spur_acp::AcpSessionId::new("acp-b1"));
        std::sync::Arc::new(spur_acp::SpurAgentCaps::new(
            &init,
            &new,
            spur_acp::AgentKind::CodexAcp,
        ))
    }

    fn caps_with_effort() -> std::sync::Arc<spur_acp::SpurAgentCaps> {
        let init = spur_acp::InitializeResponse::new(spur_acp::ProtocolVersion::LATEST);
        let mut new = spur_acp::NewSessionResponse::new(spur_acp::AcpSessionId::new("acp-b1"));
        new.config_options = Some(vec![effort_config_option()]);
        std::sync::Arc::new(spur_acp::SpurAgentCaps::new(
            &init,
            &new,
            spur_acp::AgentKind::CodexAcp,
        ))
    }

    #[test]
    fn agent_session_ready_installs_caps_on_session_detail() {
        let mut app = App::new_for_tests();
        let session = SessionId("b1".into());
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "codex".into(),
            session: session.clone(),
        }));
        app.handle_spur_event(wrap(SpurEventBody::CommandRegistryDirty {
            session: session.clone(),
            caps: Some(caps_with_effort()),
            config_options: vec![effort_config_option()],
        }));

        let names_before: Vec<String> = app
            .session_detail
            .as_ref()
            .expect("BrainSpawned must create session detail")
            .available_slash_commands()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(
            names_before.iter().any(|name| name == "effort"),
            "precondition: advertised caps include /effort; got {names_before:?}"
        );

        app.handle_spur_event(wrap(SpurEventBody::AgentSessionReady {
            session: session.clone(),
            acp_session_id: "acp-b1".into(),
            brain: "codex".into(),
            resumed: false,
            cancel_mode: spur_acp::CancelMode::AcpSoft,
            fs_unsafe: false,
            caps: Some(caps_without_config_options()),
        }));

        let names_after: Vec<String> = app
            .session_detail
            .as_ref()
            .expect("session detail must remain focused")
            .available_slash_commands()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(
            !names_after.iter().any(|name| name == "effort"),
            "AgentSessionReady caps must constrain advertised commands; got {names_after:?}"
        );
    }

    #[test]
    fn brain_retired_nulls_brain_name() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));
        assert_eq!(app.brain_name.as_deref(), Some("kiro"));

        app.handle_spur_event(wrap(SpurEventBody::BrainRetired {
            session: SessionId("b1".into()),
            reason: BrainRetireReason::UserClear,
        }));

        assert!(
            app.brain_name.is_none(),
            "brain_name must null on retire so readbacks aren't stale"
        );
    }

    #[test]
    fn brain_retired_clears_last_active_auto_resume_pointers() {
        // Simulates: BrainSpawned → AgentSessionReady writes last_active_*
        // → /clear emits BrainRetired → arm clears last_active_*.
        // Result: spur-cli's `last_active_acp()` returns None on relaunch.
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));
        app.handle_spur_event(wrap(SpurEventBody::AgentSessionReady {
            session: SessionId("b1".into()),
            acp_session_id: "acp-b1".into(),
            brain: "kiro".into(),
            resumed: false,
            cancel_mode: spur_acp::CancelMode::AcpSoft,
            fs_unsafe: false,
            caps: None,
        }));
        assert!(
            app.metadata_store.last_active_acp().is_some(),
            "precondition: AgentSessionReady seeds last_active_acp"
        );

        app.handle_spur_event(wrap(SpurEventBody::BrainRetired {
            session: SessionId("b1".into()),
            reason: BrainRetireReason::UserClear,
        }));

        assert!(
            app.metadata_store.last_active_acp().is_none(),
            "last_active_acp must be cleared on retire so /clear+quit doesn't auto-resume"
        );
    }

    #[test]
    fn clear_session_resets_session_detail_on_successful_send() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));

        let sid_before = app.session_detail.as_ref().unwrap().session_id().clone();
        app.process_action(Action::ClearSession);

        let detail = app.session_detail.as_ref().expect("view must still exist");
        assert!(detail.is_cleared());
        assert!(detail.ready_banner_text().is_some());
        assert_eq!(detail.session_id(), &sid_before, "session_id stays retired");
        assert_eq!(app.brain_status, BrainStatus::Idle);
    }

    #[tokio::test]
    async fn resume_session_does_not_close_shared_notebook_daemon() {
        let (nonce, _cleanup, listener) = bind_notebook_listener("resume");
        let mut accepted_close = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept command");
            read_notebook_frame(&mut stream).await
        });

        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("notebook-a".into()),
        }));
        app.handle_spur_event(wrap(SpurEventBody::NotebookSocketReady {
            session: SessionId("notebook-a".into()),
            socket_nonce: nonce,
        }));

        app.process_action(Action::ResumeSession {
            session_id: "notebook-b".into(),
        });

        match timeout(Duration::from_millis(150), &mut accepted_close).await {
            Ok(Ok(request)) => panic!(
                "session switch must not close shared notebook daemon; got command {:?}",
                request["command"]
            ),
            Ok(Err(err)) => panic!("notebook accept task failed: {err}"),
            Err(_) => {
                accepted_close.abort();
            }
        }
    }

    #[tokio::test]
    async fn notebook_command_uses_shared_nonce_after_prior_session_completes() {
        let (nonce, _cleanup, listener) = bind_notebook_listener("shared");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept command");
            let request = read_notebook_frame(&mut stream).await;
            write_notebook_frame(
                &mut stream,
                serde_json::json!({
                    "ok": true,
                    "path": null,
                    "error": null
                }),
            )
            .await;
            request
        });

        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("notebook-a".into()),
        }));
        app.handle_spur_event(wrap(SpurEventBody::NotebookSocketReady {
            session: SessionId("notebook-a".into()),
            socket_nonce: nonce,
        }));
        app.handle_spur_event(wrap(SpurEventBody::SessionCompleted {
            session: SessionId("notebook-a".into()),
            success: true,
        }));
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("notebook-b".into()),
        }));

        app.process_action(Action::NotebookCommand { arg: "new".into() });

        let request = timeout(Duration::from_millis(500), server)
            .await
            .expect("notebook command should reach shared socket")
            .expect("server task");
        assert_eq!(request["daemon"], "notebook.v1");
        assert_eq!(request["command"], "new");
        assert!(request.get("path").is_none());
    }

    #[test]
    fn clear_session_preserves_input_bar_contents() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));
        app.session_detail
            .as_mut()
            .unwrap()
            .input_bar_mut_for_test()
            .set_text("typed before clear".into(), 18);

        app.process_action(Action::ClearSession);

        assert_eq!(
            app.session_detail.as_ref().unwrap().input_bar_text(),
            "typed before clear"
        );
    }

    #[test]
    fn clear_while_streaming_does_not_panic_and_resets_flags() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));
        app.session_detail.as_mut().unwrap().stream_in_flight = true;

        app.process_action(Action::ClearSession);

        let detail = app.session_detail.as_ref().unwrap();
        assert!(!detail.stream_in_flight);
        assert!(detail.is_cleared());
    }

    #[test]
    fn connected_dashboard_first_submit_still_spawns_new_session() {
        let (mut app, mut rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainConnected {
            brain: "kiro".into(),
        }));

        assert_eq!(app.brain_status, BrainStatus::Connected);

        app.handle_crossterm_event_for_test(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('h'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_crossterm_event_for_test(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('i'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_crossterm_event_for_test(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));

        match rx.try_recv() {
            Ok(UserInput::NewSessionWithMessage { blocks, interrupt }) => {
                assert_eq!(blocks.len(), 1, "expected single text block");
                assert!(!interrupt, "plain Enter must not set interrupt");
            }
            Ok(_) => panic!("expected NewSessionWithMessage"),
            Err(err) => panic!("expected queued user input, got {err:?}"),
        }
    }

    #[test]
    fn brain_retired_user_clear_resets_view_defensively() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));

        app.handle_spur_event(wrap(SpurEventBody::BrainRetired {
            session: SessionId("b1".into()),
            reason: BrainRetireReason::UserClear,
        }));

        let detail = app.session_detail.as_ref().unwrap();
        assert!(detail.is_cleared());
        assert!(detail.ready_banner_text().is_some());
    }

    #[test]
    fn brain_retired_resume_switch_does_not_reset_view() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));

        app.handle_spur_event(wrap(SpurEventBody::BrainRetired {
            session: SessionId("b1".into()),
            reason: BrainRetireReason::ResumeSwitch,
        }));

        let detail = app.session_detail.as_ref().unwrap();
        assert!(
            !detail.is_cleared(),
            "ResumeSwitch must NOT trigger view reset"
        );
        assert!(detail.ready_banner_text().is_none());
    }

    #[test]
    fn draft_carryover_across_clear_to_new_brain_spawn() {
        // Use unique session IDs to avoid cross-test pollution from the
        // shared on-disk metadata store.
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("carryover-a".into()),
        }));
        // Seed session A's saved draft.
        app.session_detail
            .as_mut()
            .unwrap()
            .input_bar_mut_for_test()
            .set_text("draft-A".into(), 7);
        app.process_action(Action::SaveDraft {
            session_id: "carryover-a".into(),
            draft: "draft-A".into(),
        });

        // User submits /clear.
        app.process_action(Action::ClearSession);

        // User types a new prompt into the preserved InputBar.
        app.session_detail
            .as_mut()
            .unwrap()
            .input_bar_mut_for_test()
            .set_text("post-clear-prompt".into(), 17);

        // New brain B spawns.
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("carryover-b".into()),
        }));

        // A's saved draft was NOT corrupted.
        let metadata_a_draft = app
            .metadata_store
            .entry("carryover-a")
            .map(|e| e.draft.clone())
            .unwrap_or_default();
        assert_eq!(metadata_a_draft, "draft-A");

        // New view for B has the carryover.
        let detail = app.session_detail.as_ref().unwrap();
        assert_eq!(detail.session_id().0, "carryover-b");
        assert_eq!(detail.input_bar_text(), "post-clear-prompt");
    }

    #[test]
    fn draft_carryover_empty_is_noop() {
        // Use unique session IDs to avoid cross-test pollution from the
        // shared on-disk metadata store.
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("empty-carryover-a".into()),
        }));
        app.process_action(Action::ClearSession);
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("empty-carryover-b".into()),
        }));

        let detail = app.session_detail.as_ref().unwrap();
        assert_eq!(detail.input_bar_text(), "");
        let md = &app.metadata_store;
        assert!(md
            .entry("empty-carryover-a")
            .map(|e| e.draft.clone())
            .unwrap_or_default()
            .is_empty());
        assert!(md
            .entry("empty-carryover-b")
            .map(|e| e.draft.clone())
            .unwrap_or_default()
            .is_empty());
    }

    #[test]
    fn clear_session_banner_cleared_on_next_brain_spawn() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("banner-a".into()),
        }));
        app.process_action(Action::ClearSession);
        assert!(app
            .session_detail
            .as_ref()
            .unwrap()
            .ready_banner_text()
            .is_some());

        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("banner-b".into()),
        }));

        let detail = app.session_detail.as_ref().unwrap();
        assert!(detail.ready_banner_text().is_none());
        assert!(!detail.is_cleared());
    }

    #[test]
    fn clear_end_to_end_flow() {
        let (mut app, _rx) = app_with_user_input_tx();

        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("e2e-a".into()),
        }));
        app.session_detail
            .as_mut()
            .unwrap()
            .input_bar_mut_for_test()
            .set_text("mid-thought".into(), 11);

        app.process_action(Action::ClearSession);
        {
            let d = app.session_detail.as_ref().unwrap();
            assert!(d.is_cleared());
            assert!(d.ready_banner_text().is_some());
            assert_eq!(d.input_bar_text(), "mid-thought");
        }

        app.handle_spur_event(wrap(SpurEventBody::BrainRetired {
            session: SessionId("e2e-a".into()),
            reason: BrainRetireReason::UserClear,
        }));
        {
            let d = app.session_detail.as_ref().unwrap();
            assert!(d.is_cleared());
            assert_eq!(d.input_bar_text(), "mid-thought");
        }

        app.session_detail
            .as_mut()
            .unwrap()
            .input_bar_mut_for_test()
            .set_text("explain quicksort".into(), 17);
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("e2e-b".into()),
        }));

        let d = app.session_detail.as_ref().unwrap();
        assert_eq!(d.session_id().0, "e2e-b");
        assert!(!d.is_cleared());
        assert!(d.ready_banner_text().is_none());
        assert_eq!(d.input_bar_text(), "explain quicksort");
    }

    #[test]
    fn double_clear_session_is_idempotent() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("double-a".into()),
        }));
        app.process_action(Action::ClearSession);
        app.process_action(Action::ClearSession);
        let d = app.session_detail.as_ref().unwrap();
        assert!(d.is_cleared());
        assert!(d.ready_banner_text().is_some());
    }

    #[test]
    fn clear_over_resume_banner_takes_precedence() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("resume-banner-a".into()),
        }));
        app.session_detail
            .as_mut()
            .unwrap()
            .show_resume_banner("t".into(), "1s ago".into());

        app.process_action(Action::ClearSession);

        let d = app.session_detail.as_ref().unwrap();
        // reset_for_clear wipes resume_banner; ready_banner is now the only one.
        assert!(
            !d.has_resume_banner(),
            "resume_banner must be cleared by reset_for_clear"
        );
        assert!(d.ready_banner_text().is_some());
    }

    #[test]
    fn clear_mid_tool_call_clears_tool_depth() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("mid-tool-a".into()),
        }));
        {
            let detail = app.session_detail.as_mut().unwrap();
            detail.tool_depth_for_test_mut().insert("t1".into(), 1);
            detail.tool_depth_for_test_mut().insert("t2".into(), 2);
        }

        app.process_action(Action::ClearSession);

        assert!(app
            .session_detail
            .as_ref()
            .unwrap()
            .tool_depth_for_test()
            .is_empty());
    }

    #[test]
    fn debounce_tick_after_clear_does_not_save_to_retired_session() {
        let (mut app, _rx) = app_with_user_input_tx();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("debounce-a".into()),
        }));
        // User had a draft 'draft-A' saved.
        app.process_action(Action::SaveDraft {
            session_id: "debounce-a".into(),
            draft: "draft-A".into(),
        });
        // /clear + new typing.
        app.process_action(Action::ClearSession);
        app.session_detail
            .as_mut()
            .unwrap()
            .input_bar_mut_for_test()
            .set_text("post-clear".into(), 10);
        // Force the debounce to trigger (600ms ago).
        app.session_detail
            .as_mut()
            .unwrap()
            .test_set_last_draft_change({
                let now = std::time::Instant::now();
                now.checked_sub(std::time::Duration::from_millis(600))
                    .unwrap_or(now)
            });
        let action = app.session_detail.as_mut().unwrap().draft_save_action();
        assert!(
            action.is_none(),
            "cleared view must not emit SaveDraft from tick"
        );

        // A's draft must still be 'draft-A'.
        assert_eq!(
            app.metadata_store.entry("debounce-a").unwrap().draft,
            "draft-A"
        );
    }

    #[test]
    fn brain_retired_shutdown_does_not_panic() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("b1".into()),
        }));

        app.handle_spur_event(wrap(SpurEventBody::BrainRetired {
            session: SessionId("b1".into()),
            reason: BrainRetireReason::Shutdown,
        }));

        let detail = app.session_detail.as_ref().unwrap();
        assert!(!detail.is_cleared());
    }

    #[test]
    fn clear_session_with_no_tx_does_not_reset_view() {
        // Spec §3.6: Action::ClearSession must NOT reset the view when
        // `user_input_tx` is None. No brain retirement can be requested,
        // so a visual reset here would produce a ghost-cleared state
        // (view says "cleared" while the stale brain is still active).
        let mut app = App::new_for_tests(); // user_input_tx = None
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("no-tx-a".into()),
        }));
        // Set brain_status to a distinctive non-Idle value so we can
        // assert it is NOT forced to Idle by the ghost-clear path.
        app.brain_status = BrainStatus::Thinking;

        app.process_action(Action::ClearSession);

        let detail = app.session_detail.as_ref().expect("view must still exist");
        assert!(
            !detail.is_cleared(),
            "view must NOT enter cleared state without a successful send"
        );
        assert!(
            detail.ready_banner_text().is_none(),
            "no ready banner without a successful clear"
        );
        assert_eq!(
            app.brain_status,
            BrainStatus::Thinking,
            "brain_status must be unchanged when send is skipped (not forced to Idle)"
        );
    }

    #[test]
    fn clear_session_with_full_tx_does_not_reset_view() {
        // Spec §3.6: Action::ClearSession must NOT reset the view when
        // `tx.try_send` returns an Err. Dropping the receiver forces
        // `TrySendError::Closed`, which exercises the same Err branch
        // as a saturated channel (both are the send-failure gate).
        let (tx, rx) = tokio::sync::mpsc::channel::<UserInput>(1);
        drop(rx); // subsequent try_send returns TrySendError::Closed
        let mut app = App::new(Some(tx), false);
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "kiro".into(),
            session: SessionId("full-tx-a".into()),
        }));
        app.brain_status = BrainStatus::Thinking;

        app.process_action(Action::ClearSession);

        let detail = app.session_detail.as_ref().expect("view must still exist");
        assert!(
            !detail.is_cleared(),
            "view must NOT enter cleared state when send fails"
        );
        assert!(
            detail.ready_banner_text().is_none(),
            "no ready banner without a successful clear"
        );
        assert_eq!(
            app.brain_status,
            BrainStatus::Thinking,
            "brain_status must be unchanged on send failure (not forced to Idle)"
        );
    }
}

#[cfg(test)]
mod feature_gate_tests {
    use super::super::super::*;
    use spur_license::{FeatureGateError, FeatureKey, Plan, Tier};

    #[test]
    fn send_message_denied_by_feature_gate_opens_upgrade_modal() {
        let mut app = App::new_for_tests();
        app.feature_gate
            .update_state(&spur_license::LicenseState::inactive("stripped for test"));

        app.process_action(Action::SendMessage {
            session: spur_acp::SessionId("session-1".to_string()),
            blocks: Vec::new(),
            interrupt: false,
        });

        let modal = app
            .upgrade_modal
            .as_ref()
            .expect("denied send-message action must open upgrade modal");
        assert_eq!(modal.required_tier, Some(Plan::Community));
        match &modal.err {
            FeatureGateError::Denied { key, tier } => {
                assert_eq!(*key, FeatureKey::CLI_CORE_EXEC);
                assert_eq!(*tier, Tier::Community);
            }
            other => panic!("unexpected feature gate error: {other:?}"),
        }
    }

    #[test]
    fn show_session_cost_denied_for_community_opens_pro_upgrade_modal() {
        let mut app = App::new_for_tests();

        app.process_action(Action::ShowSessionCost);

        let modal = app
            .upgrade_modal
            .as_ref()
            .expect("community session-cost action must open upgrade modal");
        assert_eq!(modal.required_tier, Some(Plan::Pro));
        match &modal.err {
            FeatureGateError::Denied { key, tier } => {
                assert_eq!(*key, FeatureKey::COST_PRO_PER_PROJECT_TRACKING);
                assert_eq!(*tier, Tier::Community);
            }
            other => panic!("unexpected feature gate error: {other:?}"),
        }
    }
}

#[cfg(test)]
mod quit_shortcut_tests {
    use super::super::super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    #[test]
    fn first_ctrl_c_opens_quit_confirm_without_exiting() {
        let mut app = App::new_for_tests();

        app.handle_crossterm_event_for_test(ctrl_c());

        assert!(
            app.quit_confirm_visible,
            "first Ctrl+C should open the quit prompt"
        );
        assert!(!app.should_quit, "first Ctrl+C must not exit immediately");
    }

    #[test]
    fn second_ctrl_c_force_quits_from_confirm() {
        let mut app = App::new_for_tests();

        app.handle_crossterm_event_for_test(ctrl_c());
        app.handle_crossterm_event_for_test(ctrl_c());

        assert!(
            app.should_quit,
            "second Ctrl+C should bypass confirmation and exit"
        );
        assert!(
            !app.quit_confirm_visible,
            "force quit should dismiss the confirm dialog"
        );
    }

    #[test]
    fn quit_confirm_accepts_y_and_cancels_on_n() {
        let mut app = App::new_for_tests();

        app.handle_crossterm_event_for_test(ctrl_c());
        app.handle_crossterm_event_for_test(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(
            !app.quit_confirm_visible,
            "n should dismiss the quit prompt"
        );
        assert!(!app.should_quit, "n must keep the app running");

        app.handle_crossterm_event_for_test(ctrl_c());
        app.handle_crossterm_event_for_test(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.should_quit, "y should confirm quit");
    }

    #[test]
    fn dashboard_esc_no_longer_quits_when_nothing_is_active() {
        let mut app = App::new_for_tests();

        app.handle_crossterm_event_for_test(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(
            !app.should_quit,
            "Esc should not exit the app from an empty dashboard"
        );
        assert!(
            !app.quit_confirm_visible,
            "Esc should not open quit confirm from an empty dashboard"
        );
    }

    #[test]
    fn paste_is_ignored_while_app_overlays_are_active() {
        let mut app = App::new_for_tests();

        app.help_visible = true;
        app.handle_crossterm_event(Event::Paste("help".into()));
        assert_eq!(app.dashboard_for_test().input_bar_text_for_test(), "");
        app.help_visible = false;

        app.quit_confirm_visible = true;
        app.handle_crossterm_event(Event::Paste("quit".into()));
        assert_eq!(app.dashboard_for_test().input_bar_text_for_test(), "");
        app.quit_confirm_visible = false;

        app.palette_visible = true;
        app.handle_crossterm_event(Event::Paste("palette".into()));
        assert_eq!(app.dashboard_for_test().input_bar_text_for_test(), "");
        app.palette_visible = false;

        app.collision_modal = Some(CollisionModalState {
            acp_id: "acp-1".into(),
            holder: spur_acp::session_lock::HolderInfo::default(),
        });
        app.handle_crossterm_event(Event::Paste("collision".into()));
        assert_eq!(app.dashboard_for_test().input_bar_text_for_test(), "");
        app.collision_modal = None;

        app.handle_crossterm_event(Event::Paste("visible".into()));
        assert_eq!(
            app.dashboard_for_test().input_bar_text_for_test(),
            "visible"
        );
    }

    /// Regression: when `quit_confirm_visible` is true, the upgrade modal
    /// must NOT render even if `upgrade_modal` is `Some`. Otherwise input
    /// (handled by quit_confirm) and visuals (upgrade modal on top)
    /// silently disagree — the user sees the wrong dialog for their keys.
    #[test]
    fn upgrade_modal_render_gate_respects_quit_and_collision_precedence() {
        use crate::components::upgrade_modal::UpgradeModalState;
        use spur_license::{FeatureGateError, FeatureKey, Tier};

        let mut app = App::new_for_tests();
        app.upgrade_modal = Some(UpgradeModalState {
            err: FeatureGateError::Denied {
                key: FeatureKey::CLI_CORE_EXEC,
                tier: Tier::Community,
            },
            required_tier: None,
        });

        // Baseline: nothing else up — upgrade modal should render.
        assert!(
            app.should_render_upgrade_modal(),
            "upgrade modal should render when no higher-precedence modal is up"
        );

        // quit_confirm preempts upgrade modal.
        app.quit_confirm_visible = true;
        assert!(
            !app.should_render_upgrade_modal(),
            "upgrade modal must NOT render when quit_confirm_visible is true"
        );
        app.quit_confirm_visible = false;

        // collision preempts upgrade modal.
        app.collision_modal = Some(CollisionModalState {
            acp_id: "acp-1".into(),
            holder: spur_acp::session_lock::HolderInfo::default(),
        });
        assert!(
            !app.should_render_upgrade_modal(),
            "upgrade modal must NOT render when collision_modal is up"
        );

        // Both up: still suppressed.
        app.quit_confirm_visible = true;
        assert!(
            !app.should_render_upgrade_modal(),
            "upgrade modal must NOT render when quit_confirm and collision are both up"
        );
        app.collision_modal = None;
        app.quit_confirm_visible = false;

        // Back to baseline.
        assert!(app.should_render_upgrade_modal());
    }
}

#[cfg(test)]
mod crossterm_stream_tests {
    use super::super::super::events::handle_crossterm_stream_result;
    use super::super::super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::io;

    #[test]
    fn crossterm_read_error_shuts_down_within_bounded_iterations() {
        let mut app = App::new_for_tests();

        for _ in 0..3 {
            handle_crossterm_stream_result(
                &mut app,
                Some(Err(io::Error::new(io::ErrorKind::Other, "tty read failed"))),
            );
            if app.should_quit {
                break;
            }
        }

        assert!(app.should_quit, "read errors should confirm quit");
    }

    #[test]
    fn crossterm_stream_end_shuts_down_immediately() {
        let mut app = App::new_for_tests();

        handle_crossterm_stream_result(&mut app, None);

        assert!(app.should_quit, "stream end should confirm quit");
    }

    #[test]
    fn crossterm_happy_path_dispatches_event_unchanged() {
        let mut app = App::new_for_tests();
        let event = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        let handled = handle_crossterm_stream_result(&mut app, Some(Ok(event)));

        assert!(handled, "happy path should report one handled event");
        assert!(
            app.quit_confirm_visible,
            "Ctrl-C should still dispatch through App input handling"
        );
        assert!(!app.should_quit, "first Ctrl-C must not force shutdown");
    }
}

#[cfg(test)]
mod session_identity_tests {
    use super::super::super::*;

    #[test]
    fn resume_migrates_raw_draft_before_brain_spawned_restores_detail() {
        let acp_id = "legacy-acp-session";
        let spur_id = spur_core::plan::labels::derive_brain_session_id(&SessionId(acp_id.into()))
            .into_session_id();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = SessionMetadataStore::load(tmp.path());
        store.upsert_entry(
            acp_id.into(),
            crate::session_metadata::SessionEntry {
                draft: "keep this draft".into(),
                ..Default::default()
            },
        );

        let mut app = App::new_for_tests();
        app.set_metadata_store_for_test(store);
        app.process_action(Action::ResumeSession {
            session_id: acp_id.into(),
        });

        assert_eq!(
            app.current_view_for_test(),
            &ViewId::SessionDetail(spur_id.clone()),
            "optimistic resume view must use the local SPUR id"
        );
        let detail = app
            .session_detail_for_test()
            .expect("optimistic session detail created");
        assert_eq!(detail.session_id(), &spur_id);

        app.handle_spur_event(SpurEvent::now(SpurEventBody::SessionLoading {
            session: spur_id.clone(),
        }));
        assert!(matches!(
            app.session_detail_for_test()
                .expect("optimistic session detail retained")
                .load_state(),
            crate::views::session_detail::LoadState::Loading
        ));

        app.handle_spur_event(SpurEvent::now(SpurEventBody::BrainSpawned {
            agent: "claude".into(),
            session: spur_id.clone(),
        }));
        app.handle_spur_event(SpurEvent::now(SpurEventBody::AgentSessionReady {
            session: spur_id.clone(),
            acp_session_id: acp_id.into(),
            brain: "claude".into(),
            resumed: true,
            cancel_mode: spur_acp::CancelMode::AcpSoft,
            fs_unsafe: false,
            caps: None,
        }));

        let detail = app
            .session_detail
            .as_ref()
            .expect("session detail restored");
        assert_eq!(detail.session_id(), &spur_id);
        assert_eq!(detail.input_bar_text_for_test(), "keep this draft");
        assert_eq!(
            app.metadata_store_for_test()
                .entry(&spur_id.0)
                .and_then(|entry| entry.acp_session_id.as_deref()),
            Some(acp_id)
        );
    }

    #[test]
    fn resume_does_not_restore_draft_from_foreign_destination_mapping() {
        let acp_id = "target-acp-session";
        let spur_id = spur_core::plan::labels::derive_brain_session_id(&SessionId(acp_id.into()))
            .into_session_id();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = SessionMetadataStore::load(tmp.path());
        store.upsert_entry(
            spur_id.0.clone(),
            crate::session_metadata::SessionEntry {
                draft: "foreign draft".into(),
                acp_session_id: Some("different-acp-session".into()),
                ..Default::default()
            },
        );

        let mut app = App::new_for_tests();
        app.set_metadata_store_for_test(store);
        app.process_action(Action::ResumeSession {
            session_id: acp_id.into(),
        });

        let detail = app
            .session_detail_for_test()
            .expect("optimistic session detail created");
        assert_eq!(detail.session_id(), &spur_id);
        assert!(
            detail.input_bar_text_for_test().is_empty(),
            "foreign destination metadata must not leak into the resumed session"
        );

        app.handle_spur_event(SpurEvent::now(SpurEventBody::BrainSpawned {
            agent: "claude".into(),
            session: spur_id.clone(),
        }));
        assert!(
            app.session_detail_for_test()
                .expect("configured detail replaces optimistic placeholder")
                .input_bar_text_for_test()
                .is_empty(),
            "BrainSpawned must not bypass destination ownership"
        );
        app.handle_spur_event(SpurEvent::now(SpurEventBody::AgentSessionReady {
            session: spur_id.clone(),
            acp_session_id: acp_id.into(),
            brain: "claude".into(),
            resumed: true,
            cancel_mode: spur_acp::CancelMode::AcpSoft,
            fs_unsafe: false,
            caps: None,
        }));
        let entry = app
            .metadata_store_for_test()
            .entry(&spur_id.0)
            .expect("foreign destination retained");
        assert_eq!(
            entry.acp_session_id.as_deref(),
            Some("different-acp-session")
        );
        assert_eq!(entry.draft, "foreign draft");
    }

    #[test]
    fn resume_does_not_restore_draft_from_unproven_destination_entry() {
        let acp_id = "target-acp-session";
        let spur_id = spur_core::plan::labels::derive_brain_session_id(&SessionId(acp_id.into()))
            .into_session_id();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = SessionMetadataStore::load(tmp.path());
        store.upsert_entry(
            spur_id.0.clone(),
            crate::session_metadata::SessionEntry {
                draft: "unproven draft".into(),
                ..Default::default()
            },
        );

        let mut app = App::new_for_tests();
        app.set_metadata_store_for_test(store);
        app.process_action(Action::ResumeSession {
            session_id: acp_id.into(),
        });

        assert!(
            app.session_detail_for_test()
                .expect("optimistic session detail created")
                .input_bar_text_for_test()
                .is_empty(),
            "unmapped destination metadata must not be assumed to belong to the ACP row"
        );

        app.handle_spur_event(SpurEvent::now(SpurEventBody::BrainSpawned {
            agent: "claude".into(),
            session: spur_id.clone(),
        }));
        assert!(
            app.session_detail_for_test()
                .expect("configured detail replaces optimistic placeholder")
                .input_bar_text_for_test()
                .is_empty(),
            "BrainSpawned must not restore unproven destination metadata"
        );
        app.handle_spur_event(SpurEvent::now(SpurEventBody::AgentSessionReady {
            session: spur_id.clone(),
            acp_session_id: acp_id.into(),
            brain: "claude".into(),
            resumed: true,
            cancel_mode: spur_acp::CancelMode::AcpSoft,
            fs_unsafe: false,
            caps: None,
        }));
        let entry = app
            .metadata_store_for_test()
            .entry(&spur_id.0)
            .expect("unproven destination retained");
        assert_eq!(entry.acp_session_id, None);
        assert_eq!(entry.draft, "unproven draft");

        app.process_action(Action::ResumeSession {
            session_id: acp_id.into(),
        });
        assert!(
            app.session_detail_for_test()
                .expect("subsequent optimistic detail created")
                .input_bar_text_for_test()
                .is_empty(),
            "a rejected AgentSessionReady mapping must not become trusted later"
        );
    }
}

#[cfg(test)]
mod synopsis_wire_tests {
    use super::super::super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use spur_acp::domain::events::{SpurEvent, SpurEventBody};
    use spur_acp::{ContentBlock, ContentChunk, SessionNotification, SessionUpdate, TextContent};
    use spur_acp::{SessionId, SessionInfo};
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    fn wrap(body: SpurEventBody) -> SpurEvent {
        SpurEvent::now(body)
    }

    fn user_message(session: &str, text: &str) -> SpurEvent {
        wrap(SpurEventBody::AgentNotification {
            session: SessionId(session.into()),
            notification: Box::new(SessionNotification::new(
                spur_acp::AcpSessionId::new(session),
                SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
                    TextContent::new(text),
                ))),
            )),
        })
    }

    fn session(id: &str, title: &str) -> SessionInfo {
        SessionInfo::new(id.to_string(), PathBuf::from("/tmp")).title(title.to_string())
    }

    fn app_in_picker_with_empty_metadata() -> App {
        let tmp = NamedTempFile::new().unwrap();
        let mut app = App::new_for_tests();
        app.set_metadata_store_for_test(SessionMetadataStore::load(tmp.path()));
        app.process_action(Action::RequestSessions);
        app
    }

    fn type_picker_search(app: &mut App, query: &str) {
        app.handle_crossterm_event(Event::Key(KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        )));
        for ch in query.chars() {
            app.handle_crossterm_event(Event::Key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            )));
        }
    }

    #[test]
    fn handle_spur_event_applies_to_synopsis_projection() {
        let mut app = App::new_for_tests();

        app.handle_spur_event(user_message("S1", "hello world"));

        let s = app
            .synopsis()
            .get(&SessionId("S1".into()))
            .expect("commit-on-read fallback");
        assert_eq!(s.last_user_msg.as_deref(), Some("hello world"));
    }

    #[test]
    fn picker_filter_picks_up_late_synopsis_updates_without_refresh() {
        let acp_id = "acp-session-S1";
        let spur_id = spur_core::plan::labels::derive_brain_session_id(&SessionId(acp_id.into()))
            .as_session_id()
            .0
            .clone();
        let mut app = app_in_picker_with_empty_metadata();
        app.handle_spur_event(wrap(SpurEventBody::SessionsListed {
            agent: "claude".into(),
            sessions: vec![session(acp_id, "Build fix")],
        }));

        app.handle_spur_event(user_message(&spur_id, "late synopsis needle"));
        type_picker_search(&mut app, "needle");

        let picker = app.session_picker_for_test().expect("picker open");
        assert_eq!(
            picker.visible_session_count(app.synopsis()),
            1,
            "filter should see synopsis content applied after SessionsListed"
        );
    }

    #[test]
    fn picker_filter_picks_up_rename_without_refresh() {
        let mut app = app_in_picker_with_empty_metadata();
        app.handle_spur_event(wrap(SpurEventBody::SessionsListed {
            agent: "claude".into(),
            sessions: vec![session("S1", "Old title")],
        }));

        app.process_action(Action::RenameSession {
            session_id: "S1".into(),
            new_title: "renamed recall needle".into(),
            original_title: "Old title".into(),
        });
        type_picker_search(&mut app, "needle");

        let picker = app.session_picker_for_test().expect("picker open");
        assert_eq!(
            picker.visible_session_count(app.synopsis()),
            1,
            "filter should see title_override applied after SessionsListed"
        );
    }
}

#[cfg(test)]
mod brain_switch_status_tests {
    //! P0 audit fixes: optimistic Connecting from `/brain` must clear on
    //! noop/error; BrainSwitch retire must not wipe the session view.
    use super::super::super::*;
    use spur_acp::domain::events::{BrainRetireReason, SpurEvent, SpurEventBody};
    use spur_acp::SessionId;

    fn wrap(body: SpurEventBody) -> SpurEvent {
        SpurEvent::now(body)
    }

    #[test]
    fn brain_switch_noop_clears_optimistic_connecting() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "grok".into(),
            session: SessionId("s1".into()),
        }));
        app.brain_status = BrainStatus::Connecting;

        app.handle_spur_event(wrap(SpurEventBody::BrainSwitchNoop {
            name: "grok".into(),
        }));

        assert_eq!(
            app.brain_status,
            BrainStatus::Ready,
            "noop must clear optimistic Connecting when a session detail exists"
        );
    }

    #[test]
    fn brain_switch_error_clears_optimistic_connecting() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "grok".into(),
            session: SessionId("s1".into()),
        }));
        app.brain_status = BrainStatus::Connecting;

        app.handle_spur_event(wrap(SpurEventBody::BrainSwitchError {
            name: "nope".into(),
            available: vec!["grok".into(), "opencode".into()],
        }));

        assert_eq!(
            app.brain_status,
            BrainStatus::Ready,
            "error must clear optimistic Connecting when a session detail exists"
        );
    }

    #[test]
    fn brain_switch_error_without_session_goes_idle() {
        let mut app = App::new_for_tests();
        app.brain_status = BrainStatus::Connecting;

        app.handle_spur_event(wrap(SpurEventBody::BrainSwitchError {
            name: "nope".into(),
            available: vec![],
        }));

        assert_eq!(app.brain_status, BrainStatus::Idle);
    }

    #[test]
    fn brain_retired_brain_switch_does_not_clear_session_view() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "grok".into(),
            session: SessionId("s1".into()),
        }));

        app.handle_spur_event(wrap(SpurEventBody::BrainRetired {
            session: SessionId("s1".into()),
            reason: BrainRetireReason::BrainSwitch,
        }));

        let detail = app.session_detail.as_ref().expect("view retained");
        assert!(
            !detail.is_cleared(),
            "BrainSwitch must not trigger UserClear view wipe; BrainSpawned rebuilds"
        );
        assert!(app.brain_name.is_none());
    }

    /// Scope A: switching while already on SessionDetail must update
    /// `current_view` to the new session id (not leave a stale ViewId).
    #[test]
    fn brain_spawned_while_on_session_detail_updates_view_id() {
        let mut app = App::new_for_tests();
        let s1 = SessionId("s1".into());
        let s2 = SessionId("s2".into());
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "grok".into(),
            session: s1.clone(),
        }));
        assert_eq!(app.current_view(), &ViewId::SessionDetail(s1.clone()));
        // History after Dashboard → SessionDetail(s1) is [Dashboard].
        let hist_before = app.view_history.clone();

        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "codex".into(),
            session: s2.clone(),
        }));

        assert_eq!(
            app.current_view(),
            &ViewId::SessionDetail(s2.clone()),
            "ViewId must track the new session after hot-swap BrainSpawned"
        );
        let detail = app.session_detail.as_ref().expect("detail present");
        assert_eq!(detail.session_id(), &s2);
        assert_eq!(
            app.view_history, hist_before,
            "must not push retired SessionDetail onto the back stack"
        );
        assert!(
            !app.view_history
                .iter()
                .any(|v| matches!(v, ViewId::SessionDetail(id) if id == &s1)),
            "retired session must not appear in view history"
        );
    }

    #[test]
    fn brain_picker_open_on_session_detail_opens_picker() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "grok".into(),
            session: SessionId("s1".into()),
        }));

        app.handle_spur_event(wrap(SpurEventBody::BrainPickerOpen {
            brains: vec![
                spur_acp::BrainInfo {
                    name: "grok".into(),
                    kind: spur_acp::AgentKind::Grok,
                    is_default: true,
                },
                spur_acp::BrainInfo {
                    name: "codex".into(),
                    kind: spur_acp::AgentKind::CodexAcp,
                    is_default: false,
                },
            ],
            active: "grok".into(),
        }));

        assert!(
            app.session_detail
                .as_ref()
                .is_some_and(|d| d.completion_active()),
            "BrainPickerOpen on SessionDetail must open the fuzzy brain picker"
        );
        assert!(
            app.transient_hint_for_test().is_none(),
            "picker path should not fall back to the brains list flash"
        );
    }

    #[test]
    fn brain_picker_open_on_dashboard_opens_picker() {
        let mut app = App::new_for_tests();
        // Stay on Dashboard (no BrainSpawned navigation).
        app.handle_spur_event(wrap(SpurEventBody::BrainPickerOpen {
            brains: vec![spur_acp::BrainInfo {
                name: "grok".into(),
                kind: spur_acp::AgentKind::Grok,
                is_default: true,
            }],
            active: "grok".into(),
        }));

        assert!(
            app.dashboard_for_test().completion_active(),
            "BrainPickerOpen on Dashboard must open the fuzzy brain picker"
        );
    }

    #[test]
    fn brain_picker_open_on_unwired_view_flashes_status() {
        let mut app = App::new_for_tests();
        app.current_view = ViewId::SessionPicker;

        app.handle_spur_event(wrap(SpurEventBody::BrainPickerOpen {
            brains: vec![spur_acp::BrainInfo {
                name: "grok".into(),
                kind: spur_acp::AgentKind::Grok,
                is_default: true,
            }],
            active: "grok".into(),
        }));

        let hint = app
            .transient_hint_for_test()
            .expect("unwired views should flash brains status");
        assert!(
            hint.text.contains("brains:"),
            "status flash should list brains, got `{}`",
            hint.text
        );
    }

    #[test]
    fn brain_error_flashes_long_hint() {
        let mut app = App::new_for_tests();
        app.handle_spur_event(wrap(SpurEventBody::BrainError {
            session: SessionId("s1".into()),
            message: "spawn failed: binary not found".into(),
        }));

        let hint = app
            .transient_hint_for_test()
            .expect("BrainError must set a transient hint");
        assert!(
            hint.text.contains("spawn failed"),
            "hint should include error message, got `{}`",
            hint.text
        );
        assert!(
            hint.expires_at.duration_since(std::time::Instant::now())
                > std::time::Duration::from_secs(5),
            "spawn errors must stay visible longer than a short flash"
        );
        assert!(matches!(app.brain_status, BrainStatus::Error(_)));
    }

    #[test]
    fn process_brain_cmd_without_tx_surfaces_loud_error() {
        let mut app = App::new_for_tests();
        // new_for_tests has no user_input_tx.
        app.process_action(crate::action::Action::BrainCommand {
            arg: "codex".into(),
        });

        let hint = app
            .transient_hint_for_test()
            .expect("missing channel must flash loudly");
        assert!(
            hint.text.contains("channel") || hint.text.contains("failed"),
            "expected channel-failure hint, got `{}`",
            hint.text
        );
        assert_eq!(
            app.brain_status,
            BrainStatus::Idle,
            "failed named switch must not leave optimistic Connecting"
        );
    }

    fn seed_brain_agents(app: &mut App) {
        // SpurConfig::default() has empty agents.entries; seed like a real
        // project config so local_brain_info_list is non-empty.
        let mut cfg = (*app.config).clone();
        cfg.brain.default = "grok".into();
        let mut grok = spur_acp::AgentConfig::with_defaults("grok");
        grok.kind = spur_acp::AgentKind::Grok;
        grok.role = spur_acp::AgentRole::Both;
        let mut codex = spur_acp::AgentConfig::with_defaults("codex");
        codex.kind = spur_acp::AgentKind::CodexAcp;
        codex.role = spur_acp::AgentRole::Both;
        cfg.agents.entries = vec![grok, codex];
        app.config = std::sync::Arc::new(cfg);
    }

    /// Bare `/brain` must open the picker from local config immediately —
    /// no orchestrator round-trip, no user_input_tx required.
    #[test]
    fn bare_brain_command_opens_local_picker_without_channel() {
        let mut app = App::new_for_tests();
        seed_brain_agents(&mut app);

        app.process_action(crate::action::Action::BrainCommand { arg: String::new() });

        assert!(
            app.dashboard_for_test().completion_active(),
            "bare `/brain` on Dashboard must open the fuzzy picker locally"
        );
        assert!(
            app.transient_hint_for_test().is_none(),
            "local picker path must not flash the brains list"
        );
    }

    #[test]
    fn bare_brain_command_on_session_detail_opens_local_picker() {
        let mut app = App::new_for_tests();
        seed_brain_agents(&mut app);
        app.handle_spur_event(wrap(SpurEventBody::BrainSpawned {
            agent: "grok".into(),
            session: SessionId("s1".into()),
        }));

        app.process_action(crate::action::Action::BrainCommand { arg: String::new() });

        assert!(
            app.session_detail
                .as_ref()
                .is_some_and(|d| d.completion_active()),
            "bare `/brain` on SessionDetail must open the fuzzy picker locally"
        );
    }
}
