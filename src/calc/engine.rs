//! Turns typed or keypad input into a number. Pure logic with no Windows dependency, so `cargo test`
//! covers it directly.
//!
//! Input and output both use '.' for decimals and ',' between function arguments, so a result can
//! be edited and evaluated again as-is.

use std::f64::consts::{E, PI};
use std::fmt;

/// Display precision for real numbers; hides binary noise such as 0.1 + 0.2 = 0.30000000000000004.
const SIGNIFICANT_DIGITS: usize = 15;

/// A computed value. Whole numbers stay exact until they overflow, then switch to floating point
/// instead of wrapping around, like a real calculator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    Integer(i64),
    Real(f64),
}

impl Number {
    fn real(self) -> f64 {
        match self {
            Number::Integer(i) => i as f64,
            Number::Real(r) => r,
        }
    }

    fn is_zero(self) -> bool {
        self.real() == 0.0
    }
}

/// One successful evaluation: the value (available as `ans` next time) and its display text.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub value: Number,
    pub text: String,
}

/// Why an expression has no result. `Display` gives the message shown under the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalcError {
    Empty,
    Invalid,
    DivideByZero,
    TooLarge,
    Undefined,
    UnknownFunction,
    UnknownName,
    CantEvaluate,
}

impl fmt::Display for CalcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CalcError::Empty => "Enter an expression",
            CalcError::Invalid => "Invalid expression",
            CalcError::DivideByZero => "Can't divide by zero",
            CalcError::TooLarge => "Result is too large",
            CalcError::Undefined => "Result is undefined",
            CalcError::UnknownFunction => "Unknown function",
            CalcError::UnknownName => "Unknown name in expression",
            CalcError::CantEvaluate => "Can't evaluate that",
        })
    }
}

impl std::error::Error for CalcError {}

/// Evaluates `input`; `previous_answer` is available as `ans`.
pub fn evaluate(input: &str, previous_answer: Option<Number>) -> Result<Answer, CalcError> {
    let expression = normalize(input);
    if expression.is_empty() {
        return Err(CalcError::Empty);
    }

    // Parse the whole expression before evaluating any of it, so a typo reads as a typo even when
    // an earlier part would fail to evaluate.
    let tree = Parser::new(tokenize(&expression)?).parse()?;
    let value = tree.evaluate(previous_answer.unwrap_or(Number::Integer(0)))?;
    Ok(Answer { value, text: format(value)? })
}

/// Formats a value as text that can be evaluated again.
pub fn format(value: Number) -> Result<String, CalcError> {
    match value {
        Number::Integer(i) => Ok(i.to_string()),
        Number::Real(r) if r.is_nan() => Err(CalcError::Undefined),
        Number::Real(r) if r.is_infinite() => Err(CalcError::TooLarge),
        Number::Real(r) => Ok(format_real(r)),
    }
}

/// Normalizes keypad symbols and calculator habits: ×, ÷ and − become ASCII, π becomes `pi` (with
/// the multiplication implied by `2π`), and a trailing `=` is dropped.
fn normalize(input: &str) -> String {
    let mut text = String::with_capacity(input.len());
    let mut previous = None;
    for c in input.chars() {
        match c {
            '×' => text.push('*'),
            '÷' => text.push('/'),
            '−' => text.push('-'),
            'π' => {
                if matches!(previous, Some(p) if p == ')' || char::is_ascii_digit(&p)) {
                    text.push('*');
                }
                text.push_str("pi");
            }
            _ => text.push(c),
        }
        previous = Some(c);
    }

    text.trim().trim_end_matches('=').trim_end().to_owned()
}

/// Formats like .NET's "G15": up to 15 significant digits, trailing zeros dropped, and scientific
/// notation (`9.9999999998E+21`) for very large or very small magnitudes.
fn format_real(value: f64) -> String {
    // Covers -0, which reads as 0.
    if value == 0.0 {
        return "0".to_owned();
    }

    // Let the standard library do the correctly rounded part, then lay the digits out.
    let scientific = format!("{:.*e}", SIGNIFICANT_DIGITS - 1, value);
    let (mantissa, exponent) = scientific.split_once('e').expect("{:e} always has an exponent");
    let exponent: i32 = exponent.parse().expect("{:e} exponents are integers");
    let sign = if value < 0.0 { "-" } else { "" };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');

    if !(-5 < exponent && exponent < SIGNIFICANT_DIGITS as i32) {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!("{sign}{first}{point}{rest}E{exponent_sign}{:02}", exponent.abs())
    } else if exponent < 0 {
        let zeros = "0".repeat((-exponent - 1) as usize);
        format!("{sign}0.{zeros}{digits}")
    } else {
        let whole = exponent as usize + 1;
        if digits.len() <= whole {
            format!("{sign}{digits}{}", "0".repeat(whole - digits.len()))
        } else {
            format!("{sign}{}.{}", &digits[..whole], &digits[whole..])
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(Number),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Power,
    Open,
    Close,
    Comma,
}

fn tokenize(text: &str) -> Result<Vec<Token>, CalcError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c.is_ascii_digit() || (c == '.' && next.is_some_and(|n| n.is_ascii_digit())) {
            let (number, end) = scan_number(&chars, i)?;
            tokens.push(Token::Number(number));
            i = end;
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
            continue;
        }

        let token = match c {
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' if next == Some('*') => {
                i += 1;
                Token::Power
            }
            '*' => Token::Star,
            '/' => Token::Slash,
            '%' => Token::Percent,
            '^' => Token::Power,
            '(' => Token::Open,
            ')' => Token::Close,
            ',' => Token::Comma,
            _ => return Err(CalcError::Invalid),
        };
        tokens.push(token);
        i += 1;
    }

    Ok(tokens)
}

/// Reads a number starting at `start`: digits, an optional fraction and an optional exponent.
/// Plain digits are an integer unless they don't fit in one.
fn scan_number(chars: &[char], start: usize) -> Result<(Number, usize), CalcError> {
    let digits_from = |mut i: usize| {
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        i
    };

    let mut end = digits_from(start);
    let mut is_real = false;
    if chars.get(end) == Some(&'.') {
        is_real = true;
        end = digits_from(end + 1);
    }

    if matches!(chars.get(end), Some('e' | 'E')) {
        let mut exponent = end + 1;
        if matches!(chars.get(exponent), Some('+' | '-')) {
            exponent += 1;
        }
        if chars.get(exponent).is_some_and(char::is_ascii_digit) {
            is_real = true;
            end = digits_from(exponent);
        }
    }

    let text: String = chars[start..end].iter().collect();
    let number = match text.parse::<i64>() {
        Ok(i) if !is_real => Number::Integer(i),
        _ => Number::Real(text.parse().map_err(|_| CalcError::Invalid)?),
    };
    Ok((number, end))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,
}

#[derive(Debug, PartialEq)]
enum Expr {
    Number(Number),
    Name(String),
    Negate(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// Recursive descent over the usual precedence: `+ -`, then `* / %`, then unary minus, then `^`
/// (right-associative, so `-2^2` is -4 and `2^3^2` is 512).
struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, position: 0 }
    }

    fn parse(mut self) -> Result<Expr, CalcError> {
        let expr = self.sum()?;
        if self.position == self.tokens.len() { Ok(expr) } else { Err(CalcError::Invalid) }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn eat(&mut self, token: &Token) -> bool {
        let found = self.peek() == Some(token);
        if found {
            self.position += 1;
        }
        found
    }

    fn sum(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.product()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinaryOp::Add,
                Some(Token::Minus) => BinaryOp::Subtract,
                _ => return Ok(left),
            };
            self.position += 1;
            left = Expr::Binary(op, Box::new(left), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinaryOp::Multiply,
                Some(Token::Slash) => BinaryOp::Divide,
                Some(Token::Percent) => BinaryOp::Remainder,
                _ => return Ok(left),
            };
            self.position += 1;
            left = Expr::Binary(op, Box::new(left), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Result<Expr, CalcError> {
        if self.eat(&Token::Minus) {
            return Ok(Expr::Negate(Box::new(self.unary()?)));
        }
        if self.eat(&Token::Plus) {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<Expr, CalcError> {
        let base = self.primary()?;
        if self.eat(&Token::Power) {
            // The exponent may carry its own sign: 2^-1.
            return Ok(Expr::Binary(BinaryOp::Power, Box::new(base), Box::new(self.unary()?)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, CalcError> {
        let token = self.peek().cloned().ok_or(CalcError::Invalid)?;
        self.position += 1;
        match token {
            Token::Number(n) => Ok(Expr::Number(n)),
            Token::Open => {
                let inner = self.sum()?;
                if self.eat(&Token::Close) { Ok(inner) } else { Err(CalcError::Invalid) }
            }
            Token::Name(name) if self.eat(&Token::Open) => {
                let mut args = Vec::new();
                if !self.eat(&Token::Close) {
                    loop {
                        args.push(self.sum()?);
                        if self.eat(&Token::Close) {
                            break;
                        }
                        if !self.eat(&Token::Comma) {
                            return Err(CalcError::Invalid);
                        }
                    }
                }
                Ok(Expr::Call(name, args))
            }
            Token::Name(name) => Ok(Expr::Name(name)),
            _ => Err(CalcError::Invalid),
        }
    }
}

impl Expr {
    fn evaluate(&self, ans: Number) -> Result<Number, CalcError> {
        match self {
            Expr::Number(n) => Ok(*n),
            Expr::Name(name) => match name.to_ascii_lowercase().as_str() {
                "pi" => Ok(Number::Real(PI)),
                "e" => Ok(Number::Real(E)),
                "ans" => Ok(ans),
                _ => Err(CalcError::UnknownName),
            },
            Expr::Negate(inner) => Ok(match inner.evaluate(ans)? {
                Number::Integer(i) => i.checked_neg().map_or(Number::Real(-(i as f64)), Number::Integer),
                Number::Real(r) => Number::Real(-r),
            }),
            Expr::Binary(op, left, right) => binary(*op, left.evaluate(ans)?, right.evaluate(ans)?),
            Expr::Call(name, args) => {
                let args = args.iter().map(|a| a.evaluate(ans)).collect::<Result<Vec<_>, _>>()?;
                call(name, &args)
            }
        }
    }
}

fn binary(op: BinaryOp, left: Number, right: Number) -> Result<Number, CalcError> {
    if matches!(op, BinaryOp::Divide | BinaryOp::Remainder) && right.is_zero() {
        return Err(CalcError::DivideByZero);
    }

    if let (Number::Integer(a), Number::Integer(b)) = (left, right) {
        let exact = match op {
            BinaryOp::Add => a.checked_add(b),
            BinaryOp::Subtract => a.checked_sub(b),
            BinaryOp::Multiply => a.checked_mul(b),
            BinaryOp::Remainder => a.checked_rem(b),
            // Division and powers are real even for whole numbers: 7/2 is 3.5.
            BinaryOp::Divide | BinaryOp::Power => None,
        };
        if let Some(value) = exact {
            return Ok(Number::Integer(value));
        }
    }

    let (a, b) = (left.real(), right.real());
    let value = match op {
        BinaryOp::Add => a + b,
        BinaryOp::Subtract => a - b,
        BinaryOp::Multiply => a * b,
        BinaryOp::Divide => a / b,
        BinaryOp::Remainder => a % b,
        BinaryOp::Power => a.powf(b),
    };
    real(value, &[a, b])
}

/// Wraps a real result, reporting overflow from finite inputs as too large.
fn real(value: f64, inputs: &[f64]) -> Result<Number, CalcError> {
    if value.is_infinite() && inputs.iter().all(|i| i.is_finite()) {
        Err(CalcError::TooLarge)
    } else {
        Ok(Number::Real(value))
    }
}

/// The built-in functions, matched case-insensitively.
fn call(name: &str, args: &[Number]) -> Result<Number, CalcError> {
    let unary = |f: fn(f64) -> f64| match args {
        [x] => real(f(x.real()), &[x.real()]),
        _ => Err(CalcError::CantEvaluate),
    };
    let binary_real = |f: fn(f64, f64) -> f64| match args {
        [x, y] => real(f(x.real(), y.real()), &[x.real(), y.real()]),
        _ => Err(CalcError::CantEvaluate),
    };

    match name.to_ascii_lowercase().as_str() {
        "abs" => match args {
            [Number::Integer(i)] => Ok(i.checked_abs().map_or(Number::Real((*i as f64).abs()), Number::Integer)),
            _ => unary(f64::abs),
        },
        "acos" => unary(f64::acos),
        "asin" => unary(f64::asin),
        "atan" => unary(f64::atan),
        "atan2" => binary_real(f64::atan2),
        "ceiling" => unary(f64::ceil),
        "cos" => unary(f64::cos),
        "exp" => unary(f64::exp),
        "floor" => unary(f64::floor),
        "ieeeremainder" => binary_real(|x, y| x - y * (x / y).round_ties_even()),
        "ln" => unary(f64::ln),
        "log" => match args {
            [_] => unary(f64::log10),
            _ => binary_real(f64::log),
        },
        "log10" => unary(f64::log10),
        "max" => pick(args, i64::max, f64::max),
        "min" => pick(args, i64::min, f64::min),
        "pow" => binary_real(f64::powf),
        "round" => match args {
            [x] => Ok(Number::Real(x.real().round_ties_even())),
            [x, digits] => round(x.real(), digits.real()),
            _ => Err(CalcError::CantEvaluate),
        },
        "sign" => match args {
            [x] if x.real().is_nan() => Err(CalcError::Undefined),
            [x] => Ok(Number::Integer(if x.is_zero() { 0 } else if x.real() < 0.0 { -1 } else { 1 })),
            _ => Err(CalcError::CantEvaluate),
        },
        "sin" => unary(f64::sin),
        "sqrt" => unary(f64::sqrt),
        "tan" => unary(f64::tan),
        "truncate" => unary(f64::trunc),
        _ => Err(CalcError::UnknownFunction),
    }
}

/// max or min of two numbers, staying whole when both are.
fn pick(args: &[Number], integer: fn(i64, i64) -> i64, real: fn(f64, f64) -> f64) -> Result<Number, CalcError> {
    match args {
        [Number::Integer(a), Number::Integer(b)] => Ok(Number::Integer(integer(*a, *b))),
        [a, b] => Ok(Number::Real(real(a.real(), b.real()))),
        _ => Err(CalcError::CantEvaluate),
    }
}

/// Rounds to `digits` decimal places, halves to even, as .NET's Math.Round does.
fn round(value: f64, digits: f64) -> Result<Number, CalcError> {
    if digits.fract() != 0.0 || !(0.0..=15.0).contains(&digits) {
        return Err(CalcError::CantEvaluate);
    }

    let scale = 10f64.powi(digits as i32);
    Ok(Number::Real((value * scale).round_ties_even() / scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(input: &str) -> String {
        match evaluate(input, None) {
            Ok(answer) => answer.text,
            Err(error) => panic!("{input} failed: {error}"),
        }
    }

    #[test]
    fn evaluate_valid_input_formats_result() {
        let cases = [
            ("1+2", "3"),
            ("2*(3+4)", "14"),
            ("7/2", "3.5"),
            ("1/2", "0.5"),
            ("0.1+0.2", "0.3"),
            ("1/3", "0.333333333333333"),
            ("6×7", "42"),
            ("9÷3", "3"),
            ("5−8", "-3"),
            ("2^10", "1024"),
            ("2**10", "1024"),
            ("-2^2", "-4"),
            ("2^3^2", "512"),
            ("2^-1", "0.5"),
            ("sqrt(16)", "4"),
            ("Sqrt(2)*Sqrt(2)", "2"),
            ("round(2.567, 2)", "2.57"),
            ("round(2.5)", "2"),
            ("10 % 3", "1"),
            ("2147483647+1", "2147483648"),
            ("99999999999*99999999999", "9.9999999998E+21"),
            ("1e3*2", "2000"),
            ("pi*2", "6.28318530717959"),
            ("2π", "6.28318530717959"),
            ("(1)π", "3.14159265358979"),
            ("12+3 =", "15"),
            ("max(2, 7) - min(2, 7)", "5"),
            ("abs(-5)", "5"),
            ("log(8, 2)", "3"),
            ("0.00001", "1E-05"),
            ("0.0001", "0.0001"),
            ("1e15", "1E+15"),
            ("1e14", "100000000000000"),
            ("-0.0", "0"),
        ];

        for (input, expected) in cases {
            assert_eq!(text(input), expected, "{input}");
        }
    }

    #[test]
    fn evaluate_ans_is_previous_answer() {
        let answer = evaluate("ans*2", Some(Number::Integer(21))).unwrap();
        assert_eq!(answer.text, "42");
    }

    #[test]
    fn evaluate_without_previous_answer_ans_is_zero() {
        assert_eq!(text("ans+1"), "1");
    }

    #[test]
    fn evaluate_result_text_can_be_evaluated_again() {
        let first = text("99999999999*99999999999");
        assert_eq!(text(&format!("{first}+0")), first);
    }

    #[test]
    fn evaluate_invalid_input_explains_failure() {
        let cases = [
            ("", CalcError::Empty),
            ("  =", CalcError::Empty),
            ("3 +", CalcError::Invalid),
            ("(1+2", CalcError::Invalid),
            ("1 2", CalcError::Invalid),
            ("1/0", CalcError::DivideByZero),
            ("5/0.0", CalcError::DivideByZero),
            ("7 % 0", CalcError::DivideByZero),
            ("1e308*10", CalcError::TooLarge),
            ("sqrt(-1)", CalcError::Undefined),
            ("foo(2)", CalcError::UnknownFunction),
            ("x+1", CalcError::UnknownName),
            ("round(1, 2, 3)", CalcError::CantEvaluate),
            ("1/0 +", CalcError::Invalid),
        ];

        for (input, expected) in cases {
            assert_eq!(evaluate(input, None), Err(expected), "{input}");
        }
    }

    #[test]
    fn calc_error_display_reads_as_message() {
        assert_eq!(CalcError::DivideByZero.to_string(), "Can't divide by zero");
        assert_eq!(CalcError::UnknownName.to_string(), "Unknown name in expression");
    }
}
