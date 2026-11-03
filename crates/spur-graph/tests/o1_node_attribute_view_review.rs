#![cfg(all(target_os = "macos", feature = "embed"))]

use ort::operator::attribute::Attribute;
use std::{cell::Cell, rc::Rc};

struct ChangingAttributes {
    calls: Rc<Cell<usize>>,
    owned: Vec<Attribute>,
}

impl AsRef<[Attribute]> for ChangingAttributes {
    fn as_ref(&self) -> &[Attribute] {
        let calls = self.calls.get();
        self.calls.set(calls + 1);
        if calls == 0 {
            &[]
        } else {
            &self.owned
        }
    }
}

#[test]
fn managed_node_reuses_the_checked_attribute_view() {
    let cache = tempfile::tempdir().unwrap();
    spur_graph::store::ensure_bundled_ort_runtime_at(cache.path()).unwrap();
    let environment = ort::environment::get_environment().unwrap();
    let calls = Rc::new(Cell::new(0));
    let attributes = ChangingAttributes {
        calls: Rc::clone(&calls),
        owned: vec![Attribute::new("alpha", 1.0_f32).unwrap()],
    };

    // Stop before native CreateNode can consume a borrowed Attribute. This
    // test observes the actual production guard and conversion calls without
    // invoking the dangerous transfer on the unfixed path.
    ort::owner_test_hooks::fail_next_editor_node();
    let result = ort::editor::Node::new(
        "LeakyRelu",
        "",
        "review-node",
        ["input"],
        ["output"],
        attributes,
    );
    ort::owner_test_hooks::clear_failpoint();
    assert!(result.is_err(), "native transfer must be intercepted");
    drop(result);
    drop(environment);
    ort::environment::try_shutdown().unwrap();

    assert_eq!(
        calls.get(),
        1,
        "the attribute slice checked by the managed guard must also be the slice passed to native code"
    );
}
