use dawsu_test_support::workspace_root;

#[test]
fn new_modules_keep_the_vaults_reference_boundaries() {
    let cases: [(&str, &[&str]); 11] = [
        // Layer 1 · Foundation
        ("crates/dawsu-foundation", &[]),
        // Layer 2 · osu! adapters
        ("crates/dawsu-osu-format", &["dawsu-foundation"]),
        // Layer 3 · Domain
        ("crates/dawsu-hitsounding", &["dawsu-foundation", "dawsu-osu-format"]),
        // Layer 4 · Engines
        ("crates/dawsu-audio", &["dawsu-foundation"]),
        ("crates/dawsu-graph", &["dawsu-foundation"]),
        ("crates/dawsu-graph-audio", &["dawsu-foundation", "dawsu-graph"]),
        // Layer 5 · Editing
        ("crates/dawsu-editor", &["dawsu-foundation"]),
        (
            "crates/dawsu-editor-composer",
            &[
                "dawsu-foundation",
                "dawsu-editor",
                "dawsu-hitsounding",
                "dawsu-keysounding",
            ],
        ),
        (
            "crates/dawsu-editor-nodal",
            &[
                "dawsu-foundation",
                "dawsu-editor",
                "dawsu-graph",
                "dawsu-hitsounding",
                "dawsu-editor-composer",
            ],
        ),
        // Layer 6 · Use cases
        (
            "crates/dawsu-application",
            &[
                "dawsu-foundation",
                "dawsu-osu-format",
                "dawsu-hitsounding",
                "dawsu-keysounding",
                "dawsu-editor",
                "dawsu-editor-composer",
                "dawsu-audio",
                "dawsu-audio-instruments",
                "dawsu-graph",
                "dawsu-graph-audio",
                "dawsu-editor-nodal",
            ],
        ),
        // Layer 7 · UI
        (
            "apps/tauri",
            &[
                "dawsu-foundation",
                "dawsu-editor",
                "dawsu-application",
                "dawsu-hitsounding",
            ],
        ),
    ];
    for (crate_path, allowed) in cases {
        let text = std::fs::read_to_string(
            workspace_root()
                .join(crate_path)
                .join("Cargo.toml"),
        )
        .unwrap();
        let dependencies = text
            .split("[dependencies]")
            .nth(1)
            .unwrap_or("")
            .split("\n[")
            .next()
            .unwrap_or("");
        for line in dependencies.lines() {
            let dep = line.split('=').next().unwrap_or("").trim();
            if dep.starts_with("dawsu-") {
                assert!(
                    allowed.contains(&dep),
                    "forbidden reference '{dep}' in {crate_path}"
                );
            }
        }
    }
}
