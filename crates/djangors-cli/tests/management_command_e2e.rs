//! Real end-to-end test of the custom management commands plugin mechanism (Phase 11, item 8).
//!
//! Exercises the actual `dj` binary as a subprocess (not the Rust functions directly),
//! proving `dj` genuinely routes an unrecognized subcommand through `cargo run --quiet` with
//! `DJANGORS_RUN_COMMAND` set, into a real project's own compiled binary, which looks up and
//! runs the matching `#[management_command]`-registered handler.

use std::path::PathBuf;
use std::process::Command;

fn polls_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("polls")
}

fn dj_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_dj"))
}

#[test]
fn custom_management_command_runs_end_to_end_via_the_real_dj_binary() {
    let marker_path =
        std::env::temp_dir().join(format!("djangors_e2e_marker_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&marker_path);

    let output = Command::new(dj_bin())
        .arg("e2e_test_marker")
        .arg(marker_path.to_str().unwrap())
        .current_dir(polls_dir())
        .output()
        .expect("failed to run dj binary");

    assert!(
        output.status.success(),
        "dj e2e_test_marker should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let contents = std::fs::read_to_string(&marker_path)
        .unwrap_or_else(|e| panic!("marker file was never written: {e}"));
    assert_eq!(contents, "management command ran");

    std::fs::remove_file(&marker_path).ok();
}

#[test]
fn a_truly_unknown_command_produces_a_clear_error_not_a_silent_success() {
    let output = Command::new(dj_bin())
        .arg("this_command_does_not_exist_anywhere")
        .current_dir(polls_dir())
        .output()
        .expect("failed to run dj binary");

    assert!(
        !output.status.success(),
        "an unrecognized, unregistered command must not succeed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown") || stderr.contains("this_command_does_not_exist_anywhere"),
        "stderr should clearly report the unknown command, got: {stderr}"
    );
}

#[test]
fn builtin_subcommands_still_work_unaffected() {
    let output = Command::new(dj_bin())
        .arg("--help")
        .output()
        .expect("failed to run dj binary");
    assert!(output.status.success(), "dj --help must still work");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: dj") && stdout.contains("new"),
        "dj --help should show real clap-generated help, got: {stdout}"
    );

    let output = Command::new(dj_bin())
        .arg("--version")
        .output()
        .expect("failed to run dj binary");
    assert!(output.status.success(), "dj --version must still work");
}
