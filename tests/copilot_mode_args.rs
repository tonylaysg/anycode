//! Regression tests for Copilot-mode argument assembly.
//!
//! `anycopilot` (and any anycode invocation against a Copilot profile) spawns
//! the GitHub Copilot CLI, which has a different flag surface than Claude
//! Code. Specifically:
//!
//! * Copilot CLI has **no** `--session-id` flag — passing one causes
//!   `error: unknown option '--session-id'` and an immediate exit.
//! * Copilot's `--resume` takes its value via `=` (it's declared as optional).
//! * `--teammate-mode` and the hooks `--settings` JSON are Claude-only.
//!
//! These tests pin the contract so future edits to the Claude codepath can't
//! silently regress Copilot spawns.

use anycode::args::{build_spawn_params, classify, flag_registry, ArgAssembler};
use anycode::config::ClaudeSettingsManager;

fn settings() -> ClaudeSettingsManager {
    ClaudeSettingsManager::new()
}

#[test]
fn copilot_initial_spawn_has_no_session_id_flag() {
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:12345",
        "COPILOT_API_URL",
        "copilot",
        "tok",
        &settings(),
        None,
        Some(9999),
        false,
            "anthropic",
        );
    assert_eq!(p.command, "copilot");
    assert!(
        !p.args.iter().any(|a| a == "--session-id"),
        "Copilot CLI has no --session-id flag, got: {:?}",
        p.args
    );
    // Subagent hooks (--settings <json>) are Claude-only and must be absent.
    assert!(
        !p.args.iter().any(|a| a == "--settings"),
        "--settings is Claude-only, got: {:?}",
        p.args
    );
    // teammate mode (only set if shim is Some) must be absent regardless.
    assert!(!p.args.iter().any(|a| a == "--teammate-mode"));
}

#[test]
fn copilot_resume_uses_equals_form() {
    let registry = flag_registry();
    let classified = classify(&["--continue".to_string()], &registry);
    let args = ArgAssembler::from_passthrough(&classified.args)
        .copilot_mode(true)
        .with_session_resume("session-42")
        .build();
    assert!(
        args.iter().any(|a| a == "--resume=session-42"),
        "Copilot resume should use --resume=<id>, got: {:?}",
        args
    );
    assert!(
        !args.iter().any(|a| a == "--session-id"),
        "--session-id leaked into Copilot args: {:?}",
        args
    );
}

#[test]
fn claude_initial_still_injects_session_id() {
    // Regression guard for the Claude codepath — unchanged behavior.
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:12345",
        "ANTHROPIC_BASE_URL",
        "claude",
        "tok",
        &settings(),
        None,
        Some(9999),
        true,
            "anthropic",
        );
    assert!(p.args.iter().any(|a| a == "--session-id"));
}

#[test]
fn copilot_assembler_strips_passthrough_session_id() {
    // If the user somehow passes --session-id on the anycopilot command line
    // (they shouldn't, but defense in depth), we must strip it before handing
    // args to Copilot CLI.
    let registry = flag_registry();
    let classified = classify(
        &[
            "--session-id".to_string(),
            "bogus-id".to_string(),
            "--allow-all-tools".to_string(),
        ],
        &registry,
    );
    let args = ArgAssembler::from_passthrough(&classified.args)
        .copilot_mode(true)
        .build();
    assert!(
        !args.iter().any(|a| a == "--session-id"),
        "copilot_mode must strip --session-id passthrough, got: {:?}",
        args
    );
    assert!(
        !args.iter().any(|a| a == "bogus-id"),
        "bogus session-id value leaked into copilot args: {:?}",
        args
    );
    assert!(
        args.iter().any(|a| a == "--allow-all-tools"),
        "unrelated Copilot passthrough flag should survive: {:?}",
        args
    );
}

#[test]
fn copilot_mode_noops_teammate_and_subagent_hooks() {
    let args = ArgAssembler::new()
        .copilot_mode(true)
        .with_subagent_hooks(8080)
        .build();
    assert!(
        args.is_empty(),
        "Copilot mode should no-op Claude-only builders, got: {:?}",
        args
    );
}
