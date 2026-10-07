//! `glyph --health`: run the real binary with a PATH holding only a fake
//! `rust-analyzer` and check the report word for word.

use std::fs;
use std::path::Path;
use std::process::Command;

fn fake_program(dir: &Path, name: &str) {
    let file = if cfg!(windows) {
        dir.join(format!("{name}.exe"))
    } else {
        dir.join(name)
    };
    fs::write(&file, "").expect("write fake server");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).expect("chmod fake server");
    }
}

fn health(path: &Path, config: &str) -> std::process::Output {
    let config_dir = tempfile::tempdir().expect("create config dir");
    let config_file = config_dir.path().join("config.toml");
    fs::write(&config_file, config).expect("write config");
    Command::new(env!("CARGO_BIN_EXE_glyph"))
        .arg("--health")
        .env("PATH", path)
        .env("GLYPH_CONFIG", &config_file)
        .output()
        .expect("run glyph --health")
}

#[test]
fn health_lists_each_language_and_whether_its_server_is_found() {
    let bin = tempfile::tempdir().expect("create PATH dir");
    fake_program(bin.path(), "rust-analyzer");

    let out = health(bin.path(), "");

    assert!(out.status.success(), "exit status: {:?}", out.status);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        "\
rust        rust-analyzer  found
go          gopls  missing
typescript  typescript-language-server --stdio  missing
tsx         typescript-language-server --stdio  missing
javascript  typescript-language-server --stdio  missing
jsx         typescript-language-server --stdio  missing
python      pyright-langserver --stdio  missing
html        vscode-html-language-server --stdio  missing
css         vscode-css-language-server --stdio  missing
sql         sqls  missing
"
    );
}

#[test]
fn health_reports_a_configured_server_instead_of_the_default() {
    let bin = tempfile::tempdir().expect("create PATH dir");
    fake_program(bin.path(), "pylsp");

    let out = health(bin.path(), "[lsp.python]\ncommand = \"pylsp\"\n");

    assert!(out.status.success(), "exit status: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    assert!(
        stdout.contains("\npython      pylsp  found\n"),
        "report:\n{stdout}"
    );
    assert!(stdout.starts_with("rust        rust-analyzer  missing\n"));
}
