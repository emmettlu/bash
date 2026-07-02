use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TestCaseSet {
    /// Name of the test case set
    pub name: Option<String>,
    /// Set of test cases
    pub cases: Vec<TestCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TestCase {
    /// Name of the test case
    pub name: Option<String>,
    #[serde(default)]
    pub stdin: Option<String>,
}

// Disabled glob-based YAML parser test.
// The test cases live in an external repository (previously brush-shell).
// To run manually, place yaml files in a directory and uncomment + adjust the glob.
// #[test]
// fn test_parser_using_yaml() {
//     insta::glob!("tests/parser/cases", "**/*.yaml", |path| {
//         let _ = test_parser_using_yaml(path);
//     });
// }
fn test_parser_using_yaml(path: &Path) -> Result<()> {
    let yaml_file = std::fs::File::open(path)?;
    let test_case_set: TestCaseSet = serde_yaml::from_reader(yaml_file)
        .context(format!("parsing {}", path.to_string_lossy()))?;

    test_parser_using_test_case_set(&test_case_set);

    Ok(())
}

fn test_parser_using_test_case_set(test_case_set: &TestCaseSet) {
    let name = test_case_set.name.as_deref().unwrap_or_default();

    for test_case in &test_case_set.cases {
        parse(name, test_case);
    }
}

// NOTE: The name of this function affects the name of the snapshot generated.
fn parse(test_case_set_name: &str, test_case: &TestCase) {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct TestCaseInfo {
        test_case_set: String,
        test_case: String,
    }

    let name = test_case.name.as_deref().unwrap_or_default();
    let script_content = test_case.stdin.as_deref().unwrap_or_default();

    if script_content.is_empty() {
        return;
    }

    let summary = parse_script_content(script_content);

    let info = TestCaseInfo {
        test_case_set: test_case_set_name.to_string(),
        test_case: name.to_string(),
    };

    // Generate a cleaned-up name.
    let snapshot_suffix = std::format!("{test_case_set_name}-{name}")
        .to_lowercase()
        .replace(' ', "_")
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_");

    insta::with_settings!({
        info => &info,
        prepend_module_to_snapshot => false,
        omit_expression => true,
        snapshot_suffix => snapshot_suffix,
    }, {
        insta::assert_ron_snapshot!(summary);
    });
}

#[cfg_attr(test, derive(serde::Serialize))]
struct ParseSummary<'a> {
    input: Vec<&'a str>,
    result: ParseResult,
}

#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
enum ParseResult {
    Success(crate::parser::ast::Program),
    Failure(String),
}

fn parse_script_content(s: &str) -> ParseSummary<'_> {
    let input_lines: Vec<_> = s.lines().collect();

    let tokens = match crate::parser::tokenize_str_with_options(
        s,
        &crate::parser::TokenizerOptions::default(),
    ) {
        Ok(tokens) => tokens,
        Err(err) => {
            return ParseSummary {
                input: input_lines,
                result: ParseResult::Failure(err.to_string()),
            };
        }
    };

    let parsed_program =
        match crate::parser::parse_tokens(&tokens, &crate::parser::ParserOptions::default()) {
            Ok(parsed_program) => parsed_program,
            Err(err) => {
                return ParseSummary {
                    input: input_lines,
                    result: ParseResult::Failure(err.to_string()),
                };
            }
        };

    ParseSummary {
        input: input_lines,
        result: ParseResult::Success(parsed_program),
    }
}
