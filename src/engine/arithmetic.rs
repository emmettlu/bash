//! Arithmetic evaluation

use std::borrow::Cow;

use crate::engine::{ExecutionParameters, Shell, env, expansion, variables};
use crate::parser::ast;

/// Maximum recursion depth for arithmetic variable dereference chains
/// (e.g., a=b, b=c, c=a would cycle through variable dereferences).
const MAX_VARIABLE_DEREF_DEPTH: u32 = 1024;

/// Represents an error that occurs during evaluation of an arithmetic expression.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    /// Division by zero.
    #[error("division by zero")]
    DivideByZero,

    /// Negative exponent.
    #[error("exponent less than 0")]
    NegativeExponent,

    /// Failed to tokenize an arithmetic expression.
    #[error("failed to tokenize expression")]
    FailedToTokenizeExpression,

    /// Failed to expand an arithmetic expression.
    #[error("failed to expand expression: {0}")]
    FailedToExpandExpression(String),

    /// Failed to access an element of an array.
    #[error("failed to access array")]
    FailedToAccessArray,

    /// Failed to update the shell environment in an assignment operator.
    #[error("failed to update environment: {0}")]
    FailedToUpdateEnvironment(String),

    /// Failed to parse an arithmetic expression.
    #[error("failed to parse expression: {0}")]
    ParseError(String),

    /// Error expanding an unset variable.
    #[error("expanding unset variable: {0}")]
    ExpandingUnsetVariable(String),

    /// Failed to resolve a variable or nameref target.
    #[error("failed to resolve variable: {0}")]
    FailedToResolveVariable(String),

    /// Expression recursion level exceeded.
    #[error("expression recursion level exceeded")]
    RecursionLimitExceeded,
}

/// Trait implemented by arithmetic expressions that can be evaluated.
pub(crate) trait ExpandAndEvaluate {
    /// Evaluate the given expression, returning the resulting numeric value.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell to use for evaluation.
    /// * `trace_if_needed` - Whether to trace the evaluation.
    async fn eval(
        &self,
        shell: &mut Shell,
        params: &ExecutionParameters,
        trace_if_needed: bool,
    ) -> Result<i64, EvalError>;
}

impl ExpandAndEvaluate for ast::UnexpandedArithmeticExpr {
    async fn eval(
        &self,
        shell: &mut Shell,
        params: &ExecutionParameters,
        trace_if_needed: bool,
    ) -> Result<i64, EvalError> {
        expand_and_eval(shell, params, self.value.as_str(), trace_if_needed).await
    }
}

/// Evaluate the given arithmetic expression, returning the resulting numeric value.
///
/// # Arguments
///
/// * `shell` - The shell to use for evaluation.
/// * `expr` - The unexpanded arithmetic expression to evaluate.
/// * `trace_if_needed` - Whether to trace the evaluation.
pub(crate) async fn expand_and_eval(
    shell: &mut Shell,
    params: &ExecutionParameters,
    expr: &str,
    trace_if_needed: bool,
) -> Result<i64, EvalError> {
    // Per documentation, first shell-expand it.
    let options = expansion::ExpanderOptions {
        tilde_expand: false,
        ..Default::default()
    };
    let expanded_self = expansion::basic_expand_word_with_options(shell, params, expr, &options)
        .await
        .map_err(|_e| EvalError::FailedToExpandExpression(expr.to_owned()))?;

    // Now parse.
    let expr = crate::parser::arithmetic::parse(&expanded_self)
        .map_err(|_e| EvalError::ParseError(expanded_self))?;

    // Trace if applicable.
    if trace_if_needed && shell.options().print_commands_and_arguments {
        shell
            .trace_command(params, std::format!("(( {expr} ))"))
            .await;
    }

    // Now evaluate.
    expr.eval(shell)
}

/// Trait implemented by evaluatable arithmetic expressions.
pub trait Evaluatable {
    /// Evaluate the given arithmetic expression, returning the resulting numeric value.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell to use for evaluation.
    fn eval(&self, shell: &mut Shell) -> Result<i64, EvalError>;
}

impl Evaluatable for ast::ArithmeticExpr {
    fn eval(&self, shell: &mut Shell) -> Result<i64, EvalError> {
        eval_expr_impl(self, shell, 0)
    }
}

/// 使用现有算术解析器和求值器计算已完成 shell 展开的字符串.
pub(crate) fn eval_str(shell: &mut Shell, value: &str) -> Result<i64, EvalError> {
    let expression = crate::parser::arithmetic::parse(value)
        .map_err(|_| EvalError::ParseError(value.to_owned()))?;
    expression.eval(shell)
}

fn eval_expr_impl(
    expr: &ast::ArithmeticExpr,
    shell: &mut Shell,
    depth: u32,
) -> Result<i64, EvalError> {
    let value = match expr {
        ast::ArithmeticExpr::Literal(l) => *l,
        ast::ArithmeticExpr::Reference(lvalue) => deref_lvalue(shell, lvalue, depth)?,
        ast::ArithmeticExpr::UnaryOp(op, operand) => apply_unary_op(shell, *op, operand, depth)?,
        ast::ArithmeticExpr::BinaryOp(op, left, right) => {
            apply_binary_op(shell, *op, left, right, depth)?
        }
        ast::ArithmeticExpr::Conditional(condition, then_expr, else_expr) => {
            let conditional_eval = eval_expr_impl(condition, shell, depth)?;

            // Ensure we only evaluate the branch indicated by the condition.
            if conditional_eval != 0 {
                eval_expr_impl(then_expr, shell, depth)?
            } else {
                eval_expr_impl(else_expr, shell, depth)?
            }
        }
        ast::ArithmeticExpr::Assignment(lvalue, rhs) => {
            let expr_eval = eval_expr_impl(rhs, shell, depth)?;
            assign(shell, lvalue, expr_eval, depth)?
        }
        ast::ArithmeticExpr::UnaryAssignment(op, lvalue) => {
            apply_unary_assignment_op(shell, lvalue, *op, depth)?
        }
        ast::ArithmeticExpr::BinaryAssignment(op, lvalue, operand) => {
            let resolved_lvalue = resolve_lvalue(shell, lvalue, depth)?;
            let left = deref_resolved_lvalue(shell, &resolved_lvalue, depth)?;
            let right = eval_expr_impl(operand, shell, depth)?;
            let value = apply_binary_op_to_values(*op, left, right)?;
            assign_resolved_lvalue(shell, resolved_lvalue, value)?
        }
    };

    Ok(value)
}

fn get_var_value<'a>(shell: &'a Shell, name: &str) -> Result<Cow<'a, str>, EvalError> {
    let resolved = shell
        .env()
        .get_resolved(name)
        .map_err(|error| EvalError::FailedToResolveVariable(error.to_string()))?;

    if let Some((_, variable, index)) = resolved {
        let value = variable.resolve_value(shell);
        let value = if let Some(index) = index {
            value
                .get_at(index.as_str(), shell)
                .map_err(|error| EvalError::FailedToResolveVariable(error.to_string()))?
                .map(|value| Cow::Owned(value.into_owned()))
        } else {
            value
                .try_get_cow_str(shell)
                .map(|value| Cow::Owned(value.into_owned()))
        };
        if let Some(value) = value {
            return Ok(value);
        }
    }

    if shell.options().treat_unset_variables_as_error {
        return Err(EvalError::ExpandingUnsetVariable(name.into()));
    }

    Ok("".into())
}

enum ResolvedArithmeticTarget {
    Variable(String),
    ArrayElement { name: String, index: String },
}

fn resolve_lvalue(
    shell: &mut Shell,
    lvalue: &ast::ArithmeticTarget,
    depth: u32,
) -> Result<ResolvedArithmeticTarget, EvalError> {
    match lvalue {
        ast::ArithmeticTarget::Variable(name) => {
            let target = shell
                .env()
                .resolve_target(name)
                .map_err(|error| EvalError::FailedToResolveVariable(error.to_string()))?;
            if let Some(index) = target.index {
                Ok(ResolvedArithmeticTarget::ArrayElement {
                    name: target.name,
                    index,
                })
            } else {
                Ok(ResolvedArithmeticTarget::Variable(target.name))
            }
        }
        ast::ArithmeticTarget::ArrayElement(name, index_expr) => {
            let target = shell
                .env()
                .resolve_target(name)
                .map_err(|error| EvalError::FailedToResolveVariable(error.to_string()))?;
            if target.index.is_some() {
                return Err(EvalError::FailedToResolveVariable(
                    "combining an arithmetic subscript with an array-element nameref is unsupported"
                        .into(),
                ));
            }
            let index = eval_expr_impl(index_expr, shell, depth)?.to_string();
            Ok(ResolvedArithmeticTarget::ArrayElement {
                name: target.name,
                index,
            })
        }
    }
}

fn deref_lvalue(
    shell: &mut Shell,
    lvalue: &ast::ArithmeticTarget,
    depth: u32,
) -> Result<i64, EvalError> {
    let resolved_lvalue = resolve_lvalue(shell, lvalue, depth)?;
    deref_resolved_lvalue(shell, &resolved_lvalue, depth)
}

fn deref_resolved_lvalue(
    shell: &mut Shell,
    lvalue: &ResolvedArithmeticTarget,
    depth: u32,
) -> Result<i64, EvalError> {
    let value_str: Cow<'_, str> = match lvalue {
        ResolvedArithmeticTarget::Variable(name) => get_var_value(shell, name)?,
        ResolvedArithmeticTarget::ArrayElement { name, index } => shell
            .env()
            .get(name)
            .map_or_else(
                || Ok(None),
                |(_, variable)| variable.value().get_at(index.as_str(), shell),
            )
            .map_err(|_err| EvalError::FailedToAccessArray)?
            .unwrap_or(Cow::Borrowed("")),
    };

    let parsed_value = crate::parser::arithmetic::parse(value_str.as_ref())
        .map_err(|_err| EvalError::ParseError(value_str.to_string()))?;

    // 字面量不会继续递归, 仅在变量值需要进一步求值时增加深度.
    if matches!(parsed_value, ast::ArithmeticExpr::Literal(_)) {
        return eval_expr_impl(&parsed_value, shell, depth);
    }

    if depth >= MAX_VARIABLE_DEREF_DEPTH {
        return Err(EvalError::RecursionLimitExceeded);
    }

    eval_expr_impl(&parsed_value, shell, depth + 1)
}

fn apply_unary_op(
    shell: &mut Shell,
    op: ast::UnaryOperator,
    operand: &ast::ArithmeticExpr,
    depth: u32,
) -> Result<i64, EvalError> {
    let operand_eval = eval_expr_impl(operand, shell, depth)?;

    match op {
        ast::UnaryOperator::UnaryPlus => Ok(operand_eval),
        ast::UnaryOperator::UnaryMinus => Ok(operand_eval.wrapping_neg()),
        ast::UnaryOperator::BitwiseNot => Ok(!operand_eval),
        ast::UnaryOperator::LogicalNot => Ok(bool_to_i64(operand_eval == 0)),
    }
}

fn apply_binary_op(
    shell: &mut Shell,
    op: ast::BinaryOperator,
    left: &ast::ArithmeticExpr,
    right: &ast::ArithmeticExpr,
    depth: u32,
) -> Result<i64, EvalError> {
    // First, special-case short-circuiting operators. For those, we need
    // to ensure we don't eagerly evaluate both operands. After we
    // get these out of the way, we can easily just evaluate operands
    // for the other operators.
    match op {
        ast::BinaryOperator::LogicalAnd => {
            let left = eval_expr_impl(left, shell, depth)?;
            if left == 0 {
                return Ok(bool_to_i64(false));
            }

            let right = eval_expr_impl(right, shell, depth)?;
            return Ok(bool_to_i64(right != 0));
        }
        ast::BinaryOperator::LogicalOr => {
            let left = eval_expr_impl(left, shell, depth)?;
            if left != 0 {
                return Ok(bool_to_i64(true));
            }

            let right = eval_expr_impl(right, shell, depth)?;
            return Ok(bool_to_i64(right != 0));
        }
        _ => (),
    }

    // The remaining operators unconditionally operate both operands.
    let left = eval_expr_impl(left, shell, depth)?;
    let right = eval_expr_impl(right, shell, depth)?;
    apply_binary_op_to_values(op, left, right)
}

#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_sign_loss)]
fn apply_binary_op_to_values(
    op: ast::BinaryOperator,
    left: i64,
    right: i64,
) -> Result<i64, EvalError> {
    match op {
        ast::BinaryOperator::Power => {
            if right >= 0 {
                Ok(wrapping_pow_u64(left, right as u64))
            } else {
                Err(EvalError::NegativeExponent)
            }
        }
        ast::BinaryOperator::Multiply => Ok(left.wrapping_mul(right)),
        ast::BinaryOperator::Divide => {
            if right == 0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(left.wrapping_div(right))
            }
        }
        ast::BinaryOperator::Modulo => {
            if right == 0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(left.wrapping_rem(right))
            }
        }
        ast::BinaryOperator::Comma => Ok(right),
        ast::BinaryOperator::Add => Ok(left.wrapping_add(right)),
        ast::BinaryOperator::Subtract => Ok(left.wrapping_sub(right)),
        ast::BinaryOperator::ShiftLeft => Ok(left.wrapping_shl(right as u32)),
        ast::BinaryOperator::ShiftRight => Ok(left.wrapping_shr(right as u32)),
        ast::BinaryOperator::LessThan => Ok(bool_to_i64(left < right)),
        ast::BinaryOperator::LessThanOrEqualTo => Ok(bool_to_i64(left <= right)),
        ast::BinaryOperator::GreaterThan => Ok(bool_to_i64(left > right)),
        ast::BinaryOperator::GreaterThanOrEqualTo => Ok(bool_to_i64(left >= right)),
        ast::BinaryOperator::Equals => Ok(bool_to_i64(left == right)),
        ast::BinaryOperator::NotEquals => Ok(bool_to_i64(left != right)),
        ast::BinaryOperator::BitwiseAnd => Ok(left & right),
        ast::BinaryOperator::BitwiseXor => Ok(left ^ right),
        ast::BinaryOperator::BitwiseOr => Ok(left | right),
        ast::BinaryOperator::LogicalAnd => Ok(bool_to_i64(left != 0 && right != 0)),
        ast::BinaryOperator::LogicalOr => Ok(bool_to_i64(left != 0 || right != 0)),
    }
}

fn apply_unary_assignment_op(
    shell: &mut Shell,
    lvalue: &ast::ArithmeticTarget,
    op: ast::UnaryAssignmentOperator,
    depth: u32,
) -> Result<i64, EvalError> {
    let resolved_lvalue = resolve_lvalue(shell, lvalue, depth)?;
    let value = deref_resolved_lvalue(shell, &resolved_lvalue, depth)?;

    match op {
        ast::UnaryAssignmentOperator::PrefixIncrement => {
            let new_value = value.wrapping_add(1);
            assign_resolved_lvalue(shell, resolved_lvalue, new_value)?;
            Ok(new_value)
        }
        ast::UnaryAssignmentOperator::PrefixDecrement => {
            let new_value = value.wrapping_sub(1);
            assign_resolved_lvalue(shell, resolved_lvalue, new_value)?;
            Ok(new_value)
        }
        ast::UnaryAssignmentOperator::PostfixIncrement => {
            let new_value = value.wrapping_add(1);
            assign_resolved_lvalue(shell, resolved_lvalue, new_value)?;
            Ok(value)
        }
        ast::UnaryAssignmentOperator::PostfixDecrement => {
            let new_value = value.wrapping_sub(1);
            assign_resolved_lvalue(shell, resolved_lvalue, new_value)?;
            Ok(value)
        }
    }
}

fn assign(
    shell: &mut Shell,
    lvalue: &ast::ArithmeticTarget,
    value: i64,
    depth: u32,
) -> Result<i64, EvalError> {
    let resolved_lvalue = resolve_lvalue(shell, lvalue, depth)?;
    assign_resolved_lvalue(shell, resolved_lvalue, value)
}

fn assign_resolved_lvalue(
    shell: &mut Shell,
    lvalue: ResolvedArithmeticTarget,
    value: i64,
) -> Result<i64, EvalError> {
    match lvalue {
        ResolvedArithmeticTarget::Variable(name) => {
            shell
                .env_mut()
                .update_or_add(
                    name,
                    variables::ShellValueLiteral::Scalar(value.to_string()),
                    |_| Ok(()),
                    env::EnvironmentLookup::Anywhere,
                    env::EnvironmentScope::Global,
                )
                .map_err(|error| EvalError::FailedToUpdateEnvironment(error.to_string()))?;
        }
        ResolvedArithmeticTarget::ArrayElement { name, index } => {
            shell
                .env_mut()
                .update_or_add_array_element(
                    name,
                    index,
                    value.to_string(),
                    |_| Ok(()),
                    env::EnvironmentLookup::Anywhere,
                    env::EnvironmentScope::Global,
                )
                .map_err(|error| EvalError::FailedToUpdateEnvironment(error.to_string()))?;
        }
    }

    Ok(value)
}

const fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

// N.B. We implement our own version of wrapping_pow that takes a 64-bit exponent.
// This seems to be the best way to guarantee that we handle overflow cases
// with exponents correctly.
const fn wrapping_pow_u64(mut base: i64, mut exponent: u64) -> i64 {
    let mut result: i64 = 1;

    while exponent > 0 {
        if exponent % 2 == 1 {
            result = result.wrapping_mul(base);
        }

        base = base.wrapping_mul(base);
        exponent /= 2;
    }

    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::engine::variables::{ShellValue, ShellVariable};
    use anyhow::Result;

    fn scalar_value(shell: &Shell, name: &str) -> String {
        shell
            .env_var(name)
            .unwrap()
            .value()
            .to_cow_str(shell)
            .into_owned()
    }

    fn array_value(shell: &Shell, name: &str, index: &str) -> String {
        shell
            .env_var(name)
            .unwrap()
            .value()
            .get_at(index, shell)
            .unwrap()
            .unwrap()
            .into_owned()
    }

    fn reset_array_test_variables(shell: &mut Shell) -> Result<()> {
        shell.env_mut().set_global("i", ShellVariable::new("0"))?;
        shell.env_mut().set_global(
            "a",
            ShellVariable::new(ShellValue::indexed_array_from_strs(&["10", "20"])),
        )?;
        Ok(())
    }

    #[compio::test]
    async fn array_assignment_subscripts_are_evaluated_once() -> Result<()> {
        let mut shell = Shell::builder().build().await?;
        reset_array_test_variables(&mut shell)?;

        let expression = crate::parser::arithmetic::parse("a[i++]++")?;
        assert_eq!(expression.eval(&mut shell)?, 10);
        assert_eq!(scalar_value(&shell, "i"), "1");
        assert_eq!(array_value(&shell, "a", "0"), "11");
        assert_eq!(array_value(&shell, "a", "1"), "20");

        reset_array_test_variables(&mut shell)?;
        let expression = crate::parser::arithmetic::parse("a[i++] += 5")?;
        assert_eq!(expression.eval(&mut shell)?, 15);
        assert_eq!(scalar_value(&shell, "i"), "1");
        assert_eq!(array_value(&shell, "a", "0"), "15");
        assert_eq!(array_value(&shell, "a", "1"), "20");

        Ok(())
    }

    #[compio::test]
    async fn arithmetic_assignment_updates_dynamic_variables() -> Result<()> {
        let mut shell = Shell::builder().build().await?;

        let assign_random = crate::parser::arithmetic::parse("RANDOM = 1234")?;
        assert_eq!(assign_random.eval(&mut shell)?, 1234);
        let read_random = crate::parser::arithmetic::parse("RANDOM")?;
        let first = read_random.eval(&mut shell)?;
        assert_eq!(assign_random.eval(&mut shell)?, 1234);
        assert_eq!(read_random.eval(&mut shell)?, first);

        let assign_seconds = crate::parser::arithmetic::parse("SECONDS = -2")?;
        assert_eq!(assign_seconds.eval(&mut shell)?, -2);
        let seconds = crate::parser::arithmetic::parse("SECONDS")?.eval(&mut shell)?;
        assert!((-2..=0).contains(&seconds));

        let readonly = crate::parser::arithmetic::parse("BASHOPTS = 1")?;
        let error = readonly.eval(&mut shell).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot mutate readonly variable")
        );

        Ok(())
    }

    fn nameref(target: &str) -> ShellVariable {
        let mut variable = ShellVariable::new(target);
        variable.treat_as_nameref();
        variable
    }

    #[compio::test]
    async fn arithmetic_reads_and_assigns_through_namerefs() -> Result<()> {
        let mut shell = Shell::builder().build().await?;
        shell
            .env_mut()
            .set_global("target", ShellVariable::new("3"))?;
        shell.env_mut().set_global("reference", nameref("target"))?;

        assert_eq!(
            crate::parser::arithmetic::parse("reference + 2")?.eval(&mut shell)?,
            5
        );
        assert_eq!(
            crate::parser::arithmetic::parse("reference = 9")?.eval(&mut shell)?,
            9
        );
        assert_eq!(scalar_value(&shell, "target"), "9");

        shell.env_mut().set_global(
            "items",
            ShellVariable::new(ShellValue::indexed_array_from_strs(&["4", "7"])),
        )?;
        shell.env_mut().set_global("element", nameref("items[1]"))?;
        assert_eq!(
            crate::parser::arithmetic::parse("element")?.eval(&mut shell)?,
            7
        );
        assert_eq!(
            crate::parser::arithmetic::parse("element += 5")?.eval(&mut shell)?,
            12
        );
        assert_eq!(array_value(&shell, "items", "1"), "12");
        Ok(())
    }

    #[compio::test]
    async fn arithmetic_rejects_nameref_cycles() -> Result<()> {
        let mut shell = Shell::builder().build().await?;
        shell.env_mut().set_global("a", nameref("b"))?;
        shell.env_mut().set_global("b", nameref("a"))?;

        let error = crate::parser::arithmetic::parse("a")?
            .eval(&mut shell)
            .unwrap_err();
        assert!(error.to_string().contains("nameref cycle"));
        Ok(())
    }
}
