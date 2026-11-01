use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use spur_acp::{ContentBlock, IssueSummaryEvent, SessionId, SpurEvent, SpurEventBody};
use spur_core::plan::labels::derive_brain_session_id;
use spur_tui::action::{Action, ViewId};
use spur_tui::{app::App, UserInput};
use tokio::sync::mpsc;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn summary(id: &str, title: &str, issue_type: Option<&str>) -> IssueSummaryEvent {
    IssueSummaryEvent {
        id: id.into(),
        source: "beads".into(),
        title: title.into(),
        status: "open".into(),
        labels: Vec::new(),
        priority: Some(1),
        issue_type: issue_type.map(String::from),
        assignee: None,
        description: None,
    }
}

fn text_from_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

fn render_text(app: &mut App, terminal: &mut Terminal<TestBackend>) -> String {
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let mut rendered = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            rendered.push_str(buf[(x, y)].symbol());
        }
        rendered.push('\n');
    }
    rendered
}

fn seeded_issue_browser_app(
    issues: Vec<IssueSummaryEvent>,
) -> (App, mpsc::Receiver<UserInput>, Terminal<TestBackend>) {
    let (mut app, mut rx) = spur_tui::test_support::app_with_user_input_tx();
    spur_tui::test_support::process_action(&mut app, Action::NavigateTo(ViewId::IssueBrowser));
    match rx.try_recv() {
        Ok(UserInput::RefreshIssues) => {}
        Ok(_) => panic!("expected first IssueBrowser navigation to request RefreshIssues"),
        Err(err) => panic!("expected RefreshIssues after first navigation, got {err}"),
    }
    spur_tui::test_support::push_event(
        &mut app,
        SpurEvent::now(SpurEventBody::IssuesLoaded { issues }),
    );
    let terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    (app, rx, terminal)
}

#[test]
fn e_on_epic_opens_modal_and_enter_dispatches_active_session_prompt() {
    let (mut app, mut rx, mut terminal) =
        seeded_issue_browser_app(vec![summary("bd-epic", "Launch migration", Some("epic"))]);
    spur_tui::test_support::process_action(
        &mut app,
        Action::ResumeSession {
            session_id: "active-session".into(),
        },
    );
    match rx.try_recv() {
        Ok(UserInput::ResumeSession { session_id }) => assert_eq!(session_id, "active-session"),
        Ok(_) => panic!("expected ResumeSession after test setup"),
        Err(err) => panic!("expected ResumeSession after test setup, got {err}"),
    }
    spur_tui::test_support::process_action(&mut app, Action::NavigateTo(ViewId::IssueBrowser));

    let initial = render_text(&mut app, &mut terminal);
    assert!(initial.contains("e: Execute Item"), "rendered:\n{initial}");

    app.handle_crossterm_event_for_test(key('e'));
    let modal = render_text(&mut app, &mut terminal);
    assert!(modal.contains("Execute Item"), "rendered:\n{modal}");
    assert!(modal.contains("bd-epic"), "rendered:\n{modal}");
    assert!(modal.contains("Launch migration"), "rendered:\n{modal}");
    assert!(
        rx.try_recv().is_err(),
        "opening the modal must not dispatch to the backend"
    );

    app.handle_crossterm_event_for_test(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match rx.try_recv() {
        Ok(UserInput::Message {
            session,
            blocks,
            interrupt,
        }) => {
            assert_eq!(
                session,
                derive_brain_session_id(&SessionId("active-session".into())).into_session_id()
            );
            assert!(!interrupt);
            let prompt = text_from_blocks(&blocks);
            assert!(
                prompt.contains("The user wants to execute this work item."),
                "prompt:\n{prompt}"
            );
            assert!(
                prompt.contains("Item: bd-epic — Launch migration"),
                "prompt:\n{prompt}"
            );
            assert!(
                prompt.contains("Type: epic | Status: open | Priority: P1"),
                "prompt:\n{prompt}"
            );
            assert!(
                prompt.contains("Please analyze this item, gather necessary information, and determine how to execute it."),
                "prompt:\n{prompt}"
            );
        }
        Ok(_) => panic!("expected Message after confirming execute modal"),
        Err(err) => panic!("expected Message after confirming execute modal, got {err}"),
    }
}

#[test]
fn e_on_non_epic_opens_modal_and_enter_dispatches_active_session_prompt() {
    let (mut app, mut rx, mut terminal) =
        seeded_issue_browser_app(vec![summary("bd-task", "Task item", Some("task"))]);

    spur_tui::test_support::process_action(
        &mut app,
        Action::ResumeSession {
            session_id: "active-session".into(),
        },
    );
    match rx.try_recv() {
        Ok(UserInput::ResumeSession { session_id }) => assert_eq!(session_id, "active-session"),
        Ok(_) => panic!("expected ResumeSession after test setup"),
        Err(err) => panic!("expected ResumeSession after test setup, got {err}"),
    }
    spur_tui::test_support::process_action(&mut app, Action::NavigateTo(ViewId::IssueBrowser));

    let initial = render_text(&mut app, &mut terminal);
    assert!(initial.contains("e: Execute Item"), "rendered:\n{initial}");

    app.handle_crossterm_event_for_test(key('e'));
    let modal = render_text(&mut app, &mut terminal);
    assert!(modal.contains("Execute Item"), "rendered:\n{modal}");

    app.handle_crossterm_event_for_test(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match rx.try_recv() {
        Ok(UserInput::Message { blocks, .. }) => {
            let prompt = text_from_blocks(&blocks);
            assert!(
                prompt.contains("The user wants to execute this work item."),
                "prompt:\n{prompt}"
            );
            assert!(
                prompt.contains("Item: bd-task — Task item"),
                "prompt:\n{prompt}"
            );
            assert!(
                prompt.contains("Type: task | Status: open | Priority: P1"),
                "prompt:\n{prompt}"
            );
        }
        Ok(_) => panic!("expected Message after confirming execute modal"),
        Err(err) => panic!("expected Message after confirming execute modal, got {err}"),
    }
}

#[test]
fn e_in_execute_modal_prefills_active_session_prompt_without_dispatch() {
    let (mut app, mut rx, mut terminal) =
        seeded_issue_browser_app(vec![summary("bd-epic", "Launch migration", Some("epic"))]);
    spur_tui::test_support::process_action(
        &mut app,
        Action::ResumeSession {
            session_id: "active-session".into(),
        },
    );
    match rx.try_recv() {
        Ok(UserInput::ResumeSession { session_id }) => assert_eq!(session_id, "active-session"),
        Ok(_) => panic!("expected ResumeSession after test setup"),
        Err(err) => panic!("expected ResumeSession after test setup, got {err}"),
    }
    spur_tui::test_support::process_action(&mut app, Action::NavigateTo(ViewId::IssueBrowser));

    app.handle_crossterm_event_for_test(key('e'));
    let modal = render_text(&mut app, &mut terminal);
    assert!(
        modal.contains("[e] Edit in input bar"),
        "rendered:\n{modal}"
    );

    app.handle_crossterm_event_for_test(key('e'));

    assert_eq!(
        app.current_view(),
        &ViewId::SessionDetail(
            derive_brain_session_id(&SessionId("active-session".into())).into_session_id()
        )
    );
    let detail =
        spur_tui::test_support::session_detail(&app).expect("session detail should be active");
    let prompt = detail.input_bar_text_for_test();
    assert!(
        prompt.contains("The user wants to execute this work item."),
        "prompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Item: bd-epic — Launch migration"),
        "prompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Type: epic | Status: open | Priority: P1"),
        "prompt:\n{prompt}"
    );
    assert_eq!(
        app.transient_hint_text(),
        Some("Prompt loaded — review and press Enter to send")
    );
    assert!(
        rx.try_recv().is_err(),
        "editing the prompt must not dispatch to the backend"
    );
}

#[test]
fn e_in_execute_modal_prefills_dashboard_prompt_without_session() {
    let (mut app, mut rx, mut terminal) =
        seeded_issue_browser_app(vec![summary("bd-task", "Task item", Some("task"))]);

    app.handle_crossterm_event_for_test(key('e'));
    let modal = render_text(&mut app, &mut terminal);
    assert!(
        modal.contains("[e] Edit in input bar"),
        "rendered:\n{modal}"
    );

    app.handle_crossterm_event_for_test(key('e'));

    assert_eq!(app.current_view(), &ViewId::Dashboard);
    let prompt = app.dashboard_mut_for_test().input_bar_text_for_test();
    assert!(
        prompt.contains("The user wants to execute this work item."),
        "prompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Item: bd-task — Task item"),
        "prompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Type: task | Status: open | Priority: P1"),
        "prompt:\n{prompt}"
    );
    assert_eq!(
        app.transient_hint_text(),
        Some("Prompt loaded — review and press Enter to send")
    );
    assert!(
        rx.try_recv().is_err(),
        "editing the prompt must not dispatch to the backend"
    );
}

#[test]
fn esc_in_execute_modal_dismisses_without_dispatch() {
    let (mut app, mut rx, mut terminal) =
        seeded_issue_browser_app(vec![summary("bd-epic", "Launch migration", Some("epic"))]);

    app.handle_crossterm_event_for_test(key('e'));
    let modal = render_text(&mut app, &mut terminal);
    assert!(modal.contains("Execute Item"), "rendered:\n{modal}");

    app.handle_crossterm_event_for_test(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let dismissed = render_text(&mut app, &mut terminal);
    assert!(
        !dismissed.contains("This sends a prompt asking the brain to analyze"),
        "modal should be dismissed:\n{dismissed}"
    );
    assert!(
        rx.try_recv().is_err(),
        "Esc in execute modal must not dispatch"
    );
}
