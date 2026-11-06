//! The `chamfera-cli` binary as a user's shell runs it.

use std::process::{Command, Stdio};

/// `chamfera-cli commands | head -0`: stdout is a pipe whose reader is already gone, so
/// the first write fails with EPIPE. The CLI stops quietly with status 0 instead of
/// panicking ("failed printing to stdout: Broken pipe").
#[test]
fn a_closed_stdout_pipe_ends_the_cli_quietly() {
    for args in [
        &["commands"][..],
        &["exec", "app.commands"][..],
        &["--version"][..],
    ] {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let out = Command::new(env!("CARGO_BIN_EXE_chamfera-cli"))
            .args(args)
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("panicked"), "{args:?}: {err}");
        assert!(err.is_empty(), "{args:?}: {err}");
        assert_eq!(out.status.code(), Some(0), "{args:?}: {err}");
    }
}

/// A real error still goes to stderr with status 1.
#[test]
fn an_unknown_command_is_an_error_with_status_one() {
    let out = Command::new(env!("CARGO_BIN_EXE_chamfera-cli"))
        .args(["exec", "no.such.command"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: "));
}
