//! Expression Parser for CODL Template Strings
//!
//! Parses template expressions like:
//! - `${gap}` - Variable reference
//! - `${instances[i-1]}` - Array index with arithmetic
//! - `${i > 0}` - Comparison for where clauses
//! - `100` - Literal value
//!
//! Uses Pratt Parser (Top-Down Operator Precedence) for correct operator
//! precedence and left-associativity of arithmetic operators.

use crate::ast::{BinaryOp, CodlExpr, ComparisonOp};
use crate::error::{CodlError, CodlResult};
use regex::Regex;
use std::sync::LazyLock;

/// Regex pattern for template variable extraction.
static TEMPLATE_VAR_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\$\{([^}]+)\}$").unwrap());

// =============================================================================
// Tokenizer
// =============================================================================

/// Token types for the expression parser.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// Integer literal
    Integer(String),
    /// Rational literal (e.g., "16/3")
    Rational(String),
    /// Identifier (variable name)
    Ident(String),
    /// Keyword "null"
    Null,
    /// Operators
    Plus,
    Minus,
    Star,
    Slash,
    /// Comparison operators
    Eq,      // ==
    Ne,      // !=
    Lt,      // <
    Le,      // <=
    Gt,      // >
    Ge,      // >=
    /// Brackets
    LBracket, // [
    RBracket, // ]
    LParen,   // (
    RParen,   // )
    /// End of input
    Eof,
}

/// Tokenizer for expression strings.
struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) {
        if let Some(c) = self.peek_char() {
            self.pos += c.len_utf8();
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_identifier(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn read_number(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        // Check for rational notation (e.g., "16/3" as a single token)
        if self.peek_char() == Some('/') {
            let slash_pos = self.pos;
            self.advance();
            let has_denominator = self.peek_char().is_some_and(|c| c.is_ascii_digit());
            if has_denominator {
                while let Some(c) = self.peek_char() {
                    if c.is_ascii_digit() {
                        self.advance();
                    } else {
                        break;
                    }
                }
                return self.input[start..self.pos].to_string();
            } else {
                // Not a rational, rollback
                self.pos = slash_pos;
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn next_token(&mut self) -> CodlResult<Token> {
        self.skip_whitespace();

        let Some(c) = self.peek_char() else {
            return Ok(Token::Eof);
        };

        // Two-character operators
        if self.input[self.pos..].starts_with("==") {
            self.pos += 2;
            return Ok(Token::Eq);
        }
        if self.input[self.pos..].starts_with("!=") {
            self.pos += 2;
            return Ok(Token::Ne);
        }
        if self.input[self.pos..].starts_with("<=") {
            self.pos += 2;
            return Ok(Token::Le);
        }
        if self.input[self.pos..].starts_with(">=") {
            self.pos += 2;
            return Ok(Token::Ge);
        }

        // Single-character tokens
        match c {
            '+' => {
                self.advance();
                Ok(Token::Plus)
            }
            '-' => {
                self.advance();
                Ok(Token::Minus)
            }
            '*' => {
                self.advance();
                Ok(Token::Star)
            }
            '/' => {
                self.advance();
                Ok(Token::Slash)
            }
            '<' => {
                self.advance();
                Ok(Token::Lt)
            }
            '>' => {
                self.advance();
                Ok(Token::Gt)
            }
            '[' => {
                self.advance();
                Ok(Token::LBracket)
            }
            ']' => {
                self.advance();
                Ok(Token::RBracket)
            }
            '(' => {
                self.advance();
                Ok(Token::LParen)
            }
            ')' => {
                self.advance();
                Ok(Token::RParen)
            }
            _ if c.is_ascii_digit() => {
                let num = self.read_number();
                if num.contains('/') {
                    Ok(Token::Rational(num))
                } else {
                    Ok(Token::Integer(num))
                }
            }
            _ if c.is_alphabetic() || c == '_' => {
                let ident = self.read_identifier();
                if ident == "null" {
                    Ok(Token::Null)
                } else {
                    Ok(Token::Ident(ident))
                }
            }
            _ => Err(CodlError::ParseError(format!(
                "Unexpected character: '{}'",
                c
            ))),
        }
    }
}

// =============================================================================
// Pratt Parser
// =============================================================================

/// Pratt parser for expressions with correct operator precedence.
struct PrattParser<'a> {
    tokenizer: Tokenizer<'a>,
    current: Token,
}

impl<'a> PrattParser<'a> {
    fn new(input: &'a str) -> CodlResult<Self> {
        let mut tokenizer = Tokenizer::new(input);
        let current = tokenizer.next_token()?;
        Ok(Self { tokenizer, current })
    }

    fn advance(&mut self) -> CodlResult<Token> {
        let prev = std::mem::replace(&mut self.current, Token::Eof);
        self.current = self.tokenizer.next_token()?;
        Ok(prev)
    }

    fn peek(&self) -> &Token {
        &self.current
    }

    /// Get binding power for binary operators.
    /// Returns (left_bp, right_bp).
    /// Left-associative: right_bp > left_bp
    /// Right-associative: left_bp > right_bp
    fn binding_power(op: &Token) -> Option<(u8, u8)> {
        match op {
            // Comparison operators: lowest precedence
            Token::Eq | Token::Ne | Token::Lt | Token::Le | Token::Gt | Token::Ge => Some((1, 2)),
            // Addition/Subtraction: medium precedence, left-associative
            Token::Plus | Token::Minus => Some((3, 4)),
            // Multiplication/Division: highest precedence, left-associative
            Token::Star | Token::Slash => Some((5, 6)),
            _ => None,
        }
    }

    /// Parse expression with Pratt algorithm (precedence climbing).
    fn parse_expr(&mut self, min_bp: u8) -> CodlResult<CodlExpr> {
        // Parse prefix (atom)
        let mut lhs = self.parse_atom()?;

        loop {
            // Check for postfix: array indexing
            if *self.peek() == Token::LBracket {
                // Only if lhs is an identifier
                if let CodlExpr::Variable(array_name) = lhs {
                    self.advance()?; // consume '['
                    let index = self.parse_expr(0)?;
                    if *self.peek() != Token::RBracket {
                        return Err(CodlError::ParseError(
                            "Expected ']' after array index".to_string(),
                        ));
                    }
                    self.advance()?; // consume ']'
                    lhs = CodlExpr::ArrayIndex {
                        array: array_name,
                        index: Box::new(index),
                    };
                    continue;
                }
            }

            // Check for infix operator
            let Some((l_bp, r_bp)) = Self::binding_power(self.peek()) else {
                break;
            };

            if l_bp < min_bp {
                break;
            }

            let op_token = self.advance()?;
            let rhs = self.parse_expr(r_bp)?;

            lhs = self.make_binary_expr(lhs, &op_token, rhs)?;
        }

        Ok(lhs)
    }

    /// Parse an atomic expression (literals, variables, parenthesized expressions).
    fn parse_atom(&mut self) -> CodlResult<CodlExpr> {
        match self.peek().clone() {
            Token::Integer(n) => {
                self.advance()?;
                Ok(CodlExpr::Literal(n))
            }
            Token::Rational(r) => {
                self.advance()?;
                Ok(CodlExpr::Literal(r))
            }
            Token::Ident(name) => {
                self.advance()?;
                // Check for null comparison special case: "ident != null" or "ident == null"
                // This is handled at a higher level, so just return variable
                Ok(CodlExpr::Variable(name))
            }
            Token::Null => {
                self.advance()?;
                // null by itself is treated as a literal
                Ok(CodlExpr::Literal("null".to_string()))
            }
            Token::LParen => {
                self.advance()?; // consume '('
                let expr = self.parse_expr(0)?;
                if *self.peek() != Token::RParen {
                    return Err(CodlError::ParseError(
                        "Expected ')' after expression".to_string(),
                    ));
                }
                self.advance()?; // consume ')'
                Ok(expr)
            }
            Token::Minus => {
                // Unary minus
                self.advance()?;
                let operand = self.parse_atom()?;
                Ok(CodlExpr::BinaryOp {
                    left: Box::new(CodlExpr::Literal("0".to_string())),
                    op: BinaryOp::Sub,
                    right: Box::new(operand),
                })
            }
            other => Err(CodlError::ParseError(format!(
                "Unexpected token in expression: {:?}",
                other
            ))),
        }
    }

    fn make_binary_expr(
        &self,
        lhs: CodlExpr,
        op: &Token,
        rhs: CodlExpr,
    ) -> CodlResult<CodlExpr> {
        match op {
            Token::Plus => Ok(CodlExpr::BinaryOp {
                left: Box::new(lhs),
                op: BinaryOp::Add,
                right: Box::new(rhs),
            }),
            Token::Minus => Ok(CodlExpr::BinaryOp {
                left: Box::new(lhs),
                op: BinaryOp::Sub,
                right: Box::new(rhs),
            }),
            Token::Star => Ok(CodlExpr::BinaryOp {
                left: Box::new(lhs),
                op: BinaryOp::Mul,
                right: Box::new(rhs),
            }),
            Token::Slash => Ok(CodlExpr::BinaryOp {
                left: Box::new(lhs),
                op: BinaryOp::Div,
                right: Box::new(rhs),
            }),
            Token::Eq => Ok(CodlExpr::Comparison {
                left: Box::new(lhs),
                op: ComparisonOp::Eq,
                right: Box::new(rhs),
            }),
            Token::Ne => Ok(CodlExpr::Comparison {
                left: Box::new(lhs),
                op: ComparisonOp::Ne,
                right: Box::new(rhs),
            }),
            Token::Lt => Ok(CodlExpr::Comparison {
                left: Box::new(lhs),
                op: ComparisonOp::Lt,
                right: Box::new(rhs),
            }),
            Token::Le => Ok(CodlExpr::Comparison {
                left: Box::new(lhs),
                op: ComparisonOp::Le,
                right: Box::new(rhs),
            }),
            Token::Gt => Ok(CodlExpr::Comparison {
                left: Box::new(lhs),
                op: ComparisonOp::Gt,
                right: Box::new(rhs),
            }),
            Token::Ge => Ok(CodlExpr::Comparison {
                left: Box::new(lhs),
                op: ComparisonOp::Ge,
                right: Box::new(rhs),
            }),
            _ => Err(CodlError::ParseError(format!(
                "Invalid binary operator: {:?}",
                op
            ))),
        }
    }
}

// =============================================================================
// Public API
// =============================================================================

/// Parse a template expression string into a CodlExpr AST.
///
/// ## Supported Formats
///
/// - `${variable}` - Variable reference
/// - `${array[index]}` - Array indexing
/// - `${a + b}` - Binary arithmetic
/// - `${a > b}` - Comparison
/// - `literal` - Literal value (no `${}`)
///
/// ## Operator Precedence (highest to lowest)
///
/// 1. `*`, `/` - Multiplication, Division (left-associative)
/// 2. `+`, `-` - Addition, Subtraction (left-associative)
/// 3. `==`, `!=`, `<`, `<=`, `>`, `>=` - Comparisons
pub fn parse_expr(input: &str) -> CodlResult<CodlExpr> {
    let trimmed = input.trim();

    // Check if it's a template variable ${...}
    if let Some(caps) = TEMPLATE_VAR_PATTERN.captures(trimmed) {
        let inner = caps.get(1).unwrap().as_str().trim();
        return parse_inner_expr(inner);
    }

    // Otherwise, treat as literal
    Ok(CodlExpr::Literal(trimmed.to_string()))
}

/// Parse the inner content of a ${...} expression using Pratt parser.
fn parse_inner_expr(input: &str) -> CodlResult<CodlExpr> {
    let trimmed = input.trim();

    // Handle null checks as special cases before tokenizing
    // Pattern: "ident != null" or "ident == null"
    if let Some(result) = try_parse_null_check(trimmed) {
        return Ok(result);
    }

    // Use Pratt parser for everything else
    let mut parser = PrattParser::new(trimmed)?;
    let expr = parser.parse_expr(0)?;

    // Ensure we consumed all input
    if *parser.peek() != Token::Eof {
        return Err(CodlError::ParseError(format!(
            "Unexpected token after expression: {:?}",
            parser.peek()
        )));
    }

    Ok(expr)
}

/// Try to parse a null check expression.
fn try_parse_null_check(input: &str) -> Option<CodlExpr> {
    static NULL_CHECK_PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\w+)\s*(!=|==)\s*null$").unwrap());

    if let Some(caps) = NULL_CHECK_PATTERN.captures(input) {
        let var_name = caps.get(1).unwrap().as_str().to_string();
        let op = caps.get(2).unwrap().as_str();

        return Some(if op == "!=" {
            CodlExpr::IsNotNull(var_name)
        } else {
            CodlExpr::IsNull(var_name)
        });
    }
    None
}


/// Parse a where clause condition.
///
/// Where clauses are simple comparison expressions like:
/// - `i > 0`
/// - `i >= 1`
/// - `origin_x != null`
pub fn parse_where_clause(input: &str) -> CodlResult<CodlExpr> {
    parse_inner_expr(input.trim())
}

/// Extract all variable references from an expression.
pub fn extract_variables(expr: &CodlExpr) -> Vec<String> {
    let mut vars = Vec::new();
    collect_variables(expr, &mut vars);
    vars
}

fn collect_variables(expr: &CodlExpr, vars: &mut Vec<String>) {
    match expr {
        CodlExpr::Literal(_) => {}
        CodlExpr::Variable(name) => {
            if !vars.contains(name) {
                vars.push(name.clone());
            }
        }
        CodlExpr::ArrayIndex { array, index } => {
            if !vars.contains(array) {
                vars.push(array.clone());
            }
            collect_variables(index, vars);
        }
        CodlExpr::BinaryOp { left, right, .. } => {
            collect_variables(left, vars);
            collect_variables(right, vars);
        }
        CodlExpr::Comparison { left, right, .. } => {
            collect_variables(left, vars);
            collect_variables(right, vars);
        }
        CodlExpr::IsNull(name) | CodlExpr::IsNotNull(name) => {
            if !vars.contains(name) {
                vars.push(name.clone());
            }
        }
    }
}

/// Extract array accesses with their index expressions.
pub fn extract_array_accesses(expr: &CodlExpr) -> Vec<(String, CodlExpr)> {
    let mut accesses = Vec::new();
    collect_array_accesses(expr, &mut accesses);
    accesses
}

fn collect_array_accesses(expr: &CodlExpr, accesses: &mut Vec<(String, CodlExpr)>) {
    match expr {
        CodlExpr::ArrayIndex { array, index } => {
            accesses.push((array.clone(), *index.clone()));
            collect_array_accesses(index, accesses);
        }
        CodlExpr::BinaryOp { left, right, .. } => {
            collect_array_accesses(left, accesses);
            collect_array_accesses(right, accesses);
        }
        CodlExpr::Comparison { left, right, .. } => {
            collect_array_accesses(left, accesses);
            collect_array_accesses(right, accesses);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_literal() {
        let expr = parse_expr("100").unwrap();
        assert_eq!(expr, CodlExpr::Literal("100".to_string()));
    }

    #[test]
    fn test_parse_template_variable() {
        let expr = parse_expr("${gap}").unwrap();
        assert_eq!(expr, CodlExpr::Variable("gap".to_string()));
    }

    #[test]
    fn test_parse_array_index() {
        let expr = parse_expr("${instances[i]}").unwrap();
        match expr {
            CodlExpr::ArrayIndex { array, index } => {
                assert_eq!(array, "instances");
                assert_eq!(*index, CodlExpr::Variable("i".to_string()));
            }
            _ => panic!("Expected ArrayIndex"),
        }
    }

    #[test]
    fn test_parse_array_index_with_arithmetic() {
        let expr = parse_expr("${instances[i-1]}").unwrap();
        match expr {
            CodlExpr::ArrayIndex { array, index } => {
                assert_eq!(array, "instances");
                match *index {
                    CodlExpr::BinaryOp { op, .. } => {
                        assert_eq!(op, BinaryOp::Sub);
                    }
                    _ => panic!("Expected BinaryOp in index"),
                }
            }
            _ => panic!("Expected ArrayIndex"),
        }
    }

    #[test]
    fn test_parse_comparison() {
        let expr = parse_where_clause("i > 0").unwrap();
        match expr {
            CodlExpr::Comparison { left, op, right } => {
                assert_eq!(*left, CodlExpr::Variable("i".to_string()));
                assert_eq!(op, ComparisonOp::Gt);
                assert_eq!(*right, CodlExpr::Literal("0".to_string()));
            }
            _ => panic!("Expected Comparison"),
        }
    }

    #[test]
    fn test_parse_null_check() {
        let expr = parse_where_clause("origin_x != null").unwrap();
        assert_eq!(expr, CodlExpr::IsNotNull("origin_x".to_string()));
    }

    #[test]
    fn test_extract_variables() {
        let expr = parse_expr("${instances[i-1]}").unwrap();
        let vars = extract_variables(&expr);
        assert!(vars.contains(&"instances".to_string()));
        assert!(vars.contains(&"i".to_string()));
    }

    // =========================================================================
    // Operator Precedence Tests (Pratt Parser)
    // =========================================================================

    /// Helper to verify AST structure for binary operations.
    fn assert_binop(
        expr: &CodlExpr,
        expected_op: BinaryOp,
        check_left: impl FnOnce(&CodlExpr),
        check_right: impl FnOnce(&CodlExpr),
    ) {
        match expr {
            CodlExpr::BinaryOp { left, op, right } => {
                assert_eq!(*op, expected_op, "Operator mismatch");
                check_left(left);
                check_right(right);
            }
            _ => panic!("Expected BinaryOp, got {:?}", expr),
        }
    }

    fn assert_literal(expr: &CodlExpr, expected: &str) {
        match expr {
            CodlExpr::Literal(s) => assert_eq!(s, expected),
            _ => panic!("Expected Literal({}), got {:?}", expected, expr),
        }
    }

    fn assert_variable(expr: &CodlExpr, expected: &str) {
        match expr {
            CodlExpr::Variable(s) => assert_eq!(s, expected),
            _ => panic!("Expected Variable({}), got {:?}", expected, expr),
        }
    }

    #[test]
    fn test_precedence_mul_over_add() {
        // 2 + 3 * 4 should parse as 2 + (3 * 4), not (2 + 3) * 4
        let expr = parse_expr("${2 + 3 * 4}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Add,
            |l| assert_literal(l, "2"),
            |r| {
                assert_binop(
                    r,
                    BinaryOp::Mul,
                    |l| assert_literal(l, "3"),
                    |r| assert_literal(r, "4"),
                )
            },
        );
    }

    #[test]
    fn test_precedence_mul_over_sub() {
        // 10 - 2 * 3 should parse as 10 - (2 * 3)
        let expr = parse_expr("${10 - 2 * 3}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Sub,
            |l| assert_literal(l, "10"),
            |r| {
                assert_binop(
                    r,
                    BinaryOp::Mul,
                    |l| assert_literal(l, "2"),
                    |r| assert_literal(r, "3"),
                )
            },
        );
    }

    #[test]
    fn test_precedence_div_over_add() {
        // 10 + 6 / 2 should parse as 10 + (6 / 2)
        let expr = parse_expr("${10 + 6 / 2}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Add,
            |l| assert_literal(l, "10"),
            |r| {
                assert_binop(
                    r,
                    BinaryOp::Div,
                    |l| assert_literal(l, "6"),
                    |r| assert_literal(r, "2"),
                )
            },
        );
    }

    #[test]
    fn test_left_associativity_add() {
        // 1 + 2 + 3 should parse as (1 + 2) + 3, not 1 + (2 + 3)
        let expr = parse_expr("${1 + 2 + 3}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Add,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Add,
                    |ll| assert_literal(ll, "1"),
                    |lr| assert_literal(lr, "2"),
                )
            },
            |r| assert_literal(r, "3"),
        );
    }

    #[test]
    fn test_left_associativity_sub() {
        // 10 - 4 - 2 should parse as (10 - 4) - 2, evaluating to 4
        let expr = parse_expr("${10 - 4 - 2}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Sub,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Sub,
                    |ll| assert_literal(ll, "10"),
                    |lr| assert_literal(lr, "4"),
                )
            },
            |r| assert_literal(r, "2"),
        );
    }

    #[test]
    fn test_left_associativity_div() {
        // 24 / 4 / 2 should parse as (24 / 4) / 2, evaluating to 3
        let expr = parse_expr("${24 / 4 / 2}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Div,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Div,
                    |ll| assert_literal(ll, "24"),
                    |lr| assert_literal(lr, "4"),
                )
            },
            |r| assert_literal(r, "2"),
        );
    }

    #[test]
    fn test_left_associativity_mul() {
        // 2 * 3 * 4 should parse as (2 * 3) * 4
        let expr = parse_expr("${2 * 3 * 4}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Mul,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Mul,
                    |ll| assert_literal(ll, "2"),
                    |lr| assert_literal(lr, "3"),
                )
            },
            |r| assert_literal(r, "4"),
        );
    }

    #[test]
    fn test_parentheses_override_precedence() {
        // (2 + 3) * 4 should parse with addition first
        let expr = parse_expr("${(2 + 3) * 4}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Mul,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Add,
                    |ll| assert_literal(ll, "2"),
                    |lr| assert_literal(lr, "3"),
                )
            },
            |r| assert_literal(r, "4"),
        );
    }

    #[test]
    fn test_complex_expression_precedence() {
        // a * 2 + 16/3 should parse as (a * 2) + (16/3)
        // This was the original failing case!
        let expr = parse_expr("${a * 2 + 16/3}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Add,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Mul,
                    |ll| assert_variable(ll, "a"),
                    |lr| assert_literal(lr, "2"),
                )
            },
            |r| {
                // 16/3 is parsed as a rational literal, not division
                assert_literal(r, "16/3")
            },
        );
    }

    #[test]
    fn test_complex_expression_with_division_operator() {
        // a * 2 + b / 3 should parse as (a * 2) + (b / 3)
        let expr = parse_expr("${a * 2 + b / 3}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Add,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Mul,
                    |ll| assert_variable(ll, "a"),
                    |lr| assert_literal(lr, "2"),
                )
            },
            |r| {
                assert_binop(
                    r,
                    BinaryOp::Div,
                    |rl| assert_variable(rl, "b"),
                    |rr| assert_literal(rr, "3"),
                )
            },
        );
    }

    #[test]
    fn test_mixed_mul_div_left_associative() {
        // 12 * 3 / 4 should parse as (12 * 3) / 4
        let expr = parse_expr("${12 * 3 / 4}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Div,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Mul,
                    |ll| assert_literal(ll, "12"),
                    |lr| assert_literal(lr, "3"),
                )
            },
            |r| assert_literal(r, "4"),
        );
    }

    #[test]
    fn test_rational_literal_in_expression() {
        // 16/3 as a standalone literal
        let expr = parse_expr("${16/3}").unwrap();
        assert_literal(&expr, "16/3");
    }

    #[test]
    fn test_unary_minus() {
        let expr = parse_expr("${-5}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Sub,
            |l| assert_literal(l, "0"),
            |r| assert_literal(r, "5"),
        );
    }

    #[test]
    fn test_nested_parentheses() {
        // ((1 + 2) * 3) + 4
        let expr = parse_expr("${((1 + 2) * 3) + 4}").unwrap();
        assert_binop(
            &expr,
            BinaryOp::Add,
            |l| {
                assert_binop(
                    l,
                    BinaryOp::Mul,
                    |ll| {
                        assert_binop(
                            ll,
                            BinaryOp::Add,
                            |lll| assert_literal(lll, "1"),
                            |llr| assert_literal(llr, "2"),
                        )
                    },
                    |lr| assert_literal(lr, "3"),
                )
            },
            |r| assert_literal(r, "4"),
        );
    }

    #[test]
    fn test_comparison_lowest_precedence() {
        // a + b > c * d should parse as (a + b) > (c * d)
        let expr = parse_where_clause("a + b > c * d").unwrap();
        match expr {
            CodlExpr::Comparison { left, op, right } => {
                assert_eq!(op, ComparisonOp::Gt);
                assert_binop(
                    &left,
                    BinaryOp::Add,
                    |l| assert_variable(l, "a"),
                    |r| assert_variable(r, "b"),
                );
                assert_binop(
                    &right,
                    BinaryOp::Mul,
                    |l| assert_variable(l, "c"),
                    |r| assert_variable(r, "d"),
                );
            }
            _ => panic!("Expected Comparison"),
        }
    }
}
