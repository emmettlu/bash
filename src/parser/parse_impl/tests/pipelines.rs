//! Tests for pipeline parsing.

use super::{ParseResult, parse, test_with_snapshot};
use crate::assert_snapshot_redacted;
use crate::parser::ast::{Command, IoFileRedirectTarget, IoRedirect, PipeKind};
use anyhow::Result;

#[test]
fn parse_simple_pipe() -> Result<()> {
    let input = "echo hello | grep world";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_multi_stage_pipe() -> Result<()> {
    let input = "cat file | grep pattern | wc -l";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_pipe_with_stderr() -> Result<()> {
    let input = "echo |& wc";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn pipe_with_stderr_preserves_edge_kind() {
    let result = parse("echo |& wc").unwrap();
    let pipeline = &result.complete_commands[0].0[0].0.first;

    assert!(matches!(
        pipeline.pipe_kinds.as_slice(),
        [PipeKind::StdoutAndStderr(_)]
    ));
}

#[test]
fn pipe_with_stderr_supports_extended_tests() {
    let result = parse("[[ -n value ]] |& wc").unwrap();
    let pipeline = &result.complete_commands[0].0[0].0.first;
    let Command::ExtendedTest(_, Some(redirects)) = &pipeline.seq[0] else {
        panic!("expected an extended test with a redirect");
    };

    assert!(matches!(
        redirects.0.as_slice(),
        [IoRedirect::File(Some(2), _, IoFileRedirectTarget::Fd(1))]
    ));
}

#[test]
fn parse_timed_pipeline() -> Result<()> {
    let input = "time echo hello";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_timed_pipeline_posix() -> Result<()> {
    let input = "time -p echo hello";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_negated_pipeline() -> Result<()> {
    let input = "! echo hello";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_negated_timed_pipeline() -> Result<()> {
    let input = "time ! echo hello";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_pipe_with_multiple_commands() -> Result<()> {
    let input = "ls -la | head -10 | tail -5";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}
