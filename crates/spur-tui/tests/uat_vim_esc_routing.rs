mod common;

use crossterm::event::KeyCode;
use spur_acp::{config::ConfigPatch, SpurEvent, SpurEventBody};
use spur_tui::action::Action;
use spur_tui::components::input_bar::{EditMode, VimMode};
use spur_tui::UserInput;

use common::TestHarness;

#[test]
fn vim_insert_esc_routes_to_input_bar_before_global_handlers() {
    let mut h = TestHarness::new(80, 24);

    spur_tui::test_support::process_action(h.app_mut(), Action::ToggleVimMode);
    assert!(h.take_actions().into_iter().any(|input| matches!(
        input,
        UserInput::UpdateConfig {
            patch: ConfigPatch::TuiEditMode(_)
        }
    )));
    assert_eq!(
        h.app_mut()
            .dashboard_mut_for_test()
            .input_bar_mut_for_test()
            .mode(),
        EditMode::Emacs,
        "pending persistence must not change the live editor mode"
    );
    spur_tui::test_support::push_event(
        h.app_mut(),
        SpurEvent::now(SpurEventBody::ConfigUpdateResult {
            section: "tui".into(),
            ok: true,
            message: "saved".into(),
        }),
    );
    h.send_key(KeyCode::Char('i'));
    assert_eq!(
        h.app_mut()
            .dashboard_mut_for_test()
            .input_bar_mut_for_test()
            .mode(),
        EditMode::Vim(VimMode::Insert),
        "setup should put dashboard input bar in Vim Insert mode"
    );

    h.send_key(KeyCode::Esc);

    assert_eq!(
        h.app_mut()
            .dashboard_mut_for_test()
            .input_bar_mut_for_test()
            .mode(),
        EditMode::Vim(VimMode::Normal),
        "Esc in Vim Insert mode should transition to Normal instead of being swallowed globally"
    );
    assert_eq!(
        h.app_mut()
            .dashboard_mut_for_test()
            .input_bar_text_for_test(),
        ""
    );
}
