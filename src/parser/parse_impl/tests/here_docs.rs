//! Tests for here-document parsing.

use super::{ParseResult, parse, test_with_snapshot};
use crate::assert_snapshot_redacted;
use crate::parser::ast::{Command, CommandPrefixOrSuffixItem, IoRedirect};
use anyhow::Result;

#[test]
fn parse_here_doc_basic() -> Result<()> {
    let input = r"cat <<EOF
content line 1
content line 2
EOF
";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_no_trailing_newline() -> Result<()> {
    let input = r"cat <<EOF
Something
EOF";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_tab_removal() -> Result<()> {
    let input = "cat <<-EOF\n\tcontent with tab\nEOF\n";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_quoted_delimiter() -> Result<()> {
    let input = r"cat <<'EOF'
$variable should not expand
EOF
";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_double_quoted_delimiter() -> Result<()> {
    let input = r#"cat <<"EOF"
$variable should not expand
EOF
"#;
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_mixed_quote_removal() {
    let input = "cat <<'E'\"O\"\\F\ncontent\nEOF\n";
    let program = parse(input).unwrap();
    let Command::Simple(command) = &program.complete_commands[0].0[0].0.first.seq[0] else {
        panic!("expected simple command");
    };
    let Some(suffix) = &command.suffix else {
        panic!("expected command suffix");
    };
    let CommandPrefixOrSuffixItem::IoRedirect(IoRedirect::HereDocument(_, here_document)) =
        &suffix.0[0]
    else {
        panic!("expected here-document redirect");
    };

    assert_eq!(here_document.here_end.value, "'E'\"O\"\\F");
    assert!(!here_document.requires_expansion);
    assert_eq!(here_document.doc.value, "content\n");
    assert_eq!(here_document.loc.start.index, 4);
    assert_eq!(here_document.loc.end.index, input.len());
}

#[test]
fn parse_here_doc_with_expansion() -> Result<()> {
    let input = r"cat <<EOF
Hello $USER
Your home is $HOME
EOF
";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_empty() -> Result<()> {
    let input = r"cat <<EOF
EOF
";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_with_command_after() -> Result<()> {
    let input = r"cat <<EOF | grep hello
hello world
goodbye world
EOF
";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}

#[test]
fn parse_here_doc_with_fd() -> Result<()> {
    let input = r"command 3<<EOF
content for fd 3
EOF
";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult {
        input,
        result: &result
    });
    Ok(())
}
