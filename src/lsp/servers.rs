//! Which server each language uses: the spec's defaults, with `[lsp.<lang>]`
//! tables laid over them, and the PATH lookup behind `griffin --health`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::config::LspServer;

/// Every language with a default server, in the order `--health` lists them.
/// The ids are the highlight registry's names (plus `jsx`, which shares the
/// JavaScript grammar but has its own server key), so one id drives both.
pub const LANGUAGES: [&str; 10] = [
    "rust",
    "go",
    "typescript",
    "tsx",
    "javascript",
    "jsx",
    "python",
    "html",
    "css",
    "sql",
];

/// The spec's default command and args for `lang`.
pub fn default_for(lang: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match lang {
        "rust" => ("rust-analyzer", &[]),
        "go" => ("gopls", &[]),
        "typescript" | "tsx" | "javascript" | "jsx" => ("typescript-language-server", &["--stdio"]),
        "python" => ("pyright-langserver", &["--stdio"]),
        "html" => ("vscode-html-language-server", &["--stdio"]),
        "css" => ("vscode-css-language-server", &["--stdio"]),
        "sql" => ("sqls", &[]),
        _ => return None,
    })
}

/// The server table for every language: the defaults, overridden by `config`.
/// A table naming a command replaces the default command and args; one without
/// a command keeps the default program, so `format_on_save = true` alone doesn't
/// switch the server off. Tables for languages without a default pass through.
pub fn with_defaults(mut config: BTreeMap<String, LspServer>) -> BTreeMap<String, LspServer> {
    for lang in LANGUAGES {
        let Some((command, args)) = default_for(lang) else {
            continue;
        };
        let server = config.entry(lang.to_string()).or_default();
        if server.command.is_none() {
            server.command = Some(command.to_string());
            if server.args.is_empty() {
                server.args = args.iter().map(|a| a.to_string()).collect();
            }
        }
    }
    config
}

/// One line per language for `griffin --health`: the language padded to 11, the
/// command line, and whether the program was found. `path` and `pathext` are the
/// `PATH` and `PATHEXT` values to search.
pub fn health(
    servers: &BTreeMap<String, LspServer>,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> String {
    let mut out = String::new();
    for lang in LANGUAGES {
        let Some(server) = servers.get(lang) else {
            continue;
        };
        let Some(command) = server.command.as_deref() else {
            continue;
        };
        let line = std::iter::once(command)
            .chain(server.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        let found = if find(command, path, pathext).is_some() {
            "found"
        } else {
            "missing"
        };
        out.push_str(&format!("{lang:<11} {line}  {found}\n"));
    }
    out
}

/// Where `command` would be run from: the path itself when it names a directory,
/// otherwise the first match in `path`. On Windows a name without an extension
/// also matches each `PATHEXT` extension, as the shell does.
pub fn find(command: &str, path: Option<&OsStr>, pathext: Option<&OsStr>) -> Option<PathBuf> {
    let program = Path::new(command);
    let exts = extensions(pathext);
    if program.components().count() > 1 {
        return candidates(program, &exts).find(|p| is_program(p));
    }
    std::env::split_paths(path?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| candidates(&dir.join(program), &exts).collect::<Vec<_>>())
        .find(|p| is_program(p))
}

/// The extensions to try after the bare name. Only Windows has them.
fn extensions(pathext: Option<&OsStr>) -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let list = pathext
        .and_then(OsStr::to_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(".COM;.EXE;.BAT;.CMD");
    list.split(';')
        .map(str::trim)
        .filter(|e| e.starts_with('.') && e.len() > 1)
        // Windows file names ignore case; lower case reads as the files usually do.
        .map(str::to_ascii_lowercase)
        .collect()
}

fn candidates<'a>(base: &'a Path, exts: &'a [String]) -> impl Iterator<Item = PathBuf> + 'a {
    // A name that already has an extension ("foo.cmd") is taken as is first.
    std::iter::once(base.to_path_buf()).chain(exts.iter().map(move |ext| {
        let mut name = base.as_os_str().to_os_string();
        name.push(ext);
        PathBuf::from(name)
    }))
}

#[cfg(unix)]
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_program(path: &Path) -> bool {
    // Windows runs a bare name only through an extension, so the extensionless
    // file a Unix-style shim leaves beside `foo.cmd` doesn't count.
    path.is_file() && path.extension().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::languages;

    fn server(command: Option<&str>, args: &[&str]) -> LspServer {
        LspServer {
            command: command.map(str::to_string),
            args: args.iter().map(|a| a.to_string()).collect(),
            ..LspServer::default()
        }
    }

    fn command_line(servers: &BTreeMap<String, LspServer>, lang: &str) -> (String, Vec<String>) {
        let s = &servers[lang];
        (s.command.clone().unwrap_or_default(), s.args.clone())
    }

    #[test]
    fn with_no_config_every_language_gets_the_spec_default() {
        let servers = with_defaults(BTreeMap::new());
        let expect = [
            ("rust", "rust-analyzer", vec![]),
            ("go", "gopls", vec![]),
            ("typescript", "typescript-language-server", vec!["--stdio"]),
            ("tsx", "typescript-language-server", vec!["--stdio"]),
            ("javascript", "typescript-language-server", vec!["--stdio"]),
            ("jsx", "typescript-language-server", vec!["--stdio"]),
            ("python", "pyright-langserver", vec!["--stdio"]),
            ("html", "vscode-html-language-server", vec!["--stdio"]),
            ("css", "vscode-css-language-server", vec!["--stdio"]),
            ("sql", "sqls", vec![]),
        ];
        assert_eq!(servers.len(), expect.len());
        for (lang, command, args) in expect {
            assert_eq!(
                command_line(&servers, lang),
                (
                    command.to_string(),
                    args.iter().map(|a| a.to_string()).collect()
                ),
                "{lang}"
            );
            assert!(!servers[lang].format_on_save, "{lang}");
        }
    }

    #[test]
    fn config_overrides_each_default() {
        for lang in LANGUAGES {
            let config = BTreeMap::from([(lang.to_string(), server(Some("my-server"), &["-x"]))]);
            let servers = with_defaults(config);
            assert_eq!(
                command_line(&servers, lang),
                ("my-server".to_string(), vec!["-x".to_string()]),
                "{lang}"
            );
            // The others keep their defaults.
            let other = if lang == "rust" { "go" } else { "rust" };
            assert_eq!(
                Some(command_line(&servers, other).0.as_str()),
                default_for(other).map(|d| d.0)
            );
        }
    }

    #[test]
    fn a_command_without_args_drops_the_default_args() {
        let config = BTreeMap::from([("python".to_string(), server(Some("pylsp"), &[]))]);
        assert_eq!(
            command_line(&with_defaults(config), "python"),
            ("pylsp".to_string(), vec![])
        );
    }

    #[test]
    fn a_table_without_a_command_keeps_the_default_program() {
        let mut only_format = server(None, &[]);
        only_format.format_on_save = true;
        let config = BTreeMap::from([
            ("python".to_string(), only_format),
            ("go".to_string(), server(None, &["serve"])),
            ("zig".to_string(), server(Some("zls"), &[])),
        ]);
        let servers = with_defaults(config);
        assert_eq!(
            command_line(&servers, "python"),
            (
                "pyright-langserver".to_string(),
                vec!["--stdio".to_string()]
            )
        );
        assert!(servers["python"].format_on_save);
        assert_eq!(
            command_line(&servers, "go"),
            ("gopls".to_string(), vec!["serve".to_string()])
        );
        assert_eq!(servers["zig"].command.as_deref(), Some("zls"));
    }

    #[test]
    fn language_ids_match_the_highlight_registry() {
        for lang in LANGUAGES {
            let registry = if lang == "jsx" { "javascript" } else { lang };
            assert!(languages::for_name(registry).is_some(), "{lang}");
            assert!(default_for(lang).is_some(), "{lang}");
        }
    }

    fn touch(path: &Path) {
        std::fs::write(path, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn health_lists_every_language_in_order_with_what_was_found() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "gopls.exe" } else { "gopls" };
        touch(&dir.path().join(name));
        let report = health(
            &with_defaults(BTreeMap::new()),
            Some(dir.path().as_os_str()),
            None,
        );
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "rust        rust-analyzer  missing");
        assert_eq!(lines[1], "go          gopls  found");
        assert_eq!(
            lines[3],
            "tsx         typescript-language-server --stdio  missing"
        );
    }

    #[test]
    fn find_searches_each_path_entry_in_order() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "srv.exe" } else { "srv" };
        touch(&b.path().join(name));
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert_eq!(find("srv", Some(&path), None), Some(b.path().join(name)));
        assert_eq!(find("nope", Some(&path), None), None);
        assert_eq!(find("srv", None, None), None);
        // A command given as a path is checked where it points.
        let full = b.path().join(name);
        assert_eq!(find(full.to_str().unwrap(), None, None), Some(full.clone()));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_is_not_executable_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("srv"), "").unwrap();
        assert_eq!(find("srv", Some(dir.path().as_os_str()), None), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_lookup_honors_pathext() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("srv.cmd"));
        touch(&dir.path().join("tool.bat"));
        let path = Some(dir.path().as_os_str());
        assert_eq!(
            find("srv", path, Some(OsStr::new(".COM;.EXE;.BAT;.CMD"))),
            Some(dir.path().join("srv.cmd"))
        );
        // The default list applies when PATHEXT is unset.
        assert_eq!(find("tool", path, None), Some(dir.path().join("tool.bat")));
        // A PATHEXT without .CMD doesn't find a .cmd.
        assert_eq!(find("srv", path, Some(OsStr::new(".EXE"))), None);
        // An extensionless file alone isn't runnable on Windows.
        touch(&dir.path().join("bare"));
        assert_eq!(find("bare", path, None), None);
    }
}
