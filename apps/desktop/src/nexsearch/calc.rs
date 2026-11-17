//! Calculator provider: a small recursive-descent expression parser.
//!
//! Grammar (lowest to highest precedence):
//! `expr := term (('+'|'-') term)*`, `term := unary (('*'|'/'|'%') unary)*`,
//! `unary := ('-'|'+') unary | power`, `power := atom ('^' unary)?` (right associative),
//! `atom := number | '(' expr ')'`.
//! Both `.` and `,` are accepted as decimal separator; whitespace is ignored.

use super::{SearchAction, SearchProvider, SearchResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalcError {
    Empty,
    UnexpectedChar(char),
    UnexpectedEnd,
    UnexpectedToken,
    UnbalancedParens,
    InvalidNumber,
    DivisionByZero,
    NotFinite,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Num(f64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
}

fn tokenize(input: &str) -> Result<Vec<Token>, CalcError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' | '\u{2212}' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' | '\u{00d7}' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' | '\u{00f7}' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '0'..='9' | '.' | ',' => {
                let start = i;
                let mut seps = 0;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | ','))
                {
                    if matches!(chars[i], '.' | ',') {
                        seps += 1;
                    }
                    i += 1;
                }
                if seps > 1 {
                    return Err(CalcError::InvalidNumber);
                }
                let text: String = chars[start..i]
                    .iter()
                    .map(|&c| if c == ',' { '.' } else { c })
                    .collect();
                if !text.chars().any(|c| c.is_ascii_digit()) {
                    return Err(CalcError::InvalidNumber);
                }
                let n: f64 = text.parse().map_err(|_| CalcError::InvalidNumber)?;
                tokens.push(Token::Num(n));
            }
            other => return Err(CalcError::UnexpectedChar(other)),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.peek();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expr(&mut self) -> Result<f64, CalcError> {
        let mut v = self.term()?;
        while let Some(t @ (Token::Plus | Token::Minus)) = self.peek() {
            self.pos += 1;
            let r = self.term()?;
            v = if t == Token::Plus { v + r } else { v - r };
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, CalcError> {
        let mut v = self.unary()?;
        while let Some(t @ (Token::Star | Token::Slash | Token::Percent)) = self.peek() {
            self.pos += 1;
            let r = self.unary()?;
            v = match t {
                Token::Star => v * r,
                Token::Slash => {
                    if r == 0.0 {
                        return Err(CalcError::DivisionByZero);
                    }
                    v / r
                }
                _ => {
                    if r == 0.0 {
                        return Err(CalcError::DivisionByZero);
                    }
                    v % r
                }
            };
        }
        Ok(v)
    }

    fn unary(&mut self) -> Result<f64, CalcError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(-self.unary()?)
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<f64, CalcError> {
        let base = self.atom()?;
        if self.peek() == Some(Token::Caret) {
            self.pos += 1;
            // Right associative; exponent may be negative (2^-1).
            let exp = self.unary()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<f64, CalcError> {
        match self.next() {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::LParen) => {
                let v = self.expr()?;
                match self.next() {
                    Some(Token::RParen) => Ok(v),
                    _ => Err(CalcError::UnbalancedParens),
                }
            }
            Some(Token::RParen) => Err(CalcError::UnbalancedParens),
            Some(_) => Err(CalcError::UnexpectedToken),
            None => Err(CalcError::UnexpectedEnd),
        }
    }
}

/// Evaluates an arithmetic expression.
pub fn evaluate(input: &str) -> Result<f64, CalcError> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err(CalcError::Empty);
    }
    let mut p = Parser { tokens, pos: 0 };
    let v = p.expr()?;
    match p.peek() {
        None => {}
        Some(Token::RParen) => return Err(CalcError::UnbalancedParens),
        Some(_) => return Err(CalcError::UnexpectedToken),
    }
    if v.is_finite() {
        Ok(v)
    } else {
        Err(CalcError::NotFinite)
    }
}

/// Formats a result: integers without fraction, otherwise up to 10 decimals trimmed.
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if v.abs() >= 1e15 || v.abs() < 1e-10 {
        return format!("{v:e}");
    }
    let s = format!("{v:.10}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// True if the input looks like math (contains a digit and an operator or parenthesis),
/// so that plain numbers and words do not produce a calculator result.
fn looks_like_expression(input: &str) -> bool {
    input.chars().any(|c| c.is_ascii_digit())
        && input.chars().any(|c| {
            matches!(
                c,
                '+' | '-'
                    | '*'
                    | '/'
                    | '%'
                    | '^'
                    | '('
                    | ')'
                    | '\u{2212}'
                    | '\u{00d7}'
                    | '\u{00f7}'
            )
        })
}

#[derive(Debug, Default)]
pub struct CalcProvider;

impl SearchProvider for CalcProvider {
    fn query(&self, input: &str) -> Vec<SearchResult> {
        let input = input.trim();
        // A trailing "=" is allowed ("2+2=").
        let expr = input.strip_suffix('=').unwrap_or(input).trim();
        if !looks_like_expression(expr) {
            return Vec::new();
        }
        match evaluate(expr) {
            Ok(v) => {
                let text = format_number(v);
                vec![SearchResult {
                    id: format!("calc:{expr}"),
                    title: text.clone(),
                    subtitle: format!("{expr} ="),
                    score: 100.0,
                    action: SearchAction::CopyText(text),
                }]
            }
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        evaluate(s).unwrap_or_else(|e| panic!("{s}: {e:?}"))
    }

    #[test]
    fn basic_ops() {
        assert_eq!(ev("1+2"), 3.0);
        assert_eq!(ev("5-8"), -3.0);
        assert_eq!(ev("6*7"), 42.0);
        assert_eq!(ev("9/4"), 2.25);
        assert_eq!(ev("10%3"), 1.0);
        assert_eq!(ev("2^10"), 1024.0);
    }

    #[test]
    fn precedence() {
        assert_eq!(ev("2+3*4"), 14.0);
        assert_eq!(ev("2*3+4"), 10.0);
        assert_eq!(ev("10-4-3"), 3.0);
        assert_eq!(ev("100/10/5"), 2.0);
        assert_eq!(ev("2*3^2"), 18.0);
    }

    #[test]
    fn power_right_associative() {
        assert_eq!(ev("2^3^2"), 512.0);
        assert_eq!(ev("2^-1"), 0.5);
    }

    #[test]
    fn unary_minus() {
        assert_eq!(ev("-5"), -5.0);
        assert_eq!(ev("--5"), 5.0);
        assert_eq!(ev("3*-2"), -6.0);
        assert_eq!(ev("-(2+3)"), -5.0);
        assert_eq!(ev("+4"), 4.0);
        assert_eq!(ev("2--2"), 4.0);
        // Unary minus binds looser than ^, as in math: -2^2 = -4.
        assert_eq!(ev("-2^2"), -4.0);
    }

    #[test]
    fn parentheses() {
        assert_eq!(ev("(2+3)*4"), 20.0);
        assert_eq!(ev("((1+2))*(3+4)"), 21.0);
        assert_eq!(ev("2*(3+(4-1))"), 12.0);
    }

    #[test]
    fn decimals_point_and_comma() {
        assert_eq!(ev("1.5+1.5"), 3.0);
        assert_eq!(ev("1,5+1,5"), 3.0);
        assert_eq!(ev(".5*2"), 1.0);
        assert_eq!(ev("2,5*2"), 5.0);
    }

    #[test]
    fn whitespace_and_unicode_operators() {
        assert_eq!(ev("  1 +   2 "), 3.0);
        assert_eq!(ev("6\u{00d7}7"), 42.0);
        assert_eq!(ev("8\u{00f7}2"), 4.0);
        assert_eq!(ev("5\u{2212}2"), 3.0);
    }

    #[test]
    fn division_by_zero() {
        assert_eq!(evaluate("1/0"), Err(CalcError::DivisionByZero));
        assert_eq!(evaluate("1/(2-2)"), Err(CalcError::DivisionByZero));
        assert_eq!(evaluate("5%0"), Err(CalcError::DivisionByZero));
    }

    #[test]
    fn syntax_errors() {
        assert_eq!(evaluate(""), Err(CalcError::Empty));
        assert_eq!(evaluate("   "), Err(CalcError::Empty));
        assert_eq!(evaluate("1+"), Err(CalcError::UnexpectedEnd));
        assert_eq!(evaluate("*2"), Err(CalcError::UnexpectedToken));
        assert_eq!(evaluate("2 3"), Err(CalcError::UnexpectedToken));
        assert_eq!(evaluate("(1+2"), Err(CalcError::UnbalancedParens));
        assert_eq!(evaluate("1+2)"), Err(CalcError::UnbalancedParens));
        assert_eq!(evaluate(")"), Err(CalcError::UnbalancedParens));
        assert_eq!(evaluate("()"), Err(CalcError::UnbalancedParens));
        assert_eq!(evaluate("1+a"), Err(CalcError::UnexpectedChar('a')));
        assert_eq!(evaluate("1.2.3"), Err(CalcError::InvalidNumber));
        assert_eq!(evaluate("."), Err(CalcError::InvalidNumber));
    }

    #[test]
    fn overflow_is_not_finite() {
        assert_eq!(evaluate("10^1000"), Err(CalcError::NotFinite));
    }

    #[test]
    fn format_numbers() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(-3.0), "-3");
        assert_eq!(format_number(2.25), "2.25");
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(1e20), "1e20");
    }

    #[test]
    fn provider_returns_copy_text() {
        let r = CalcProvider.query("2+2");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "4");
        assert_eq!(r[0].action, SearchAction::CopyText("4".into()));
        assert!(r[0].score >= 100.0);
    }

    #[test]
    fn provider_accepts_trailing_equals_and_comma() {
        let r = CalcProvider.query("1,5*2=");
        assert_eq!(r[0].title, "3");
    }

    #[test]
    fn provider_ignores_non_math() {
        assert!(CalcProvider.query("firefox").is_empty());
        assert!(CalcProvider.query("42").is_empty());
        assert!(CalcProvider.query("-").is_empty());
        assert!(CalcProvider.query("").is_empty());
        assert!(CalcProvider.query("chrome 2").is_empty());
    }

    #[test]
    fn provider_no_result_on_errors() {
        assert!(CalcProvider.query("1/0").is_empty());
        assert!(CalcProvider.query("(1+2").is_empty());
        assert!(CalcProvider.query("2+").is_empty());
    }
}
