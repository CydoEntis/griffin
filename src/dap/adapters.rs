//! Which debug adapter each language uses and how its program is launched: the
//! spec's defaults, with `config.toml` `[debug.<lang>]` laid over the adapter and
//! `.glyph.toml` `[debug]` laid over the launch.

// Nothing starts a session yet; the start-debugging ticket is the first caller.
// Tests exercise everything here, so only the non-test build sees it unused.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::{DebugAdapter, DebugLaunch};
use crate::lsp::servers;

/// Every language with a default adapter, by the highlight registry's names.
pub const LANGUAGES: [&str; 3] = ["rust", "python", "go"];

/// The spec's default adapter command and args for `lang`.
pub fn default_adapter(lang: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match lang {
        "rust" => ("lldb-dap", &[]),
        "python" => ("python", &["-m", "debugpy.adapter"]),
        "go" => ("dlv", &["dap"]),
        _ => return None,
    })
}

/// Why a debug session can't be set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugError {
    /// Neither a default nor `[debug.<lang>]` names an adapter.
    NoDebugger(String),
    /// The default launch needs the active file and there is none.
    NeedsFile,
    /// Rust's default program is named after the package, and `Cargo.toml` at the
    /// root has none (or can't be read).
    NeedsPackage,
}

impl fmt::Display for DebugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DebugError::NoDebugger(lang) => write!(f, "no debugger for {lang}"),
            DebugError::NeedsFile => f.write_str("no file to debug"),
            DebugError::NeedsPackage => f.write_str("no package name in Cargo.toml"),
        }
    }
}

/// The adapter program to start, before it is looked up on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub command: String,
    pub args: Vec<String>,
}

/// The adapter for `lang`: the default, overridden by `config`. A table naming
/// an adapter replaces the default command and args; one with only `args` keeps
/// the default program with those args, as `[lsp.<lang>]` does. A language with
/// no default can still be debugged when its table names an adapter.
pub fn adapter_for(
    lang: &str,
    config: &BTreeMap<String, DebugAdapter>,
) -> Result<Adapter, DebugError> {
    let table = config.get(lang);
    if let Some(command) = table.and_then(|t| t.adapter.clone()) {
        let args = table.map(|t| t.args.clone()).unwrap_or_default();
        return Ok(Adapter { command, args });
    }
    let (command, default_args) =
        default_adapter(lang).ok_or_else(|| DebugError::NoDebugger(lang.to_string()))?;
    let args = match table {
        Some(t) if !t.args.is_empty() => t.args.clone(),
        _ => default_args.iter().map(|a| a.to_string()).collect(),
    };
    Ok(Adapter {
        command: command.to_string(),
        args,
    })
}

/// Where `winget install LLVM.LLVM` puts `lldb-dap.exe`, which isn't on PATH by
/// default. Only Windows has it.
pub fn llvm_bin() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let program_files =
        std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into());
    Some(PathBuf::from(program_files).join("LLVM").join("bin"))
}

/// Where `command` would be run from: the PATH lookup the language servers use,
/// then, for `lldb-dap` only, `llvm_bin` (the folder LLVM's installer uses).
pub fn find(
    command: &str,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
    llvm_bin: Option<&Path>,
) -> Option<PathBuf> {
    servers::find(command, path, pathext).or_else(|| {
        let dir = llvm_bin.filter(|_| command == "lldb-dap")?;
        servers::find(command, Some(dir.as_os_str()), pathext)
    })
}

/// What to start for `command` in this process's environment; the bare name when
/// it's found nowhere, so starting it fails as "not found".
pub fn program(command: &str) -> PathBuf {
    let path = std::env::var_os("PATH");
    let pathext = std::env::var_os("PATHEXT");
    find(
        command,
        path.as_deref(),
        pathext.as_deref(),
        llvm_bin().as_deref(),
    )
    .unwrap_or_else(|| PathBuf::from(command))
}

/// Everything needed to launch the program under the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Run in the run panel before the adapter starts; a failed build stops there.
    pub build: Option<String>,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Delve's launch mode; other adapters take none.
    pub mode: Option<&'static str>,
}

impl Launch {
    /// The `arguments` of the DAP `launch` request.
    pub fn arguments(&self) -> Value {
        let mut args = json!({
            "program": self.program.to_string_lossy(),
            "args": self.args,
            "cwd": self.cwd.to_string_lossy(),
        });
        if let Some(mode) = self.mode {
            args["mode"] = json!(mode);
        }
        args
    }
}

/// The launch for `lang` in the project at `root` with `file` the active file:
/// the language's default, with each field `project` sets replacing it. A field
/// the project sets is never computed, so a Rust project naming its `program`
/// needs no package in `Cargo.toml`.
pub fn launch_for(
    lang: &str,
    root: &Path,
    file: Option<&Path>,
    project: &DebugLaunch,
) -> Result<Launch, DebugError> {
    let build = match &project.build {
        Some(build) if build.trim().is_empty() => None,
        Some(build) => Some(build.clone()),
        None => (lang == "rust").then(|| "cargo build".to_string()),
    };
    let program = match &project.program {
        Some(program) => root.join(program),
        None => default_program(lang, root, file)?,
    };
    let cwd = project
        .cwd
        .as_ref()
        .map_or_else(|| root.to_path_buf(), |cwd| root.join(cwd));
    Ok(Launch {
        build,
        program,
        args: project.args.clone().unwrap_or_default(),
        cwd,
        mode: (lang == "go").then_some("debug"),
    })
}

/// Rust: the package's debug binary; Go: the active file's package folder;
/// anything else (Python, or a language debugged through `[debug.<lang>]`): the
/// active file itself.
fn default_program(lang: &str, root: &Path, file: Option<&Path>) -> Result<PathBuf, DebugError> {
    match lang {
        "rust" => {
            let name = package_name(root).ok_or(DebugError::NeedsPackage)?;
            let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
            Ok(root.join("target").join("debug").join(exe))
        }
        "go" => {
            let file = file.ok_or(DebugError::NeedsFile)?;
            Ok(file.parent().unwrap_or(root).to_path_buf())
        }
        _ => file.map(Path::to_path_buf).ok_or(DebugError::NeedsFile),
    }
}

/// `[package] name` from `Cargo.toml` at `root`.
fn package_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let manifest: toml::Table = toml::from_str(&text).ok()?;
    manifest
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::languages;

    fn table(adapter: Option<&str>, args: &[&str]) -> DebugAdapter {
        DebugAdapter {
            adapter: adapter.map(str::to_string),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn adapter(command: &str, args: &[&str]) -> Adapter {
        Adapter {
            command: command.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn rust_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn each_language_gets_the_spec_default_adapter() {
        let none = BTreeMap::new();
        assert_eq!(adapter_for("rust", &none), Ok(adapter("lldb-dap", &[])));
        assert_eq!(
            adapter_for("python", &none),
            Ok(adapter("python", &["-m", "debugpy.adapter"]))
        );
        assert_eq!(adapter_for("go", &none), Ok(adapter("dlv", &["dap"])));
    }

    #[test]
    fn a_language_without_an_adapter_has_no_debugger() {
        let err = adapter_for("typescript", &BTreeMap::new()).unwrap_err();
        assert_eq!(err, DebugError::NoDebugger("typescript".into()));
        assert_eq!(err.to_string(), "no debugger for typescript");
        // A table with only args doesn't conjure an adapter either.
        let config = BTreeMap::from([("sql".to_string(), table(None, &["-v"]))]);
        assert_eq!(
            adapter_for("sql", &config).unwrap_err().to_string(),
            "no debugger for sql"
        );
    }

    #[test]
    fn config_replaces_the_adapter_and_its_args() {
        let config = BTreeMap::from([
            (
                "rust".to_string(),
                table(Some("codelldb"), &["--port", "0"]),
            ),
            ("python".to_string(), table(Some("my-debugpy"), &[])),
            ("go".to_string(), table(None, &["dap", "--log"])),
            ("zig".to_string(), table(Some("zig-dap"), &[])),
        ]);
        assert_eq!(
            adapter_for("rust", &config),
            Ok(adapter("codelldb", &["--port", "0"]))
        );
        // Naming an adapter drops the default's args.
        assert_eq!(
            adapter_for("python", &config),
            Ok(adapter("my-debugpy", &[]))
        );
        // Only args keeps the default program.
        assert_eq!(
            adapter_for("go", &config),
            Ok(adapter("dlv", &["dap", "--log"]))
        );
        assert_eq!(adapter_for("zig", &config), Ok(adapter("zig-dap", &[])));
    }

    #[test]
    fn language_ids_match_the_highlight_registry() {
        for lang in LANGUAGES {
            assert!(languages::for_name(lang).is_some(), "{lang}");
            assert!(default_adapter(lang).is_some(), "{lang}");
        }
        let lang = languages::for_path(Path::new("main.go")).unwrap().name;
        assert!(adapter_for(lang, &BTreeMap::new()).is_ok());
    }

    fn touch(path: &Path) {
        std::fs::write(path, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn exe(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    #[test]
    fn adapters_are_found_on_path() {
        let bin = tempfile::tempdir().unwrap();
        touch(&bin.path().join(exe("dlv")));
        let path = Some(bin.path().as_os_str());
        assert_eq!(
            find("dlv", path, None, None),
            Some(bin.path().join(exe("dlv")))
        );
        assert_eq!(find("lldb-dap", path, None, None), None);
    }

    #[test]
    fn lldb_dap_is_also_looked_for_in_the_llvm_folder() {
        let path_dir = tempfile::tempdir().unwrap();
        let llvm = tempfile::tempdir().unwrap();
        touch(&llvm.path().join(exe("lldb-dap")));
        touch(&llvm.path().join(exe("dlv")));
        let path = Some(path_dir.path().as_os_str());
        assert_eq!(
            find("lldb-dap", path, None, Some(llvm.path())),
            Some(llvm.path().join(exe("lldb-dap")))
        );
        // Only lldb-dap: the folder is LLVM's, not a second PATH.
        assert_eq!(find("dlv", path, None, Some(llvm.path())), None);
        // PATH wins when both have it.
        touch(&path_dir.path().join(exe("lldb-dap")));
        assert_eq!(
            find("lldb-dap", path, None, Some(llvm.path())),
            Some(path_dir.path().join(exe("lldb-dap")))
        );
    }

    #[test]
    fn the_llvm_folder_is_only_searched_on_windows() {
        let dir = llvm_bin();
        assert_eq!(dir.is_some(), cfg!(windows));
        if let Some(dir) = dir {
            assert!(dir.ends_with(Path::new("LLVM").join("bin")), "{dir:?}");
        }
    }

    #[test]
    fn an_adapter_found_nowhere_starts_by_its_bare_name() {
        let name = "glyph-test-no-such-adapter";
        assert_eq!(program(name), PathBuf::from(name));
    }

    #[test]
    fn rust_launches_the_packages_debug_binary_after_cargo_build() {
        let dir = rust_project();
        let root = dir.path();
        let file = root.join("src").join("main.rs");
        let launch = launch_for("rust", root, Some(&file), &DebugLaunch::default()).unwrap();
        let program = root
            .join("target")
            .join("debug")
            .join(format!("my-app{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(launch.build.as_deref(), Some("cargo build"));
        assert_eq!(launch.program, program);
        assert_eq!(launch.cwd, root);
        assert_eq!(
            launch.arguments(),
            json!({
                "program": program.to_string_lossy(),
                "args": [],
                "cwd": root.to_string_lossy(),
            })
        );
    }

    #[test]
    fn rust_without_a_package_cannot_launch_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let none = DebugLaunch::default();
        assert_eq!(
            launch_for("rust", dir.path(), None, &none),
            Err(DebugError::NeedsPackage)
        );
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        assert_eq!(
            launch_for("rust", dir.path(), None, &none),
            Err(DebugError::NeedsPackage)
        );
    }

    #[test]
    fn python_launches_the_active_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("app").join("main.py");
        let launch =
            launch_for("python", dir.path(), Some(&file), &DebugLaunch::default()).unwrap();
        assert_eq!(launch.build, None);
        assert_eq!(
            launch.arguments(),
            json!({
                "program": file.to_string_lossy(),
                "args": [],
                "cwd": dir.path().to_string_lossy(),
            })
        );
        assert_eq!(
            launch_for("python", dir.path(), None, &DebugLaunch::default()),
            Err(DebugError::NeedsFile)
        );
    }

    #[test]
    fn go_launches_the_active_files_folder_in_debug_mode() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("cmd").join("server");
        let file = pkg.join("main.go");
        let launch = launch_for("go", dir.path(), Some(&file), &DebugLaunch::default()).unwrap();
        assert_eq!(launch.build, None);
        assert_eq!(
            launch.arguments(),
            json!({
                "program": pkg.to_string_lossy(),
                "args": [],
                "cwd": dir.path().to_string_lossy(),
                "mode": "debug",
            })
        );
    }

    #[test]
    fn the_project_replaces_each_launch_field() {
        // No Cargo.toml: a project naming its program never needs the package.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let project = DebugLaunch {
            program: Some("out/app".into()),
            args: Some(vec!["--port".into(), "8080".into()]),
            cwd: Some("work".into()),
            build: Some("make debug".into()),
        };
        let launch = launch_for("rust", root, None, &project).unwrap();
        assert_eq!(
            launch,
            Launch {
                build: Some("make debug".into()),
                program: root.join("out/app"),
                args: vec!["--port".into(), "8080".into()],
                cwd: root.join("work"),
                mode: None,
            }
        );
        // The Go mode stays: the project overrides the launch, not the adapter.
        let go = launch_for("go", root, None, &project).unwrap();
        assert_eq!(go.arguments()["mode"], "debug");
        assert_eq!(go.arguments()["args"], json!(["--port", "8080"]));
    }

    #[test]
    fn an_absolute_program_stays_and_an_empty_build_means_none() {
        let dir = rust_project();
        let elsewhere = tempfile::tempdir().unwrap();
        let program = elsewhere.path().join("tool");
        let project = DebugLaunch {
            program: Some(program.to_string_lossy().into_owned()),
            build: Some(String::new()),
            ..DebugLaunch::default()
        };
        let launch = launch_for("rust", dir.path(), None, &project).unwrap();
        assert_eq!(launch.program, program);
        assert_eq!(launch.build, None);
        // Fields the project leaves out keep their defaults.
        let only_args = DebugLaunch {
            args: Some(vec!["-q".into()]),
            ..DebugLaunch::default()
        };
        let launch = launch_for("rust", dir.path(), None, &only_args).unwrap();
        assert_eq!(launch.build.as_deref(), Some("cargo build"));
        assert_eq!(launch.args, ["-q"]);
    }
}
