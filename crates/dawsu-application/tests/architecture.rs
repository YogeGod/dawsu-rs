use dawsu_test_support::workspace_root;

#[test]
fn new_modules_keep_the_vaults_reference_boundaries() {
    let cases: [(&str, &[&str]); 4] = [
        ("crates/dawsu-audio", &["dawsu-foundation"]),
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
            "crates/dawsu-application",
            &[
                "dawsu-foundation",
                "dawsu-osu-format",
                "dawsu-hitsounding",
                "dawsu-editor",
                "dawsu-editor-composer",
                "dawsu-audio",
                "dawsu-graph-audio",
            ],
        ),
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
    for (name, allowed) in cases {
        let text = std::fs::read_to_string(
            workspace_root()
                .join(name)
                .join("Cargo.toml"),
        )
        .unwrap();
        let dependencies = text
            .split("[dependencies]")
            .nth(1)
            .unwrap()
            .split("[dev-dependencies]")
            .next()
            .unwrap();
        for line in dependencies.lines() {
            let name = line.split('=').next().unwrap_or("").trim();
            if name.starts_with("dawsu-") {
                assert!(allowed.contains(&name), "forbidden reference in {name}");
            }
        }
    }
}
