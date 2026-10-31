use spur_graph::{build_facts, NodeKind, RelationKind};

#[test]
fn markdown_reference_style_links_normalize_destinations_without_binding() {
    for (form, link, destination) in [
        ("direct", "[Direct](direct.md)", "direct.md"),
        ("full", "[Guide][guide-ref]", "guide.md"),
        ("collapsed", "[guide-ref][]", "guide.md"),
        ("shortcut", "[guide-ref]", "guide.md"),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("index.md"),
            format!(
                "# Overview\n\n{link}\n\nMissing shortcut [no-such-label].\n\n\
                 [Symbol](guide_ref) and [External](https://example.com/guide.md).\n\n\
                 [guide-ref]: guide.md\n"
            ),
        )
        .expect("write index.md");
        std::fs::write(dir.path().join("direct.md"), "# Direct\n").expect("write direct.md");
        std::fs::write(dir.path().join("guide.md"), "# Guide\n").expect("write guide.md");
        std::fs::write(dir.path().join("helper.rs"), "pub fn guide_ref() {}\n")
            .expect("write same-name symbol");

        let facts = build_facts(dir.path(), None).expect("build facts").0;
        let links = facts
            .edges
            .iter()
            .filter(|edge| edge.relation == RelationKind::Links)
            .collect::<Vec<_>>();
        let link_targets = links
            .iter()
            .filter_map(|edge| edge.target_label.as_deref())
            .collect::<Vec<_>>();

        for target in [
            destination,
            "no-such-label",
            "guide_ref",
            "https://example.com/guide.md",
        ] {
            assert!(
                link_targets.contains(&target),
                "{form}: missing Markdown link edge to {target}; targets: {link_targets:?}"
            );
        }
        for label in ["[guide-ref]", "guide-ref"] {
            assert!(
                !link_targets.contains(&label),
                "{form}: reference labels must normalize to destinations: {link_targets:?}"
            );
        }
        assert!(facts
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::File && node.label == destination));
        assert!(facts
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::Function && node.label == "guide_ref"));
        for edge in links {
            assert_eq!(
                edge.target_node_id, None,
                "{form}: URL text must stay unresolved even with a same-name file or symbol: {edge:?}"
            );
        }
        assert!(
            !facts
                .edges
                .iter()
                .any(|edge| edge.relation == RelationKind::Imports),
            "{form}: Markdown @import capture channel must remap to Links, not Imports"
        );
        eprintln!(
            "markdown destination trace: form={form} targets={link_targets:?} all_unresolved=true"
        );
    }
}
