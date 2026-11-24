//! Companion servers. The MCP server and the Apify Actor are separate programs
//! (`dwg-mcp`, `dwg-apify`), installed on their own. `dwg2png --mcp` and
//! `dwg2png --apify` hand the process over to them.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub struct Plugin {
    pub flag: &'static str,
    pub program: &'static str,
    pub purpose: &'static str,
}

pub const PLUGINS: [Plugin; 2] = [
    Plugin { flag: "--mcp", program: "dwg-mcp", purpose: "the MCP server" },
    Plugin { flag: "--apify", program: "dwg-apify", purpose: "the Apify Actor" },
];

/// The companion program a first argument selects, if any.
pub fn plugin_for(first_arg: &str) -> Option<&'static Plugin> {
    PLUGINS.iter().find(|p| p.flag == first_arg)
}

/// Where the program may be, in order: next to this executable, then each PATH entry.
pub fn candidate_paths(program: &str, exe_dir: Option<&Path>, path_var: Option<&OsStr>) -> Vec<PathBuf> {
    let file = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    let path_dirs = path_var.map(|p| std::env::split_paths(p).collect::<Vec<_>>()).unwrap_or_default();
    exe_dir
        .map(Path::to_path_buf)
        .into_iter()
        .chain(path_dirs)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(&file))
        .collect()
}

fn find_program(program: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    let path_var = std::env::var_os("PATH");
    candidate_paths(program, exe_dir, path_var.as_deref()).into_iter().find(|p| p.is_file())
}

#[cfg(unix)]
fn hand_over(mut command: Command) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    // Replace this process, so signals and stdio reach the server directly.
    command.exec()
}

#[cfg(not(unix))]
fn hand_over(mut command: Command) -> std::io::Error {
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => e,
    }
}

/// Run the companion program with the remaining arguments. Returns the exit code to
/// use when it could not be started; on success it does not return.
pub fn run(plugin: &Plugin, args: &[String]) -> i32 {
    let Some(path) = find_program(plugin.program) else {
        eprintln!(
            "dwg2png: {} is a separate program, `{}`, and it is not installed.\n\
             Install it and run `dwg2png {}` again:\n  \
             brew install aileron-tech/tap/{}\n  \
             cargo install --git https://github.com/aileron-tech/{}",
            plugin.purpose, plugin.program, plugin.flag, plugin.program, plugin.program
        );
        return 127;
    };
    let mut command = Command::new(&path);
    command.args(args);
    let error = hand_over(command);
    eprintln!("dwg2png: cannot start {}: {error}", path.display());
    126
}

#[cfg(test)]
mod tests {
    use super::*;

    /* REQ-CLI-001: --mcp and --apify select their companion programs; other arguments do not */
    #[test]
    fn flags_select_companion_programs() {
        assert_eq!(plugin_for("--mcp").map(|p| p.program), Some("dwg-mcp"));
        assert_eq!(plugin_for("--apify").map(|p| p.program), Some("dwg-apify"));
        assert_eq!(plugin_for("mcp"), None);
        assert_eq!(plugin_for("--out"), None);
    }

    /* REQ-CLI-002: the companion is looked up next to dwg2png first, then on PATH in order */
    #[test]
    fn lookup_order_is_exe_dir_then_path() {
        let path_var = std::env::join_paths(["/opt/a", "/opt/b"]).unwrap();
        let found = candidate_paths("dwg-mcp", Some(Path::new("/usr/local/bin")), Some(&path_var));
        let names: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
        let exe = std::env::consts::EXE_SUFFIX;
        assert_eq!(
            names,
            vec![
                format!("/usr/local/bin/dwg-mcp{exe}"),
                format!("/opt/a/dwg-mcp{exe}"),
                format!("/opt/b/dwg-mcp{exe}")
            ]
        );
        assert!(candidate_paths("dwg-mcp", None, None).is_empty());
    }
}
