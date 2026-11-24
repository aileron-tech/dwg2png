//! `dwg2png --mcp` and `dwg2png --apify` hand over to the separately installed
//! companion programs. A stand-in script plays the companion.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

fn install_stand_in(dir: &Path, program: &str) {
    let path = dir.join(program);
    std::fs::write(&path, "#!/bin/sh\necho \"program=$(basename \"$0\") args=$*\"\ncat\nexit 7\n")
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn dwg2png(path_var: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
        .args(args)
        .env("PATH", format!("{}:/bin:/usr/bin", path_var.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/* REQ-CLI-003: --mcp runs dwg-mcp with the remaining arguments, its stdio and its exit code */
#[test]
fn mcp_flag_hands_over_to_dwg_mcp() {
    let dir = tempfile::tempdir().unwrap();
    install_stand_in(dir.path(), "dwg-mcp");
    let (code, stdout, _) = dwg2png(dir.path(), &["--mcp", "--verbose", "x"], "from stdin\n");
    assert_eq!(stdout, "program=dwg-mcp args=--verbose x\nfrom stdin\n");
    assert_eq!(code, 7);
}

/* REQ-CLI-004: --apify runs dwg-apify */
#[test]
fn apify_flag_hands_over_to_dwg_apify() {
    let dir = tempfile::tempdir().unwrap();
    install_stand_in(dir.path(), "dwg-apify");
    let (code, stdout, _) = dwg2png(dir.path(), &["--apify"], "");
    assert_eq!(stdout, "program=dwg-apify args=\n");
    assert_eq!(code, 7);
}

/* REQ-CLI-005: a missing companion is reported with how to install it, and exits 127 */
#[test]
fn missing_companion_is_reported() {
    let empty = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = dwg2png(empty.path(), &["--mcp"], "");
    assert_eq!(code, 127);
    assert_eq!(stdout, "");
    assert!(stderr.contains("dwg-mcp") && stderr.contains("not installed"), "{stderr}");
    assert!(stderr.contains("brew install aileron-tech/tap/dwg-mcp"), "{stderr}");
}

/* REQ-CLI-006: without a companion flag the renderer CLI still prints its usage */
#[test]
fn plain_cli_is_unchanged() {
    let empty = tempfile::tempdir().unwrap();
    let (code, _, stderr) = dwg2png(empty.path(), &[], "");
    assert_eq!(code, 1);
    assert!(stderr.contains("usage: dwg2png"), "{stderr}");
}
