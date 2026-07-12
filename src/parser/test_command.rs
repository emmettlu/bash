//! Parser for shell test commands.

use crate::parser::{ast, error};

/// Parses a test command expression.
///
/// # Arguments
///
/// * `input` - The test command expression to parse, in string form.
pub fn parse<S: AsRef<str>>(input: &[S]) -> Result<ast::TestExpr, error::TestCommandParseError> {
    let mut depth = 0usize;
    for token in input {
        match token.as_ref() {
            "(" => {
                depth += 1;
                if depth > crate::parser::nesting::MAX_NESTING_DEPTH {
                    return Err(error::TestCommandParseError::NestingLimitExceeded {
                        limit: crate::parser::nesting::MAX_NESTING_DEPTH,
                    });
                }
            }
            ")" => depth = depth.saturating_sub(1),
            _ => {}
        }
    }

    let expr =
        test_command::full_expression(&input.iter().map(AsRef::as_ref).collect::<Vec<&str>>())?;

    Ok(expr)
}

peg::parser! {
    grammar test_command<'a>() for [&'a str] {
        pub(crate) rule full_expression() -> ast::TestExpr =
            end() { ast::TestExpr::False } /
            e:one_arg_expr() end() { e } /
            e:two_arg_expr() end()  { e } /
            e:three_arg_expr() end()  { e } /
            e:four_arg_expr() end()  { e } /
            expression()

        rule one_arg_expr() -> ast::TestExpr =
            [s] { ast::TestExpr::Literal(s.to_owned()) }

        rule two_arg_expr() -> ast::TestExpr =
            ["!"] e:one_arg_expr() { ast::TestExpr::Not(Box::from(e)) } /
            op:unary_op() [s] { ast::TestExpr::UnaryTest(op, s.to_owned()) }

        rule three_arg_expr() -> ast::TestExpr =
            [left] ["-a"] [right] { ast::TestExpr::And(Box::from(ast::TestExpr::Literal(left.to_owned())), Box::from(ast::TestExpr::Literal(right.to_owned()))) } /
            [left] ["-o"] [right] { ast::TestExpr::Or(Box::from(ast::TestExpr::Literal(left.to_owned())), Box::from(ast::TestExpr::Literal(right.to_owned()))) } /
            [left] op:binary_op() [right] { ast::TestExpr::BinaryTest(op, left.to_owned(), right.to_owned()) } /
            ["!"] e:two_arg_expr() { ast::TestExpr::Not(Box::from(e)) } /
            ["("] e:one_arg_expr() [")"] { e }

        rule four_arg_expr() -> ast::TestExpr =
            ["!"] e:three_arg_expr() { ast::TestExpr::Not(Box::from(e)) }

        rule expression() -> ast::TestExpr = precedence! {
            left:(@) ["-o"] right:@ { ast::TestExpr::Or(Box::from(left), Box::from(right)) }
            --
            left:(@) ["-a"] right:@ { ast::TestExpr::And(Box::from(left), Box::from(right)) }
            --
            ["("] e:expression() [")"] { ast::TestExpr::Parenthesized(Box::from(e)) }
            --
            ["!"] e:@ { ast::TestExpr::Not(Box::from(e)) }
            --
            [left] op:binary_op() [right] { ast::TestExpr::BinaryTest(op, left.to_owned(), right.to_owned()) }
            --
            op:unary_op() [operand] { ast::TestExpr::UnaryTest(op, operand.to_owned()) }
            --
            [s] { ast::TestExpr::Literal(s.to_owned()) }
        }

        rule unary_op() -> ast::UnaryPredicate =
            ["-a"] { ast::UnaryPredicate::FileExists } /
            ["-b"] { ast::UnaryPredicate::FileExistsAndIsBlockSpecialFile } /
            ["-c"] { ast::UnaryPredicate::FileExistsAndIsCharSpecialFile } /
            ["-d"] { ast::UnaryPredicate::FileExistsAndIsDir } /
            ["-e"] { ast::UnaryPredicate::FileExists } /
            ["-f"] { ast::UnaryPredicate::FileExistsAndIsRegularFile } /
            ["-g"] { ast::UnaryPredicate::FileExistsAndIsSetgid } /
            ["-h"] { ast::UnaryPredicate::FileExistsAndIsSymlink } /
            ["-k"] { ast::UnaryPredicate::FileExistsAndHasStickyBit } /
            ["-n"] { ast::UnaryPredicate::StringHasNonZeroLength } /
            ["-o"] { ast::UnaryPredicate::ShellOptionEnabled } /
            ["-p"] { ast::UnaryPredicate::FileExistsAndIsFifo } /
            ["-r"] { ast::UnaryPredicate::FileExistsAndIsReadable } /
            ["-s"] { ast::UnaryPredicate::FileExistsAndIsNotZeroLength } /
            ["-t"] { ast::UnaryPredicate::FdIsOpenTerminal } /
            ["-u"] { ast::UnaryPredicate::FileExistsAndIsSetuid } /
            ["-v"] { ast::UnaryPredicate::ShellVariableIsSetAndAssigned } /
            ["-w"] { ast::UnaryPredicate::FileExistsAndIsWritable } /
            ["-x"] { ast::UnaryPredicate::FileExistsAndIsExecutable } /
            ["-z"] { ast::UnaryPredicate::StringHasZeroLength } /
            ["-G"] { ast::UnaryPredicate::FileExistsAndOwnedByEffectiveGroupId } /
            ["-L"] { ast::UnaryPredicate::FileExistsAndIsSymlink } /
            ["-N"] { ast::UnaryPredicate::FileExistsAndModifiedSinceLastRead } /
            ["-O"] { ast::UnaryPredicate::FileExistsAndOwnedByEffectiveUserId } /
            ["-R"] { ast::UnaryPredicate::ShellVariableIsSetAndNameRef } /
            ["-S"] { ast::UnaryPredicate::FileExistsAndIsSocket }

        rule binary_op() -> ast::BinaryPredicate =
            ["=="]  { ast::BinaryPredicate::StringExactlyMatchesString } /
            ["-ef"] { ast::BinaryPredicate::FilesReferToSameDeviceAndInodeNumbers } /
            ["-eq"] { ast::BinaryPredicate::ArithmeticEqualTo } /
            ["-ge"] { ast::BinaryPredicate::ArithmeticGreaterThanOrEqualTo } /
            ["-gt"] { ast::BinaryPredicate::ArithmeticGreaterThan } /
            ["-le"] { ast::BinaryPredicate::ArithmeticLessThanOrEqualTo } /
            ["-lt"] { ast::BinaryPredicate::ArithmeticLessThan } /
            ["-ne"] { ast::BinaryPredicate::ArithmeticNotEqualTo } /
            ["-nt"] { ast::BinaryPredicate::LeftFileIsNewerOrExistsWhenRightDoesNot } /
            ["-ot"] { ast::BinaryPredicate::LeftFileIsOlderOrDoesNotExistWhenRightDoes } /
            ["="]   { ast::BinaryPredicate::StringExactlyMatchesString } /
            ["!="]  { ast::BinaryPredicate::StringDoesNotExactlyMatchString } /
            ["<"]   { ast::BinaryPredicate::LeftSortsBeforeRight } /
            [">"]   { ast::BinaryPredicate::LeftSortsAfterRight }

        rule end() = ![_]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn and_has_higher_precedence_than_or() {
        let expression = parse(&["a", "-o", "b", "-a", "c"]).unwrap();

        assert_eq!(
            expression,
            ast::TestExpr::Or(
                Box::new(ast::TestExpr::Literal("a".to_owned())),
                Box::new(ast::TestExpr::And(
                    Box::new(ast::TestExpr::Literal("b".to_owned())),
                    Box::new(ast::TestExpr::Literal("c".to_owned())),
                )),
            )
        );
    }

    #[test]
    fn deeply_nested_parentheses_are_rejected() {
        let mut tokens = vec!["("; crate::parser::nesting::MAX_NESTING_DEPTH + 1];
        tokens.push("value");
        tokens.extend(std::iter::repeat_n(
            ")",
            crate::parser::nesting::MAX_NESTING_DEPTH + 1,
        ));

        assert!(matches!(
            parse(&tokens),
            Err(error::TestCommandParseError::NestingLimitExceeded { .. })
        ));
    }

    #[test]
    fn binary_test_display_uses_operand_operator_order() {
        let expression = ast::TestExpr::BinaryTest(
            ast::BinaryPredicate::ArithmeticEqualTo,
            "1".to_owned(),
            "2".to_owned(),
        );

        assert_eq!(expression.to_string(), "1 -eq 2");
    }
}
