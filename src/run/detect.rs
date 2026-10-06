//! Commands F5 offers when `.griffin.toml` has no `[[run]]`: guessed from the
//! project files at the root, never written back anywhere.

use std::path::Path;

use crate::config::RunEntry;

/// What F5 lists: the configured entries when there are any, otherwise whatever
/// the project files at `root` suggest. Configured entries win outright so a
/// project that set up `[[run]]` never sees guesses mixed in.
pub fn run_choices(configured: Vec<RunEntry>, root: &Path) -> Vec<RunEntry> {
    if configured.is_empty() {
        detect(root)
    } else {
        configured
    }
}

/// Commands guessed from `package.json`, `Cargo.toml` and `go.mod` at `root`, in
/// that order. Each entry is named after its command, since that's what a user
/// would type to run it by hand.
pub fn detect(root: &Path) -> Vec<RunEntry> {
    let mut commands = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        let pm = package_manager(root);
        commands.extend(
            package_scripts(&text)
                .into_iter()
                .map(|script| format!("{pm} run {script}")),
        );
    }
    if root.join("Cargo.toml").is_file() {
        commands.push("cargo run".to_string());
    }
    if root.join("go.mod").is_file() {
        commands.push("go run .".to_string());
    }
    commands
        .into_iter()
        .map(|command| RunEntry {
            name: command.clone(),
            command,
            cwd: None,
        })
        .collect()
}

/// The package manager whose lockfile sits at `root`; npm when there is none.
fn package_manager(root: &Path) -> &'static str {
    let has = |name: &str| root.join(name).is_file();
    if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("yarn.lock") {
        "yarn"
    } else if has("bun.lockb") || has("bun.lock") {
        "bun"
    } else {
        "npm"
    }
}

/// Script names from `package.json`, sorted. A file that doesn't parse, or has no
/// `scripts` object, offers nothing rather than an error: detection is a hint,
/// and F5 says how to add `[[run]]` when it comes up empty.
fn package_scripts(text: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    scripts
        .iter()
        .filter(|(_, command)| command.is_string())
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/detect")
            .join(name)
    }

    fn commands(entries: &[RunEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.command.as_str()).collect()
    }

    #[test]
    fn package_scripts_use_the_lockfiles_package_manager() {
        for (dir, pm) in [
            ("npm", "npm"),
            ("pnpm", "pnpm"),
            ("yarn", "yarn"),
            ("bun", "bun"),
            ("bun-text", "bun"),
        ] {
            let found = detect(&fixture(dir));
            assert_eq!(
                commands(&found),
                [format!("{pm} run build"), format!("{pm} run dev")],
                "{dir}"
            );
            assert!(found.iter().all(|e| e.name == e.command && e.cwd.is_none()));
        }
    }

    #[test]
    fn cargo_and_go_projects_offer_their_run_commands() {
        assert_eq!(commands(&detect(&fixture("cargo"))), ["cargo run"]);
        assert_eq!(commands(&detect(&fixture("go"))), ["go run ."]);
    }

    #[test]
    fn an_empty_folder_offers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).is_empty());
    }

    #[test]
    fn a_broken_package_json_offers_no_scripts() {
        assert!(package_scripts("{ not json").is_empty());
        assert!(package_scripts(r#"{"name": "x"}"#).is_empty());
        assert_eq!(package_scripts(r#"{"scripts": {"a": "x", "b": 1}}"#), ["a"]);
    }

    #[test]
    fn detected_commands_are_offered_only_without_run_entries() {
        let root = fixture("mixed");
        assert_eq!(
            commands(&run_choices(Vec::new(), &root)),
            ["npm run test", "cargo run", "go run ."]
        );
        // The fixture's own `.griffin.toml` has an entry, so only it is listed.
        let loaded = config::load_project(&root);
        assert_eq!(loaded.error, None);
        let choices = run_choices(loaded.config.run, &root);
        assert_eq!(commands(&choices), ["echo serve"]);
        assert_eq!(choices[0].name, "serve");
    }
}
