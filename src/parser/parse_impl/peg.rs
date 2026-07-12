//! PEG-based parser implementation for shell scripts.

use crate::parser::SourceSpan;
use crate::parser::ast::{self, SeparatorOperator, SourceLocation, maybe_location};
use crate::parser::tokenizer::Token;
use crate::parser::word;

use super::Tokens;

peg::parser! {
    pub grammar token_parser<'a>() for Tokens<'a> {
        pub(crate) rule program() -> ast::Program =
            linebreak() c:complete_commands() linebreak() { ast::Program { complete_commands: c } } /
            linebreak() { ast::Program { complete_commands: vec![] } }

        rule complete_commands() -> Vec<ast::CompleteCommand> =
            c:complete_command() ++ newline_list()

        rule complete_command() -> ast::CompleteCommand =
            first:and_or() remainder:(s:separator_op() l:and_or() { (s, l) })* last_sep:separator_op()? {
                let mut and_ors = vec![first];
                let mut seps = vec![];

                for (sep, ao) in remainder {
                    seps.push(sep);
                    and_ors.push(ao);
                }

                // N.B. We default to synchronous if no separator op is given.
                seps.push(last_sep.unwrap_or(SeparatorOperator::Sequence));

                let mut items = vec![];
                for (i, ao) in and_ors.into_iter().enumerate() {
                    items.push(ast::CompoundListItem(ao, seps[i].clone()));
                }

                ast::CompoundList(items)
            }

        rule and_or() -> ast::AndOrList =
            first:pipeline() additional:_and_or_item()* { ast::AndOrList { first, additional } }

        rule _and_or_item() -> ast::AndOr =
            op:_and_or_op() linebreak() p:pipeline() { op(p) }

        rule _and_or_op() -> fn(ast::Pipeline) -> ast::AndOr =
            specific_operator("&&") { ast::AndOr::And } /
            specific_operator("||") { ast::AndOr::Or }

        rule pipeline() -> ast::Pipeline =
            timed:pipeline_timed()? bang:bang()* pipeline:pipe_sequence() {?
                let (seq, pipe_kinds) = pipeline;
                if timed.is_none() && bang.is_empty() && seq.is_empty() {
                    Err("empty pipeline")
                } else {
                    let invert = bang.len() % 2 == 1;
                    Ok(ast::Pipeline { timed, bang: invert, seq, pipe_kinds })
                }
            }

        rule pipeline_timed() -> ast::PipelineTimed =
            s:specific_word("time") posix_output:specific_word("-p")? {
                let start = s.location();
                if let Some(end) = posix_output {
                    ast::PipelineTimed::TimedWithPosixOutput(SourceSpan::within(start, end.location()))
                } else {
                    ast::PipelineTimed::Timed(start.to_owned())
                }
            }

        rule bang() -> bool = specific_word("!") { true }

        pub(crate) rule pipe_sequence() -> (Vec<ast::Command>, Vec<ast::PipeKind>) =
            first:command() additional:(kind:pipe_operator() linebreak() command:command() { (kind, command) })* {
                let mut commands = Vec::with_capacity(additional.len() + 1);
                let mut pipe_kinds = Vec::with_capacity(additional.len());
                commands.push(first);

                for (kind, command) in additional {
                    if matches!(kind, ast::PipeKind::StdoutAndStderr(_)) {
                        add_pipe_extension_redirection(
                            commands.last_mut().expect("a pipeline always has a preceding command")
                        );
                    }
                    pipe_kinds.push(kind);
                    commands.push(command);
                }

                (commands, pipe_kinds)
            }

        rule pipe_operator() -> ast::PipeKind =
            p:specific_operator("|") { ast::PipeKind::Stdout(p.location().clone()) } /
            p:specific_operator("|&") { ast::PipeKind::StdoutAndStderr(p.location().clone()) }

        // N.B. We needed to move the function definition branch up to avoid conflicts with array assignment syntax.
        rule command() -> ast::Command =
            f:function_definition() { ast::Command::Function(f) } /
            c:simple_command() { ast::Command::Simple(c) } /
            c:compound_command() r:redirect_list()? { ast::Command::Compound(c, r) } /
            // N.B. Extended test commands are bash extensions.
            c:extended_test_command() r:redirect_list()? { ast::Command::ExtendedTest(c, r) } /
            expected!("command")

        // N.B. The arithmetic command is a non-sh extension.
        // N.B. The arithmetic for clause command is a non-sh extension.
        pub(crate) rule compound_command() -> ast::CompoundCommand =
            a:arithmetic_command() { ast::CompoundCommand::Arithmetic(a) } /
            c:coproc_clause() { ast::CompoundCommand::Coprocess(c) } /
            b:brace_group() { ast::CompoundCommand::BraceGroup(b) } /
            s:subshell() { ast::CompoundCommand::Subshell(s) } /
            f:for_clause() { ast::CompoundCommand::ForClause(f) } /
            c:case_clause() { ast::CompoundCommand::CaseClause(c) } /
            i:if_clause() { ast::CompoundCommand::IfClause(i) } /
            w:while_clause() { ast::CompoundCommand::WhileClause(w) } /
            u:until_clause() { ast::CompoundCommand::UntilClause(u) } /
            c:arithmetic_for_clause() { ast::CompoundCommand::ArithmeticForClause(c) } /
            expected!("compound command")

        pub(crate) rule arithmetic_command() -> ast::ArithmeticCommand =
            start:specific_operator("(") specific_operator("(") expr:arithmetic_expression() specific_operator(")") end:specific_operator(")") {
                let loc = SourceSpan::within(
                    start.location(),
                    end.location()
                );
                ast::ArithmeticCommand { expr, loc }
            }

        pub(crate) rule arithmetic_expression() -> ast::UnexpandedArithmeticExpr =
            raw_expr:$(arithmetic_expression_piece()*) { ast::UnexpandedArithmeticExpr { value: raw_expr } }

        rule arithmetic_expression_piece() =
            // Allow a parenthesized expression (with matching opening and closing parens).
            specific_operator("(") (!specific_operator(")") arithmetic_expression_piece())* specific_operator(")") {} /
            // Otherwise consume any token that's neither the normal end of the entire arithmetic expression, nor an
            // unexpected mismatched closing parenthesis. In the latter case, it may be that this really was never an
            // arithmetic expression in the first place and we need to backtrack and instead try parsing as a subshell
            // command instead.
            !arithmetic_end() !specific_operator(")") [_] {}

        // TODO(arithmetic): evaluate arithmetic end; the semicolon is used in arithmetic for loops.
        rule arithmetic_end() -> () =
            specific_operator(")") specific_operator(")") {} /
            specific_operator(";") {}

        rule subshell() -> ast::SubshellCommand =
            start:specific_operator("(") list:compound_list() end:specific_operator(")") {
                let loc = SourceSpan::within(start.location(), end.location());
                ast::SubshellCommand { list, loc }
            }

        rule compound_list() -> ast::CompoundList =
            linebreak() first:and_or() remainder:(s:separator() l:and_or() { (s, l) })* last_sep:separator()? {
                let mut and_ors = vec![first];
                let mut seps = vec![];

                for (sep, ao) in remainder {
                    seps.push(sep.unwrap_or(SeparatorOperator::Sequence));
                    and_ors.push(ao);
                }

                // N.B. We default to synchronous if no separator op is given.
                let last_sep = last_sep.unwrap_or(None);
                seps.push(last_sep.unwrap_or(SeparatorOperator::Sequence));

                let mut items = vec![];
                for (i, ao) in and_ors.into_iter().enumerate() {
                    items.push(ast::CompoundListItem(ao, seps[i].clone()));
                }

                ast::CompoundList(items)
            }

        rule for_clause() -> ast::ForClauseCommand =
            s:specific_word("for") n:name() linebreak() _in() w:wordlist()? sequential_sep() d:do_group() {
                let start = s.location();
                let end = &d.loc;
                let loc = SourceSpan::within(start, end);
                ast::ForClauseCommand { variable_name: n.to_owned(), values: w, body: d, loc }
            } /
            s:specific_word("for") n:name() sequential_sep()? d:do_group() {
                let start = s.location();
                let end = &d.loc;
                let loc = SourceSpan::within(start, end);
                ast::ForClauseCommand { variable_name: n.to_owned(), values: None, body: d, loc }
            }

        // N.B. The arithmetic for loop is a non-sh extension.
        rule arithmetic_for_clause() -> ast::ArithmeticForClauseCommand =
            s:specific_word("for")
            specific_operator("(") specific_operator("(")
                initializer:arithmetic_expression()? specific_operator(";")
                condition:arithmetic_expression()? specific_operator(";")
                updater:arithmetic_expression()?
            specific_operator(")") specific_operator(")")
            body:arithmetic_for_body() {
                let start = s.location();
                let end = &body.loc;
                let loc = SourceSpan::within(start, end);
                ast::ArithmeticForClauseCommand { initializer, condition, updater, body, loc }
            }

        rule arithmetic_for_body() -> ast::DoGroupCommand =
            sequential_sep()? body:do_group() { body } /
            body:brace_group() { ast::DoGroupCommand { list: body.list, loc: body.loc } }

        rule extended_test_command() -> ast::ExtendedTestExprCommand =
            s:specific_word("[[") linebreak() expr:extended_test_expression() linebreak() e:specific_word("]]") {
                let start = s.location();
                let end = e.location();
                let loc = SourceSpan::within(start, end);

                ast::ExtendedTestExprCommand { expr, loc }
            }

        rule extended_test_expression() -> ast::ExtendedTestExpr = precedence! {
            left:(@) linebreak() specific_operator("||") linebreak() right:@ { ast::ExtendedTestExpr::Or(Box::from(left), Box::from(right)) }
            --
            left:(@) linebreak() specific_operator("&&") linebreak() right:@ { ast::ExtendedTestExpr::And(Box::from(left), Box::from(right)) }
            --
            specific_word("!") e:@ { ast::ExtendedTestExpr::Not(Box::from(e)) }
            --
            specific_operator("(") e:extended_test_expression() specific_operator(")") { ast::ExtendedTestExpr::Parenthesized(Box::from(e)) }
            --
            // Arithmetic operators
            left:word() specific_word("-eq") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::ArithmeticEqualTo, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("-ne") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::ArithmeticNotEqualTo, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("-lt") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::ArithmeticLessThan, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("-le") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::ArithmeticLessThanOrEqualTo, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("-gt") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::ArithmeticGreaterThan, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("-ge") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::ArithmeticGreaterThanOrEqualTo, ast::Word::from(left), ast::Word::from(right)) }
            // Non-arithmetic binary operators
            left:word() specific_word("-ef") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::FilesReferToSameDeviceAndInodeNumbers, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("-nt") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::LeftFileIsNewerOrExistsWhenRightDoesNot, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("-ot") right:word() { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::LeftFileIsOlderOrDoesNotExistWhenRightDoes, ast::Word::from(left), ast::Word::from(right)) }
            left:word() (specific_word("==") / specific_word("=")) right:word()  { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::StringExactlyMatchesPattern, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("!=") right:word()  { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::StringDoesNotExactlyMatchPattern, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_word("=~") right:regex_word()  {
                if right.value.starts_with(['\'', '\"']) {
                    // TODO(test): Confirm it ends with that too?
                    ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::StringContainsSubstring, ast::Word::from(left), right)
                } else {
                    ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::StringMatchesRegex, ast::Word::from(left), right)
                }
            }
            left:word() specific_operator("<") right:word()   { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::LeftSortsBeforeRight, ast::Word::from(left), ast::Word::from(right)) }
            left:word() specific_operator(">") right:word()   { ast::ExtendedTestExpr::BinaryTest(ast::BinaryPredicate::LeftSortsAfterRight, ast::Word::from(left), ast::Word::from(right)) }
            --
            p:extended_unary_predicate() f:word() { ast::ExtendedTestExpr::UnaryTest(p, ast::Word::from(f)) }
            --
            w:word() { ast::ExtendedTestExpr::UnaryTest(ast::UnaryPredicate::StringHasNonZeroLength, ast::Word::from(w)) }
        }

        rule extended_unary_predicate() -> ast::UnaryPredicate =
            specific_word("-a") { ast::UnaryPredicate::FileExists } /
            specific_word("-b") { ast::UnaryPredicate::FileExistsAndIsBlockSpecialFile } /
            specific_word("-c") { ast::UnaryPredicate::FileExistsAndIsCharSpecialFile } /
            specific_word("-d") { ast::UnaryPredicate::FileExistsAndIsDir } /
            specific_word("-e") { ast::UnaryPredicate::FileExists } /
            specific_word("-f") { ast::UnaryPredicate::FileExistsAndIsRegularFile } /
            specific_word("-g") { ast::UnaryPredicate::FileExistsAndIsSetgid } /
            specific_word("-h") { ast::UnaryPredicate::FileExistsAndIsSymlink } /
            specific_word("-k") { ast::UnaryPredicate::FileExistsAndHasStickyBit } /
            specific_word("-n") { ast::UnaryPredicate::StringHasNonZeroLength } /
            specific_word("-o") { ast::UnaryPredicate::ShellOptionEnabled } /
            specific_word("-p") { ast::UnaryPredicate::FileExistsAndIsFifo } /
            specific_word("-r") { ast::UnaryPredicate::FileExistsAndIsReadable } /
            specific_word("-s") { ast::UnaryPredicate::FileExistsAndIsNotZeroLength } /
            specific_word("-t") { ast::UnaryPredicate::FdIsOpenTerminal } /
            specific_word("-u") { ast::UnaryPredicate::FileExistsAndIsSetuid } /
            specific_word("-v") { ast::UnaryPredicate::ShellVariableIsSetAndAssigned } /
            specific_word("-w") { ast::UnaryPredicate::FileExistsAndIsWritable } /
            specific_word("-x") { ast::UnaryPredicate::FileExistsAndIsExecutable } /
            specific_word("-z") { ast::UnaryPredicate::StringHasZeroLength } /
            specific_word("-G") { ast::UnaryPredicate::FileExistsAndOwnedByEffectiveGroupId } /
            specific_word("-L") { ast::UnaryPredicate::FileExistsAndIsSymlink } /
            specific_word("-N") { ast::UnaryPredicate::FileExistsAndModifiedSinceLastRead } /
            specific_word("-O") { ast::UnaryPredicate::FileExistsAndOwnedByEffectiveUserId } /
            specific_word("-R") { ast::UnaryPredicate::ShellVariableIsSetAndNameRef } /
            specific_word("-S") { ast::UnaryPredicate::FileExistsAndIsSocket }

        // N.B. For some reason we seem to need to allow a select subset
        // of unescaped operators in regex words.
        rule regex_word() -> ast::Word =
            value:$((!specific_word("]]") regex_word_piece())+) {
                ast::Word::from(value)
            }

        rule regex_word_piece() =
            word() {} /
            specific_operator("|") {} /
            specific_operator("(") parenthesized_regex_word()* specific_operator(")") {}

        rule parenthesized_regex_word() =
            regex_word_piece() /
            !specific_operator(")") !specific_operator("]]") [_]

        rule name() -> &'input str =
            w:[Token::Word(_, _)] { w.to_str() }

        rule _in() -> () =
            specific_word("in") { }

        rule wordlist() -> Vec<ast::Word> =
            (w:word() { ast::Word::from(w) })+

        pub(crate) rule case_clause() -> ast::CaseClauseCommand =
            start:specific_word("case") w:word() linebreak() _in() linebreak() first_items:case_item()* last_item:case_item_ns()? end:specific_word("esac") {
                let mut cases = first_items;

                if let Some(last_item) = last_item {
                    cases.push(last_item);
                }

                let loc = SourceSpan::within(start.location(), end.location());
                ast::CaseClauseCommand { value: ast::Word::from(w), cases, loc }
            }

        pub(crate) rule case_item_ns() -> ast::CaseItem =
            s:specific_operator("(")? p:pattern() specific_operator(")") c:compound_list() {
                let start = s.map(Token::location).or_else(|| p.first().and_then(|w| w.loc.as_ref()));
                let end = c.location();

                let loc = maybe_location(start, end.as_ref());

                ast::CaseItem { patterns: p, cmd: Some(c), post_action: ast::CaseItemPostAction::ExitCase, loc }
            } /
            s:specific_operator("(")? p:pattern() e:specific_operator(")") linebreak() {
                let start = s.map(Token::location).or_else(|| p.first().and_then(|w| w.loc.as_ref()));
                let end = Some(e.location());

                let loc = maybe_location(start, end);
                ast::CaseItem { patterns: p, cmd: None, post_action: ast::CaseItemPostAction::ExitCase, loc }
            }

        pub(crate) rule case_item() -> ast::CaseItem =
            s:specific_operator("(")? p:pattern() specific_operator(")") linebreak() post_action:case_item_post_action() linebreak() {
                let start = s.map(Token::location).or_else(|| p.first().and_then(|w| w.loc.as_ref()));
                let end = Some(post_action.1);
                let loc = maybe_location(start, end);
                ast::CaseItem { patterns: p, cmd: None, post_action: post_action.0, loc }
            } /
            s:specific_operator("(")? p:pattern() specific_operator(")") c:compound_list() post_action:case_item_post_action() linebreak() {
                let start = s.map(Token::location).or_else(|| p.first().and_then(|w| w.loc.as_ref()));
                let end = Some(post_action.1);
                let loc = maybe_location(start, end);
                ast::CaseItem { patterns: p, cmd: Some(c), post_action: post_action.0, loc }
            }

        rule case_item_post_action() -> (ast::CaseItemPostAction, &'input SourceSpan)  =
            s:specific_operator(";;") {
                (ast::CaseItemPostAction::ExitCase, s.location())
            } /
            s:specific_operator(";;&") {
                (ast::CaseItemPostAction::ContinueEvaluatingCases, s.location())
            } /
            s:specific_operator(";&") {
                (ast::CaseItemPostAction::UnconditionallyExecuteNextCaseItem, s.location())
            }

        rule pattern() -> Vec<ast::Word> =
            (w:word() { ast::Word::from(w) }) ++ specific_operator("|")

        rule if_clause() -> ast::IfClauseCommand =
            s:specific_word("if") condition:compound_list() specific_word("then") then:compound_list() elses:else_part()? e:specific_word("fi") {
                let start = s.location();
                let end = e.location();
                let loc = SourceSpan::within(start, end);

                ast::IfClauseCommand {
                    condition,
                    then,
                    elses,
                    loc
                }
            }

        rule else_part() -> Vec<ast::ElseClause> =
            cs:_conditional_else_part()+ u:_unconditional_else_part()? {
                let mut parts = vec![];
                for c in cs {
                    parts.push(c);
                }

                if let Some(uncond) = u {
                    parts.push(uncond);
                }

                parts
            } /
            e:_unconditional_else_part() { vec![e] }

        rule _conditional_else_part() -> ast::ElseClause =
            specific_word("elif") condition:compound_list() specific_word("then") body:compound_list() {
                ast::ElseClause { condition: Some(condition), body }
            }

        rule _unconditional_else_part() -> ast::ElseClause =
            specific_word("else") body:compound_list() {
                ast::ElseClause { condition: None, body }
             }

        rule while_clause() -> ast::WhileOrUntilClauseCommand =
            s:specific_word("while") c:compound_list() d:do_group() {
                let start = s.location();
                let end = &d.loc;
                let loc = SourceSpan::within(start, end);

                ast::WhileOrUntilClauseCommand(c, d, loc)
            }

        rule until_clause() -> ast::WhileOrUntilClauseCommand =
            s:specific_word("until") c:compound_list() d:do_group() {
                let start = s.location();
                let end = &d.loc;
                let loc = SourceSpan::within(start, end);

                ast::WhileOrUntilClauseCommand(c, d, loc)
            }

        // N.B. Coproc is a bash extension.
        rule coproc_clause() -> ast::CoprocessCommand =
            s:specific_word("coproc") linebreak() name:coproc_name()? body:command() {
                let start = s.location();
                let loc = body.location().unwrap_or_else(|| start.clone());

                ast::CoprocessCommand { name, body: Box::new(body), loc: SourceSpan::within(start, &loc) }
            }

        // N.B. The name must be followed by a compound command start ({ or ()
        rule coproc_name() -> ast::Word =
            w:fname() linebreak() &(specific_word("{") / specific_operator("(")) {
                w
            }

        // N.B. Non-sh extensions allows use of the 'function' word to indicate a function definition.
        // N.B. Without the 'function' keyword, reserved words cannot be used as function names
        // (bash rejects e.g. `for (){ :; }`). With the 'function' keyword, reserved words are
        // allowed as function names (bash accepts e.g. `function for { :; }`).
        rule function_definition() -> ast::FunctionDefinition =
            specific_word("function") fname:fname() body:function_parens_and_body() {
                ast::FunctionDefinition { fname, body }
            } /
            fname:non_reserved_fname() body:function_parens_and_body() {
                ast::FunctionDefinition { fname, body }
            } /
            specific_word("function") fname:fname() linebreak() body:function_body() {
                ast::FunctionDefinition { fname, body }
            } /
            expected!("function definition")

        pub(crate) rule function_parens_and_body() -> ast::FunctionBody =
            specific_operator("(") specific_operator(")") linebreak() body:function_body() { body }

        rule function_body() -> ast::FunctionBody =
            c:compound_command() r:redirect_list()? { ast::FunctionBody(c, r) }

        rule fname() -> ast::Word =
            // Special-case: don't allow it to end with an equals sign, to avoid the challenge of
            // misinterpreting certain declaration assignments as function definitions.
            // TODO(parser): Find a way to make this still work without requiring this targeted exception.
            w:[Token::Word(word, l) if !word.ends_with('=')] { ast::Word::with_location(word, l) }

        rule non_reserved_fname() -> ast::Word =
            !reserved_word() w:fname() { w }

        rule brace_group() -> ast::BraceGroupCommand =
            start:specific_word("{") list:compound_list() end:specific_word("}") {
                let loc = SourceSpan::within(start.location(), end.location());
                ast::BraceGroupCommand { list, loc }
            }

        rule do_group() -> ast::DoGroupCommand =
            start:specific_word("do") list:compound_list() end:specific_word("done") {
                let loc = SourceSpan::within(start.location(), end.location());
                ast::DoGroupCommand { list, loc }
            }

        rule simple_command() -> ast::SimpleCommand =
            prefix:cmd_prefix() word_and_suffix:(word_or_name:cmd_word() suffix:cmd_suffix()? { (word_or_name, suffix) })? {
                match word_and_suffix {
                    Some((word_or_name, suffix)) => {
                        ast::SimpleCommand { prefix: Some(prefix), word_or_name: Some(ast::Word::from(word_or_name)), suffix }
                    }
                    None => {
                        ast::SimpleCommand { prefix: Some(prefix), word_or_name: None, suffix: None }
                    }
                }
            } /
            word_or_name:cmd_name() suffix:cmd_suffix()? {
                ast::SimpleCommand { prefix: None, word_or_name: Some(ast::Word::from(word_or_name)), suffix } } /
            expected!("simple command")

        rule cmd_name() -> &'input Token =
            !overflowing_io_number() w:non_reserved_word() { w }

        rule cmd_word() -> &'input Token =
            !overflowing_io_number() !assignment_word() w:non_reserved_word() { w }

        rule command_word() -> &'input Token =
            !overflowing_io_number() w:word() { w }

        rule cmd_prefix() -> ast::CommandPrefix =
            p:(
                i:io_redirect() { ast::CommandPrefixOrSuffixItem::IoRedirect(i) } /
                assignment_and_word:assignment_word() {
                    let (assignment, word) = assignment_and_word;
                    ast::CommandPrefixOrSuffixItem::AssignmentWord(assignment, word)
                }
            )+ { ast::CommandPrefix(p) }

        rule cmd_suffix() -> ast::CommandSuffix =
            s:(
                sub:process_substitution() {
                    let (kind, subshell) = sub;
                    ast::CommandPrefixOrSuffixItem::ProcessSubstitution(kind, subshell)
                } /
                i:io_redirect() {
                    ast::CommandPrefixOrSuffixItem::IoRedirect(i)
                } /
                assignment_and_word:assignment_word() {
                    let (assignment, word) = assignment_and_word;
                    ast::CommandPrefixOrSuffixItem::AssignmentWord(assignment, word)
                } /
                w:command_word() {
                    ast::CommandPrefixOrSuffixItem::Word(ast::Word::from(w))
                }
            )+ { ast::CommandSuffix(s) }

        rule redirect_list() -> ast::RedirectList =
            r:io_redirect()+ { ast::RedirectList(r) } /
            expected!("redirect list")

        // N.B. here strings are extensions to the POSIX standard.
        rule io_redirect() -> ast::IoRedirect =
            n:io_number()? f:io_file() {
                let (kind, mut target, operator_location) = f;
                let (fd, start) = n.map_or((None, operator_location), |(fd, location)| {
                    (Some(fd), location)
                });
                if let Some(end) = target.location() {
                    target.set_location(SourceSpan::within(start, &end));
                }
                ast::IoRedirect::File(fd, kind, target)
            } /
            operator:specific_operator("&>>") target:filename() {
                let location = SourceSpan::within(operator.location(), target.location());
                ast::IoRedirect::OutputAndError(ast::Word::with_location(target.to_str(), &location), true)
            } /
            operator:specific_operator("&>") target:filename() {
                let location = SourceSpan::within(operator.location(), target.location());
                ast::IoRedirect::OutputAndError(ast::Word::with_location(target.to_str(), &location), false)
            } /
            n:io_number()? operator:specific_operator("<<<") w:word() {
                let (fd, start) = n.map_or((None, operator.location()), |(fd, location)| {
                    (Some(fd), location)
                });
                let location = SourceSpan::within(start, w.location());
                ast::IoRedirect::HereString(fd, ast::Word::with_location(w.to_str(), &location))
            } /
            n:io_number()? h:io_here() {
                let mut h = h;
                let fd = n.map(|(fd, location)| {
                    h.loc.start = location.start;
                    fd
                });
                ast::IoRedirect::HereDocument(fd, h)
            } /
            expected!("I/O redirect")

        // N.B. Process substitution forms are extensions to the POSIX standard.
        rule io_file() -> (ast::IoFileRedirectKind, ast::IoFileRedirectTarget, &'input SourceSpan) =
            op:specific_operator("<")  f:io_filename() { (ast::IoFileRedirectKind::Read, f, op.location()) } /
            op:specific_operator("<&") f:io_fd_duplication_source() { (ast::IoFileRedirectKind::DuplicateInput, f, op.location()) } /
            op:specific_operator(">")  f:io_filename() { (ast::IoFileRedirectKind::Write, f, op.location()) } /
            op:specific_operator(">&") f:io_fd_duplication_source() { (ast::IoFileRedirectKind::DuplicateOutput, f, op.location()) } /
            op:specific_operator(">>") f:io_filename() { (ast::IoFileRedirectKind::Append, f, op.location()) } /
            op:specific_operator("<>") f:io_filename() { (ast::IoFileRedirectKind::ReadAndWrite, f, op.location()) } /
            op:specific_operator(">|") f:io_filename() { (ast::IoFileRedirectKind::Clobber, f, op.location()) }

        rule io_fd_duplication_source() -> ast::IoFileRedirectTarget =
            w:word() { ast::IoFileRedirectTarget::Duplicate(ast::Word::from(w)) }

        rule io_filename() -> ast::IoFileRedirectTarget =
            sub:process_substitution() {
                let (kind, subshell) = sub;
                ast::IoFileRedirectTarget::ProcessSubstitution(kind, subshell)
            } /
            f:filename() { ast::IoFileRedirectTarget::Filename(ast::Word::from(f)) }

        rule filename() -> &'input Token =
            word()

        pub(crate) rule io_here() -> ast::IoHereDocument =
           operator:specific_operator("<<-") here_tag:here_tag() doc:[_] closing_tag:here_tag() {
                let requires_expansion = !here_tag.to_str().contains(['\'', '"', '\\']);
                ast::IoHereDocument {
                    remove_tabs: true,
                    requires_expansion,
                    here_end: ast::Word::from(here_tag),
                    doc: ast::Word::from(doc),
                    loc: SourceSpan::within(operator.location(), closing_tag.location()),
                }
            } /
            operator:specific_operator("<<") here_tag:here_tag() doc:[_] closing_tag:here_tag() {
                let requires_expansion = !here_tag.to_str().contains(['\'', '"', '\\']);
                ast::IoHereDocument {
                    remove_tabs: false,
                    requires_expansion,
                    here_end: ast::Word::from(here_tag),
                    doc: ast::Word::from(doc),
                    loc: SourceSpan::within(operator.location(), closing_tag.location()),
                }
            }

        rule here_tag() -> &'input Token =
            word()

        rule process_substitution() -> (ast::ProcessSubstitutionKind, ast::SubshellCommand) =
            specific_operator("<") s:subshell() { (ast::ProcessSubstitutionKind::Read, s) } /
            specific_operator(">") s:subshell() { (ast::ProcessSubstitutionKind::Write, s) }

        rule newline_list() -> () =
            newline()+ {}

        rule linebreak() -> () =
            quiet! {
                newline()* {}
            }

        rule separator_op() -> ast::SeparatorOperator =
            specific_operator("&") { ast::SeparatorOperator::Async } /
            specific_operator(";") { ast::SeparatorOperator::Sequence }

        rule separator() -> Option<ast::SeparatorOperator> =
            s:separator_op() linebreak() { Some(s) } /
            newline_list() { None }

        rule sequential_sep() -> () =
            specific_operator(";") linebreak() /
            newline_list()

        //
        // Token interpretation
        //

        rule non_reserved_word() -> &'input Token =
            !reserved_word() w:word() { w }

        rule word() -> &'input Token =
            [Token::Word(_, _)]

        rule reserved_word() -> &'input Token =
            [Token::Word(w, _) if matches!(w.as_str(),
                "!" |
                "{" |
                "}" |
                "case" |
                "do" |
                "done" |
                "elif" |
                "else" |
                "esac" |
                "fi" |
                "for" |
                "if" |
                "in" |
                "then" |
                "until" |
                "while"
            )] /

            // N.B. bash also treats the following as reserved.
            token:non_posix_reserved_word_token() { token }

        rule non_posix_reserved_word_token() -> &'input Token =
            specific_word("[[") /
            specific_word("]]") /
            specific_word("function") /
            specific_word("select") /
            specific_word("coproc")

        rule newline() -> () = quiet! {
            specific_operator("\n") {}
        }

        pub(crate) rule assignment_word() -> (ast::Assignment, ast::Word) =
            [Token::Word(w, l)] open:specific_operator("(") elements:array_elements() end:specific_operator(")") {?
                let element_values = elements.iter().map(|element| element.to_str()).collect::<Vec<_>>();
                let mut parsed = word::parse_array_assignment(w.as_str(), &element_values)?;

                let mut all_as_word = w.to_owned();
                all_as_word.push('(');
                let mut previous_end = open.location().end.index;
                for element in elements {
                    let location = element.location();
                    all_as_word.extend(std::iter::repeat_n(
                        ' ',
                        location.start.index.saturating_sub(previous_end),
                    ));
                    all_as_word.push_str(element.to_str());
                    previous_end = location.end.index;
                }
                all_as_word.extend(std::iter::repeat_n(
                    ' ',
                    end.location().start.index.saturating_sub(previous_end),
                ));
                all_as_word.push(')');

                let loc = SourceSpan::within(l, end.location());
                parsed.loc = loc.clone();
                Ok((parsed, ast::Word::with_location(&all_as_word, &loc)))
            } /
            [Token::Word(w, l)] {?
                let mut parsed = word::parse_assignment_word(w.as_str()).map_err(|_| "not assignment word")?;
                parsed.loc = l.clone();
                Ok((parsed, ast::Word::with_location(w, l)))
            }

        rule array_elements() -> Vec<&'input Token> =
             linebreak() e:array_element()* { e }

        rule array_element() -> &'input Token =
            linebreak() element:[Token::Word(_, _)] linebreak() { element }

        // N.B. An I/O number must be a string of only digits, and it must be
        // followed by a '<' or '>' character (but not consume them). We also
        // need to make sure that there was no space between the number and the
        // redirection operator; unfortunately we don't have the space anymore
        // but we can infer it by looking at the tokens' locations.
        rule io_number() -> (ast::IoFd, &'input SourceSpan) =
            [Token::Word(w, num_loc) if w.chars().all(|c: char| c.is_ascii_digit())]
            &([Token::Operator(o, redir_loc) if
                    o.starts_with(['<', '>']) &&
                    locations_are_contiguous(num_loc, redir_loc)]) {?

                let fd = w.parse::<ast::IoFd>().map_err(|_| "I/O file descriptor out of range")?;
                Ok((fd, num_loc))
            }

        rule overflowing_io_number() =
            [Token::Word(w, num_loc) if
                w.chars().all(|c: char| c.is_ascii_digit()) &&
                w.parse::<ast::IoFd>().is_err()]
            &([Token::Operator(o, redir_loc) if
                o.starts_with(['<', '>']) && locations_are_contiguous(num_loc, redir_loc)])

        //
        // Helpers
        //
        rule specific_operator(expected: &str) -> &'input Token =
            [Token::Operator(w, _) if w.as_str() == expected]

        rule specific_word(expected: &str) -> &'input Token =
            [Token::Word(w, _) if w.as_str() == expected]


    }
}

// add `2>&1` to the command if the pipeline is `|&`
fn add_pipe_extension_redirection(c: &mut ast::Command) {
    fn add_to_redirect_list(l: &mut Option<ast::RedirectList>, r: ast::IoRedirect) {
        if let Some(l) = l {
            l.0.push(r);
        } else {
            let v = vec![r];
            *l = Some(ast::RedirectList(v));
        }
    }

    let r = ast::IoRedirect::File(
        Some(2),
        ast::IoFileRedirectKind::DuplicateOutput,
        ast::IoFileRedirectTarget::Fd(1),
    );

    match c {
        ast::Command::Simple(c) => {
            let r = ast::CommandPrefixOrSuffixItem::IoRedirect(r);
            if let Some(l) = &mut c.suffix {
                l.0.push(r);
            } else {
                c.suffix = Some(ast::CommandSuffix(vec![r]));
            }
        }
        ast::Command::Compound(_, l) => add_to_redirect_list(l, r),
        ast::Command::Function(f) => add_to_redirect_list(&mut f.body.1, r),
        ast::Command::ExtendedTest(_, l) => add_to_redirect_list(l, r),
    }
}

#[inline]
fn locations_are_contiguous(
    loc_left: &crate::parser::SourceSpan,
    loc_right: &crate::parser::SourceSpan,
) -> bool {
    loc_left.end.index == loc_right.start.index
}

impl peg::Parse for Tokens<'_> {
    type PositionRepr = usize;

    #[inline]
    fn start(&self) -> usize {
        0
    }

    #[inline]
    fn is_eof(&self, p: usize) -> bool {
        p >= self.tokens.len()
    }

    #[inline]
    fn position_repr(&self, p: usize) -> Self::PositionRepr {
        p
    }
}

impl<'a> peg::ParseElem<'a> for Tokens<'a> {
    type Element = &'a Token;

    #[inline]
    fn parse_elem(&'a self, pos: usize) -> peg::RuleResult<Self::Element> {
        match self.tokens.get(pos) {
            Some(c) => peg::RuleResult::Matched(pos + 1, c),
            None => peg::RuleResult::Failed,
        }
    }
}

impl<'a> peg::ParseSlice<'a> for Tokens<'a> {
    type Slice = String;

    /// Reconstructs a source string from a slice of tokens.
    ///
    /// Uses each token's source position to detect whether whitespace existed
    /// between adjacent tokens in the original source, preserving its exact
    /// character count as spaces. This matters for constructs like `[[ x =~ (a| *) ]]`
    /// where the space inside the regex group is significant.
    ///
    /// N.B. This relies on tokens having accurate, contiguous source
    /// positions. All tokens produced by the normal tokenizer path satisfy
    /// this. If positions are missing or non-monotonic (as can happen with
    /// synthetic here-document end-tag tokens), the gap check evaluates
    /// false and no space is inserted — a safe degradation to the previous
    /// behavior, which also omitted spaces between non-word tokens.
    fn parse_slice(&'a self, start: usize, end: usize) -> Self::Slice {
        let mut result = String::new();
        let mut prev_end_index: Option<usize> = None;

        for token in &self.tokens[start..end] {
            let loc = token.location();

            if let Some(prev_end) = prev_end_index
                && loc.start.index > prev_end
            {
                result.extend(std::iter::repeat_n(' ', loc.start.index - prev_end));
            }

            result.push_str(token.to_str());
            prev_end_index = Some(loc.end.index);
        }

        result
    }
}
