use std::collections::VecDeque;
use std::fmt;

use crate::parser::word;

const MAX_BRACE_NESTING_DEPTH: usize = 32;
const MAX_BRACE_EXPANSION_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BraceExpansionError {
    NestingDepth,
    ByteBudget,
}

impl fmt::Display for BraceExpansionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NestingDepth => f.write_str("brace expansion nesting depth exceeded"),
            Self::ByteBudget => f.write_str("brace expansion byte budget exceeded"),
        }
    }
}

#[derive(Debug)]
struct ExpansionBudget {
    generated_bytes: usize,
    max_bytes: usize,
}

impl ExpansionBudget {
    const fn new(max_bytes: usize) -> Self {
        Self {
            generated_bytes: 0,
            max_bytes,
        }
    }

    fn check_depth(depth: usize) -> Result<(), BraceExpansionError> {
        if depth > MAX_BRACE_NESTING_DEPTH {
            Err(BraceExpansionError::NestingDepth)
        } else {
            Ok(())
        }
    }

    fn ensure_result_capacity(&self, count: usize) -> Result<(), BraceExpansionError> {
        let allocation = count
            .checked_mul(std::mem::size_of::<String>())
            .ok_or(BraceExpansionError::ByteBudget)?;
        self.ensure_available(allocation)
    }

    fn charge_string(&mut self, value: &str) -> Result<(), BraceExpansionError> {
        let allocation = value
            .len()
            .checked_add(std::mem::size_of::<String>())
            .ok_or(BraceExpansionError::ByteBudget)?;
        self.ensure_available(allocation)?;
        self.generated_bytes += allocation;
        Ok(())
    }

    fn ensure_available(&self, allocation: usize) -> Result<(), BraceExpansionError> {
        if self
            .generated_bytes
            .checked_add(allocation)
            .is_none_or(|total| total > self.max_bytes)
        {
            Err(BraceExpansionError::ByteBudget)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct NumberSequenceFormat {
    start: i64,
    end: i64,
    width: Option<usize>,
}

pub(crate) fn validate_brace_source(source: &str) -> Result<(), BraceExpansionError> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for c in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if c == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(c, '\'' | '"') {
            quote = Some(c);
            continue;
        }

        match c {
            '{' => {
                depth = depth
                    .checked_add(1)
                    .ok_or(BraceExpansionError::NestingDepth)?;
                ExpansionBudget::check_depth(depth)?;
            }
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }

    Ok(())
}

pub(crate) fn generate_and_combine_brace_expansions(
    pieces: Vec<word::BraceExpressionOrText>,
    source: &str,
) -> Result<Vec<String>, BraceExpansionError> {
    generate_and_combine_brace_expansions_with_budget(pieces, source, MAX_BRACE_EXPANSION_BYTES)
}

fn generate_and_combine_brace_expansions_with_budget(
    pieces: Vec<word::BraceExpressionOrText>,
    source: &str,
    max_bytes: usize,
) -> Result<Vec<String>, BraceExpansionError> {
    let mut budget = ExpansionBudget::new(max_bytes);
    let mut number_formats = find_number_sequence_formats(source);
    expand_pieces(pieces, 0, &mut budget, &mut number_formats)
}

fn expand_pieces(
    pieces: Vec<word::BraceExpressionOrText>,
    depth: usize,
    budget: &mut ExpansionBudget,
    number_formats: &mut VecDeque<NumberSequenceFormat>,
) -> Result<Vec<String>, BraceExpansionError> {
    ExpansionBudget::check_depth(depth)?;
    let mut combined = vec![String::new()];

    for piece in pieces {
        let alternatives = expand_brace_expr_or_text(piece, depth, budget, number_formats)?;
        let result_count = combined
            .len()
            .checked_mul(alternatives.len())
            .ok_or(BraceExpansionError::ByteBudget)?;
        budget.ensure_result_capacity(result_count)?;

        let mut next = Vec::with_capacity(result_count);
        for prefix in &combined {
            for suffix in &alternatives {
                let mut value = String::with_capacity(prefix.len() + suffix.len());
                value.push_str(prefix);
                value.push_str(suffix);
                budget.charge_string(&value)?;
                next.push(value);
            }
        }
        combined = next;
    }

    Ok(combined)
}

fn expand_brace_expr_or_text(
    beot: word::BraceExpressionOrText,
    depth: usize,
    budget: &mut ExpansionBudget,
    number_formats: &mut VecDeque<NumberSequenceFormat>,
) -> Result<Vec<String>, BraceExpansionError> {
    match beot {
        word::BraceExpressionOrText::Expr(members) => {
            let mut alternatives = vec![];
            for member in members {
                alternatives.extend(expand_brace_expr_member(
                    member,
                    depth,
                    budget,
                    number_formats,
                )?);
            }
            Ok(alternatives)
        }
        word::BraceExpressionOrText::Text(text) => {
            budget.charge_string(&text)?;
            Ok(vec![text])
        }
    }
}

fn expand_brace_expr_member(
    bem: word::BraceExpressionMember,
    depth: usize,
    budget: &mut ExpansionBudget,
    number_formats: &mut VecDeque<NumberSequenceFormat>,
) -> Result<Vec<String>, BraceExpansionError> {
    match bem {
        word::BraceExpressionMember::NumberSequence {
            start,
            end,
            increment,
        } => {
            let width = take_number_sequence_width(number_formats, start, end);
            expand_number_sequence(start, end, increment, width, budget)
        }
        word::BraceExpressionMember::CharSequence {
            start,
            end,
            increment,
        } => expand_char_sequence(start, end, increment, budget),
        word::BraceExpressionMember::Child(elements) => {
            let child_depth = depth
                .checked_add(1)
                .ok_or(BraceExpansionError::NestingDepth)?;
            expand_pieces(elements, child_depth, budget, number_formats)
        }
    }
}

fn expand_number_sequence(
    start: i64,
    end: i64,
    increment: i64,
    width: Option<usize>,
    budget: &mut ExpansionBudget,
) -> Result<Vec<String>, BraceExpansionError> {
    let step = i128::from(increment).abs().max(1);
    let end = i128::from(end);
    let ascending = i128::from(start) <= end;
    let mut current = i128::from(start);
    let mut result = vec![];

    while if ascending {
        current <= end
    } else {
        current >= end
    } {
        let current_value = i64::try_from(current).map_err(|_| BraceExpansionError::ByteBudget)?;
        let value = width.map_or_else(
            || current_value.to_string(),
            |width| format!("{current_value:0width$}"),
        );
        budget.charge_string(&value)?;
        result.push(value);

        current = if ascending {
            current + step
        } else {
            current - step
        };
    }

    Ok(result)
}

fn expand_char_sequence(
    start: char,
    end: char,
    increment: i64,
    budget: &mut ExpansionBudget,
) -> Result<Vec<String>, BraceExpansionError> {
    let step = i128::from(increment).abs().max(1);
    let start_value = u32::from(start);
    let end_value = u32::from(end);
    let ascending = start_value <= end_value;
    let end = i128::from(end_value);
    let mut current = i128::from(start_value);
    let mut result = vec![];

    while if ascending {
        current <= end
    } else {
        current >= end
    } {
        let value = u32::try_from(current)
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .ok_or(BraceExpansionError::ByteBudget)?;
        budget.charge_string(&value)?;
        result.push(value);

        current = if ascending {
            current + step
        } else {
            current - step
        };
    }

    Ok(result)
}

fn take_number_sequence_width(
    formats: &mut VecDeque<NumberSequenceFormat>,
    start: i64,
    end: i64,
) -> Option<usize> {
    let position = formats
        .iter()
        .position(|format| format.start == start && format.end == end)?;
    formats.remove(position).and_then(|format| format.width)
}

fn find_number_sequence_formats(source: &str) -> VecDeque<NumberSequenceFormat> {
    let mut formats = VecDeque::new();
    let mut quote = None;
    let mut escaped = false;
    let mut previous = None;

    for (index, c) in source.char_indices() {
        if escaped {
            escaped = false;
            previous = Some(c);
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            previous = Some(c);
            continue;
        }
        if let Some(active_quote) = quote {
            if c == active_quote {
                quote = None;
            }
            previous = Some(c);
            continue;
        }
        if matches!(c, '\'' | '"') {
            quote = Some(c);
            previous = Some(c);
            continue;
        }

        if c == '{'
            && previous != Some('$')
            && let Some(end_index) = find_matching_brace(source, index)
            && let Some(format) = parse_number_sequence_format(&source[index + 1..end_index])
        {
            formats.push_back(format);
        }
        previous = Some(c);
    }

    formats
}

fn find_matching_brace(source: &str, opening_index: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for (relative_index, c) in source[opening_index..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if c == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(c, '\'' | '"') {
            quote = Some(c);
            continue;
        }

        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(opening_index + relative_index);
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_number_sequence_format(inner: &str) -> Option<NumberSequenceFormat> {
    let parts: Vec<_> = inner.split("..").collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|part| !is_signed_decimal(part)) {
        return None;
    }

    let start = parts[0].parse().ok()?;
    let end = parts[1].parse().ok()?;
    let start_digits = parts[0].trim_start_matches(['+', '-']);
    let end_digits = parts[1].trim_start_matches(['+', '-']);
    let padded = (start_digits.len() > 1 && start_digits.starts_with('0'))
        || (end_digits.len() > 1 && end_digits.starts_with('0'));
    let width = padded.then(|| parts[0].len().max(parts[1].len()));

    Some(NumberSequenceFormat { start, end, width })
}

fn is_signed_decimal(value: &str) -> bool {
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number_sequence(start: i64, end: i64) -> Vec<word::BraceExpressionOrText> {
        vec![word::BraceExpressionOrText::Expr(vec![
            word::BraceExpressionMember::NumberSequence {
                start,
                end,
                increment: 1,
            },
        ])]
    }

    #[test]
    fn preserves_numeric_sequence_width() {
        assert_eq!(
            generate_and_combine_brace_expansions(number_sequence(1, 3), "{001..3}"),
            Ok(vec!["001".into(), "002".into(), "003".into()])
        );
        assert_eq!(
            generate_and_combine_brace_expansions(number_sequence(-2, 2), "{-02..2}"),
            Ok(vec![
                "-02".into(),
                "-01".into(),
                "000".into(),
                "001".into(),
                "002".into(),
            ])
        );
    }

    #[test]
    fn expands_more_than_4096_results() {
        let result = generate_and_combine_brace_expansions(number_sequence(1, 4097), "{1..4097}");

        assert_eq!(result.as_ref().map(Vec::len), Ok(4097));
        assert_eq!(
            result
                .as_ref()
                .ok()
                .and_then(|values| values.first())
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            result
                .as_ref()
                .ok()
                .and_then(|values| values.last())
                .map(String::as_str),
            Some("4097")
        );
    }

    #[test]
    fn limits_accumulated_bytes() {
        let oversized = "x".repeat(64);
        assert_eq!(
            generate_and_combine_brace_expansions_with_budget(
                vec![word::BraceExpressionOrText::Text(oversized.clone())],
                &oversized,
                128,
            ),
            Err(BraceExpansionError::ByteBudget)
        );
    }

    #[test]
    fn limits_source_nesting_depth() {
        let source = format!(
            "{}x{}",
            "{".repeat(MAX_BRACE_NESTING_DEPTH + 1),
            "}".repeat(MAX_BRACE_NESTING_DEPTH + 1)
        );
        assert_eq!(
            validate_brace_source(&source),
            Err(BraceExpansionError::NestingDepth)
        );
    }
}
