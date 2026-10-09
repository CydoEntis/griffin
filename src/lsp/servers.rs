//! Which server each language uses: the spec's defaults, with `[lsp.<lang>]`
//! tables laid over them, and the PATH lookup behind `tome --health`.

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

/// The spec's default install command for `lang`'s server and the tool that
/// command runs, which has to be on PATH for the install to work.
pub fn default_install(lang: &str) -> Option<(&'static str, &'static str)> {
    Some(match lang {
        "rust" => ("rustup component add rust-analyzer", "rustup"),
        "go" => ("go install golang.org/x/tools/gopls@latest", "go"),
        "typescript" | "tsx" | "javascript" | "jsx" => (
            "npm install -g typescript-language-server typescript",
            "npm",
        ),
        "python" => ("npm install -g pyright", "npm"),
        "html" | "css" => ("npm install -g vscode-langservers-extracted", "npm"),
        "sql" => ("go install github.com/sqls-server/sqls@latest", "go"),
        _ => return None,
    })
}

/// The server table for every language: the defaults, overridden by `config`.
/// A table naming a command replaces the default command and args; one without
/// a command keeps the default program, so `format_on_save = true` alone doesn't
/// switch the server off. Tables for languages without a default pass through.
///
/// The default install command comes with the default program only: it installs
/// that program, so a table naming another server gets none unless it sets
/// `install` itself.
pub fn with_defaults(mut config: BTreeMap<String, LspServer>) -> BTreeMap<String, LspServer> {
    for lang in LANGUAGES {
        let Some((command, args)) = default_for(lang) else {
            continue;
        };
        let server = config.entry(lang.to_string()).or_default();
        let default_program = server.command.as_deref().is_none_or(|c| c == command);
        if server.install.is_none() && default_program {
            server.install = default_install(lang).map(|(install, _)| install.to_string());
        }
        if server.command.is_none() {
            server.command = Some(command.to_string());
            if server.args.is_empty() {
                server.args = args.iter().map(|a| a.to_string()).collect();
            }
        }
    }
    config
}

/// One line per language for `tome --health`: the language padded to 11, the
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

/// Whether `install` has to be run by the user rather than by Tome: the run
/// panel has no stdin, so a `sudo` password prompt would hang it.
pub fn copy_only(install: &str) -> bool {
    install.split_whitespace().next() == Some("sudo")
}

/// The program an install command runs, which is what has to be on PATH for it
/// to work: its first word, after a leading `sudo`.
pub fn install_tool(install: &str) -> Option<&str> {
    let mut words = install.split_whitespace();
    let first = words.next()?;
    if first == "sudo" {
        words.next()
    } else {
        Some(first)
    }
}

/// What the catalog knows about one server: whether it can start now, and what
/// would install it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallState {
    /// The server's command is on PATH (or exists where its path points).
    pub command_found: bool,
    /// The program the install command runs, if there is an install command.
    pub tool: Option<String>,
    /// That program is on PATH.
    pub tool_found: bool,
    /// The command that installs the server, if one is known.
    pub install: Option<String>,
    /// The install command must be copied and run by the user (see `copy_only`).
    pub copy_only: bool,
}

/// The install state of `server`, looking programs up in `path` and `pathext`
/// as `find` does.
pub fn install_state(
    server: &LspServer,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> InstallState {
    let command_found = server
        .command
        .as_deref()
        .is_some_and(|c| find(c, path, pathext).is_some());
    // A blank `install = ""` reads as "no install command", not a command to run.
    let install = server.install.clone().filter(|i| !i.trim().is_empty());
    let tool = install
        .as_deref()
        .and_then(install_tool)
        .map(str::to_string);
    let tool_found = tool
        .as_deref()
        .is_some_and(|t| find(t, path, pathext).is_some());
    let copy_only = install.as_deref().is_some_and(copy_only);
    InstallState {
        command_found,
        tool,
        tool_found,
        install,
        copy_only,
    }
}

/// What to start for `command`: where the `--health` lookup finds it in this
/// process's `PATH`/`PATHEXT`, so a `.cmd` shim starts on Windows, where spawning
/// the bare name only tries `.exe`. A relative path with directories is left as
/// is, since the server runs in the project root and resolves it from there; so
/// is a command found nowhere, so starting it fails as "not found".
pub fn program(command: &str) -> PathBuf {
    let path = Path::new(command);
    if path.components().count() > 1 && !path.is_absolute() {
        return path.to_path_buf();
    }
    let search = std::env::var_os("PATH");
    let pathext = std::env::var_os("PATHEXT");
    find(command, search.as_deref(), pathext.as_deref()).unwrap_or_else(|| path.to_path_buf())
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

    #[test]
    fn every_language_gets_the_spec_install_command_and_tool() {
        let servers = with_defaults(BTreeMap::new());
        let ts = "npm install -g typescript-language-server typescript";
        let html = "npm install -g vscode-langservers-extracted";
        let expect = [
            ("rust", "rustup component add rust-analyzer", "rustup"),
            ("go", "go install golang.org/x/tools/gopls@latest", "go"),
            ("typescript", ts, "npm"),
            ("tsx", ts, "npm"),
            ("javascript", ts, "npm"),
            ("jsx", ts, "npm"),
            ("python", "npm install -g pyright", "npm"),
            ("html", html, "npm"),
            ("css", html, "npm"),
            ("sql", "go install github.com/sqls-server/sqls@latest", "go"),
        ];
        assert_eq!(expect.len(), LANGUAGES.len());
        for (lang, install, tool) in expect {
            assert_eq!(default_install(lang), Some((install, tool)), "{lang}");
            assert_eq!(servers[lang].install.as_deref(), Some(install), "{lang}");
            assert_eq!(install_tool(install), Some(tool), "{lang}");
        }
        assert_eq!(default_install("zig"), None);
    }

    #[test]
    fn a_configured_install_replaces_the_default() {
        let mut custom = server(None, &[]);
        custom.install = Some("pip install pyright".into());
        let mut custom_server = server(Some("pylsp"), &[]);
        custom_server.install = Some("pipx install python-lsp-server".into());
        let servers = with_defaults(BTreeMap::from([
            ("python".to_string(), custom),
            ("rust".to_string(), custom_server),
        ]));
        assert_eq!(
            servers["python"].install.as_deref(),
            Some("pip install pyright")
        );
        assert_eq!(
            servers["rust"].install.as_deref(),
            Some("pipx install python-lsp-server")
        );
    }

    #[test]
    fn a_custom_command_without_install_has_none() {
        let servers = with_defaults(BTreeMap::from([
            ("python".to_string(), server(Some("pylsp"), &[])),
            ("zig".to_string(), server(Some("zls"), &[])),
            // Naming the default program keeps its default install.
            ("go".to_string(), server(Some("gopls"), &["serve"])),
        ]));
        assert_eq!(servers["python"].install, None);
        assert_eq!(servers["zig"].install, None);
        assert_eq!(
            servers["go"].install.as_deref(),
            default_install("go").map(|d| d.0)
        );
    }

    #[test]
    fn a_sudo_command_is_copy_only() {
        assert!(copy_only("sudo apt install clangd"));
        assert!(copy_only("  sudo   pacman -S gopls"));
        assert!(!copy_only("npm install -g pyright"));
        assert!(!copy_only("sudoku install"));
        assert!(!copy_only(""));
        assert_eq!(install_tool("sudo apt install clangd"), Some("apt"));
        assert_eq!(install_tool("sudo"), None);
        assert_eq!(install_tool("   "), None);
    }

    fn exe(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    #[test]
    fn install_state_reports_the_server_and_its_tool_on_a_fake_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = Some(dir.path().as_os_str());
        let servers = with_defaults(BTreeMap::new());

        assert_eq!(
            install_state(&servers["python"], path, None),
            InstallState {
                command_found: false,
                tool: Some("npm".into()),
                tool_found: false,
                install: Some("npm install -g pyright".into()),
                copy_only: false,
            }
        );

        touch(&dir.path().join(exe("npm")));
        let tool_only = install_state(&servers["python"], path, None);
        assert!(!tool_only.command_found);
        assert!(tool_only.tool_found);

        touch(&dir.path().join(exe("pyright-langserver")));
        let both = install_state(&servers["python"], path, None);
        assert!(both.command_found);
        assert!(both.tool_found);

        // Another language's tool isn't on this PATH.
        let rust = install_state(&servers["rust"], path, None);
        assert_eq!(rust.tool.as_deref(), Some("rustup"));
        assert!(!rust.tool_found);
        assert!(!rust.command_found);
    }

    #[test]
    fn install_state_without_an_install_command_has_no_tool() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(exe("pylsp")));
        let servers = with_defaults(BTreeMap::from([(
            "python".to_string(),
            server(Some("pylsp"), &[]),
        )]));
        let state = install_state(&servers["python"], Some(dir.path().as_os_str()), None);
        assert!(state.command_found);
        assert_eq!(state.install, None);
        assert_eq!(state.tool, None);
        assert!(!state.tool_found);
        assert!(!state.copy_only);
    }

    #[test]
    fn install_state_marks_a_sudo_install_copy_only() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(exe("apt")));
        let mut clangd = server(Some("clangd"), &[]);
        clangd.install = Some("sudo apt install clangd".into());
        let state = install_state(&clangd, Some(dir.path().as_os_str()), None);
        assert!(state.copy_only);
        assert_eq!(state.tool.as_deref(), Some("apt"));
        assert!(state.tool_found);
        assert!(!state.command_found);
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
