/// Formula domain: arithmetic expression trees and the formula parser.
///
/// The self-reading variant is named `SelfRef` because `Self` is a reserved
/// Rust keyword. `eval` panics with `"Unknown_ref: <alias>"` when an alias is
/// not in `refs` (the `lookup` guard raises before calling `resolve`); the test
/// harness wraps that in `std::panic::catch_unwind`.
use crate::domain::ids::SensorId;

// ---------------------------------------------------------------------------
// Expr ADT
// ---------------------------------------------------------------------------

/// An arithmetic expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    /// The sensor's own reading (renamed from `self` because `Self` is a
    /// Rust keyword).
    SelfRef,
    /// A named alias reference (mapped to a `SensorId` in `Formula::Expr`).
    Ref(String),
    Abs(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
}

// ---------------------------------------------------------------------------
// Formula ADT
// ---------------------------------------------------------------------------

/// A sensor formula.
#[derive(Debug, Clone, PartialEq)]
pub enum Formula {
    Identity,
    Zero,
    Expr {
        /// Named alias → SensorId bindings used by the expression.
        refs: Vec<(String, SensorId)>,
        /// The expression tree.
        expr: Expr,
    },
}

// ---------------------------------------------------------------------------
// eval_expr (internal)
// ---------------------------------------------------------------------------

/// Evaluate an expression tree.
fn eval_expr(e: &Expr, self_reading: f64, resolve: &dyn Fn(&str) -> f64) -> f64 {
    match e {
        Expr::Num(n) => *n,
        Expr::SelfRef => self_reading,
        Expr::Ref(alias) => resolve(alias),
        Expr::Abs(inner) => eval_expr(inner, self_reading, resolve).abs(),
        Expr::Add(a, b) => eval_expr(a, self_reading, resolve) + eval_expr(b, self_reading, resolve),
        Expr::Sub(a, b) => eval_expr(a, self_reading, resolve) - eval_expr(b, self_reading, resolve),
        Expr::Mul(a, b) => eval_expr(a, self_reading, resolve) * eval_expr(b, self_reading, resolve),
        Expr::Div(a, b) => eval_expr(a, self_reading, resolve) / eval_expr(b, self_reading, resolve),
    }
}

// ---------------------------------------------------------------------------
// Formula public API
// ---------------------------------------------------------------------------

impl Formula {
    /// Evaluate the formula.
    ///
    /// For `Formula::Expr`, the `resolve` closure is only called for aliases
    /// present in `refs`; for any alias that is *not* in `refs` the function
    /// panics with `"Unknown_ref: <alias>"`.
    pub fn eval(&self, self_reading: f64, resolve: &dyn Fn(&str) -> f64) -> f64 {
        match self {
            Formula::Identity => self_reading,
            Formula::Zero => 0.0,
            Formula::Expr { refs, expr } => {
                let lookup = |alias: &str| {
                    if refs.iter().any(|(a, _)| a == alias) {
                        resolve(alias)
                    } else {
                        panic!("Unknown_ref: {}", alias)
                    }
                };
                eval_expr(expr, self_reading, &lookup)
            }
        }
    }

    /// Return the `SensorId`s referenced by this formula.
    pub fn referenced_ids(&self) -> Vec<SensorId> {
        match self {
            Formula::Identity | Formula::Zero => vec![],
            Formula::Expr { refs, .. } => refs.iter().map(|(_, id)| *id).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// expr helpers (public)
// ---------------------------------------------------------------------------

/// Return the distinct alias names in first-seen order.
pub fn expr_aliases(e: &Expr) -> Vec<String> {
    fn go(acc: &mut Vec<String>, e: &Expr) {
        match e {
            Expr::Num(_) | Expr::SelfRef => {}
            Expr::Ref(a) => {
                if !acc.contains(a) {
                    acc.push(a.clone());
                }
            }
            Expr::Abs(sub) => go(acc, sub),
            Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) => {
                go(acc, a);
                go(acc, b);
            }
        }
    }
    let mut acc = Vec::new();
    go(&mut acc, e);
    acc
}

/// Render an expression to source text with minimal parentheses.
///
/// Precedence: `+`/`-` = 1, `*`/`/` = 2; all binops left-associative.
pub fn expr_to_string(e: &Expr) -> String {
    // `outer` = precedence of the *parent* operator; `p` = this node's precedence.
    // Wraps only when the parent binds tighter.
    fn go(prec: u8, e: &Expr) -> String {
        match e {
            Expr::Num(n) => format!("{:.17e}", n)
                // `%.17g` suppresses trailing zeros and the exponent
                // for numbers that don't need it; replicate with a manual formatting.
                .parse::<f64>()
                .map_or_else(|_| format!("{}", n), |_| ocaml_g(*n)),
            Expr::SelfRef => "self".to_string(),
            Expr::Ref(a) => a.clone(),
            Expr::Abs(inner) => format!("abs({})", go(0, inner)),
            Expr::Add(a, b) => wrap(prec, 1, &format!("{} + {}", go(1, a), go(2, b))),
            Expr::Sub(a, b) => wrap(prec, 1, &format!("{} - {}", go(1, a), go(2, b))),
            Expr::Mul(a, b) => wrap(prec, 2, &format!("{} * {}", go(2, a), go(3, b))),
            Expr::Div(a, b) => wrap(prec, 2, &format!("{} / {}", go(2, a), go(3, b))),
        }
    }
    go(0, e)
}

/// Render an `f64` with `%.17g` format: up to 17 significant digits,
/// no trailing zeros, no exponent for "small" numbers.
///
/// `%g` rules: use scientific notation when exponent < -4 or exponent >= precision (17);
/// otherwise fixed.  Strip trailing zeros (and trailing decimal point).
fn ocaml_g(n: f64) -> String {
    if n.is_nan() {
        return "nan".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "infinity".to_string() } else { "-infinity".to_string() };
    }

    // Format with 16 decimal places in scientific notation (= 17 sig figs)
    let s = format!("{:.16e}", n);
    let (mant_s, exp_s) = s.split_once('e').unwrap();
    let exp: i32 = exp_s.parse().unwrap();

    // Strip trailing zeros from mantissa
    let mant_trimmed = mant_s.trim_end_matches('0').trim_end_matches('.');

    // %g: use fixed when -4 <= exp < precision (17)
    if (-4..17).contains(&exp) {
        // Reconstruct fixed notation.
        // mant_trimmed looks like "2.5", "-3.14", "1" etc.
        // The value equals mant * 10^exp where mant has exactly one integer digit.
        let (sign, pos) = if let Some(stripped) = mant_trimmed.strip_prefix('-') {
            ("-", stripped)
        } else {
            ("", mant_trimmed)
        };
        // Collect all significant digit characters
        let (int_digits, frac_digits) = if let Some((a, b)) = pos.split_once('.') {
            (a, b)
        } else {
            (pos, "")
        };
        let mut digits: Vec<char> = int_digits.chars().chain(frac_digits.chars()).collect();
        // `exp` tells us: the decimal point sits after digit index `exp + 1` (from left)
        let dot_pos = exp + 1; // may be 0 or negative → leading zeros needed
        if dot_pos <= 0 {
            // e.g. n = 0.0025, exp = -3: "0.00" + digits
            let leading_zeros = (-dot_pos) as usize;
            let frac: String = digits.iter().collect();
            let zeros = "0".repeat(leading_zeros);
            if frac.is_empty() {
                format!("{}0", sign)
            } else {
                format!("{}0.{}{}", sign, zeros, frac)
            }
        } else {
            let dp = dot_pos as usize;
            // Extend with trailing zeros if the decimal point sits beyond existing digits
            while digits.len() < dp {
                digits.push('0');
            }
            let (l, r) = digits.split_at(dp);
            let ls: String = l.iter().collect();
            let rs: String = r.iter().collect();
            if rs.is_empty() {
                format!("{}{}", sign, ls)
            } else {
                format!("{}{}.{}", sign, ls, rs)
            }
        }
    } else {
        // Scientific notation: <mant>e<exp>
        // Rust formats exp with sign and at least 2 digits; we want minimal digits.
        // e.g. Rust "1.5e10" → "1.5e+10": sign + no leading zero
        let exp_formatted = if exp >= 0 {
            format!("e+{}", exp)
        } else {
            format!("e{}", exp)
        };
        format!("{}{}", mant_trimmed, exp_formatted)
    }
}

fn wrap(outer: u8, p: u8, s: &str) -> String {
    if outer > p {
        format!("({})", s)
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------
//
// Grammar:
//   expr    := term (('+' | '-') term)*
//   term    := factor (('*' | '/') factor)*
//   factor  := '-' factor | primary
//   primary := number | 'self' | ident | 'abs' '(' expr ')' | '(' expr ')'
//
// `self` and `abs` are reserved; any other identifier becomes Ref(alias).

struct State {
    src: Vec<char>,
    pos: usize,
}

impl State {
    fn new(s: &str) -> Self {
        State { src: s.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.advance();
        }
    }
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    is_ident_start(c) || is_digit(c)
}

fn parse_number(st: &mut State) -> Result<Expr, String> {
    let start = st.pos;
    loop {
        match st.peek() {
            Some(c) if is_digit(c) || c == '.' || c == 'e' || c == 'E' => st.advance(),
            Some('+') | Some('-')
                if st.pos > start
                    && matches!(st.src.get(st.pos - 1), Some('e') | Some('E')) =>
            {
                st.advance()
            }
            _ => break,
        }
    }
    let tok: String = st.src[start..st.pos].iter().collect();
    match tok.parse::<f64>() {
        Ok(f) => Ok(Expr::Num(f)),
        Err(_) => Err(format!("invalid number {:?}", tok)),
    }
}

fn parse_ident(st: &mut State) -> String {
    let start = st.pos;
    while matches!(st.peek(), Some(c) if is_ident_char(c)) {
        st.advance();
    }
    st.src[start..st.pos].iter().collect()
}

fn parse_expr(st: &mut State) -> Result<Expr, String> {
    let left = parse_term(st)?;
    parse_expr_tail(st, left)
}

fn parse_expr_tail(st: &mut State, left: Expr) -> Result<Expr, String> {
    st.skip_ws();
    match st.peek() {
        Some('+') => {
            st.advance();
            let r = parse_term(st)?;
            parse_expr_tail(st, Expr::Add(Box::new(left), Box::new(r)))
        }
        Some('-') => {
            st.advance();
            let r = parse_term(st)?;
            parse_expr_tail(st, Expr::Sub(Box::new(left), Box::new(r)))
        }
        _ => Ok(left),
    }
}

fn parse_term(st: &mut State) -> Result<Expr, String> {
    let left = parse_factor(st)?;
    parse_term_tail(st, left)
}

fn parse_term_tail(st: &mut State, left: Expr) -> Result<Expr, String> {
    st.skip_ws();
    match st.peek() {
        Some('*') => {
            st.advance();
            let r = parse_factor(st)?;
            parse_term_tail(st, Expr::Mul(Box::new(left), Box::new(r)))
        }
        Some('/') => {
            st.advance();
            let r = parse_factor(st)?;
            parse_term_tail(st, Expr::Div(Box::new(left), Box::new(r)))
        }
        _ => Ok(left),
    }
}

fn parse_factor(st: &mut State) -> Result<Expr, String> {
    st.skip_ws();
    match st.peek() {
        Some('-') => {
            st.advance();
            let e = parse_factor(st)?;
            Ok(Expr::Sub(Box::new(Expr::Num(0.0)), Box::new(e)))
        }
        _ => parse_primary(st),
    }
}

fn parse_primary(st: &mut State) -> Result<Expr, String> {
    st.skip_ws();
    match st.peek() {
        None => Err("unexpected end of expression".to_string()),
        Some('(') => {
            st.advance();
            let e = parse_expr(st)?;
            st.skip_ws();
            match st.peek() {
                Some(')') => {
                    st.advance();
                    Ok(e)
                }
                _ => Err("expected ')'".to_string()),
            }
        }
        Some(c) if is_digit(c) || c == '.' => parse_number(st),
        Some(c) if is_ident_start(c) => {
            let name = parse_ident(st);
            match name.as_str() {
                "self" => Ok(Expr::SelfRef),
                "abs" => {
                    st.skip_ws();
                    match st.peek() {
                        Some('(') => {
                            st.advance();
                            let e = parse_expr(st)?;
                            st.skip_ws();
                            match st.peek() {
                                Some(')') => {
                                    st.advance();
                                    Ok(Expr::Abs(Box::new(e)))
                                }
                                _ => Err("expected ')' after abs(".to_string()),
                            }
                        }
                        _ => Err("expected '(' after abs".to_string()),
                    }
                }
                _ => Ok(Expr::Ref(name)),
            }
        }
        Some(c) => Err(format!("unexpected character {:?}", c)),
    }
}

/// Parse a formula expression string.
pub fn parse_expr_str(s: &str) -> Result<Expr, String> {
    let mut st = State::new(s);
    let e = parse_expr(&mut st)?;
    st.skip_ws();
    if st.pos < st.src.len() {
        Err(format!("unexpected trailing input near position {}", st.pos))
    } else {
        Ok(e)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Formula ADT tests
    // -----------------------------------------------------------------------

    #[test]
    fn identity_constructs() {
        let f = Formula::Identity;
        assert!(matches!(f, Formula::Identity));
    }

    #[test]
    fn expr_has_refs() {
        let id_a = SensorId::make(10001);
        let ast = Expr::Abs(Box::new(Expr::Sub(
            Box::new(Expr::SelfRef),
            Box::new(Expr::Ref("S1".to_string())),
        )));
        let f = Formula::Expr {
            refs: vec![("S1".to_string(), id_a)],
            expr: ast,
        };
        match &f {
            Formula::Expr { refs, .. } => {
                assert_eq!(refs.len(), 1);
                let (alias, sid) = &refs[0];
                assert_eq!(alias, "S1");
                assert_eq!(*sid, id_a);
            }
            _ => panic!("expected Expr"),
        }
    }

    #[test]
    fn deeply_nested_expr() {
        let id_a = SensorId::make(10001);
        let id_b = SensorId::make(10002);
        let ast = Expr::Abs(Box::new(Expr::Sub(
            Box::new(Expr::Sub(
                Box::new(Expr::SelfRef),
                Box::new(Expr::Ref("S4".to_string())),
            )),
            Box::new(Expr::Ref("S5".to_string())),
        )));
        let _f = Formula::Expr {
            refs: vec![("S4".to_string(), id_a), ("S5".to_string(), id_b)],
            expr: ast,
        };
        // Just compiles → pass
    }

    #[test]
    fn eval_identity() {
        let v = Formula::Identity.eval(7.5, &|_| panic!("should not call"));
        assert!((v - 7.5).abs() < 1e-9);
    }

    #[test]
    fn eval_arithmetic() {
        let id_a = SensorId::make(10001);
        let id_b = SensorId::make(10002);
        let ast = Expr::Sub(
            Box::new(Expr::SelfRef),
            Box::new(Expr::Add(
                Box::new(Expr::Ref("S1".to_string())),
                Box::new(Expr::Ref("S2".to_string())),
            )),
        );
        let f = Formula::Expr {
            refs: vec![("S1".to_string(), id_a), ("S2".to_string(), id_b)],
            expr: ast,
        };
        let v = f.eval(10.0, &|alias| match alias {
            "S1" => 3.0,
            "S2" => 1.0,
            _ => panic!("unknown alias"),
        });
        assert!((v - 6.0).abs() < 1e-9, "10 - (3 + 1) = 6, got {}", v);
    }

    #[test]
    fn eval_abs_flips_negative() {
        let id_a = SensorId::make(10001);
        let ast = Expr::Abs(Box::new(Expr::Sub(
            Box::new(Expr::SelfRef),
            Box::new(Expr::Ref("S1".to_string())),
        )));
        let f = Formula::Expr {
            refs: vec![("S1".to_string(), id_a)],
            expr: ast,
        };
        let v = f.eval(5.0, &|_| 12.0);
        assert!((v - 7.0).abs() < 1e-9, "|5 - 12| = 7, got {}", v);
    }

    #[test]
    fn eval_multiplier() {
        let ast = Expr::Mul(Box::new(Expr::SelfRef), Box::new(Expr::Num(2.5)));
        let f = Formula::Expr { refs: vec![], expr: ast };
        let v = f.eval(4.0, &|_| 0.0);
        assert!((v - 10.0).abs() < 1e-9, "4 * 2.5 = 10, got {}", v);
    }

    #[test]
    fn eval_div_by_zero_is_infinity() {
        let ast = Expr::Div(Box::new(Expr::SelfRef), Box::new(Expr::Num(0.0)));
        let f = Formula::Expr { refs: vec![], expr: ast };
        let v = f.eval(1.0, &|_| 0.0);
        assert!(v.is_infinite(), "expected infinite, got {}", v);
    }

    /// eval panics for an alias not in refs.
    #[test]
    fn eval_unknown_ref_raises() {
        let ast = Expr::Ref("missing".to_string());
        let f = Formula::Expr { refs: vec![], expr: ast };
        let result = std::panic::catch_unwind(|| {
            f.eval(0.0, &|_| 0.0);
        });
        assert!(result.is_err(), "expected panic for unknown ref");
        // Check the panic message contains the alias name
        if let Err(payload) = result {
            if let Some(s) = payload.downcast_ref::<String>() {
                assert!(s.contains("missing"), "panic message should mention alias: {}", s);
            }
            // &str panic messages are also acceptable
        }
    }

    #[test]
    fn referenced_ids_identity_empty() {
        let xs = Formula::Identity.referenced_ids();
        assert_eq!(xs.len(), 0);
    }

    #[test]
    fn eval_zero_returns_zero() {
        let v = Formula::Zero.eval(123.0, &|_| panic!("should not call"));
        assert_eq!(v, 0.0);
    }

    #[test]
    fn referenced_ids_zero_empty() {
        let xs = Formula::Zero.referenced_ids();
        assert_eq!(xs.len(), 0);
    }

    #[test]
    fn referenced_ids_expr() {
        let id_a = SensorId::make(10001);
        let id_b = SensorId::make(10002);
        let ast = Expr::Sub(
            Box::new(Expr::Ref("S1".to_string())),
            Box::new(Expr::Ref("S2".to_string())),
        );
        let f = Formula::Expr {
            refs: vec![("S1".to_string(), id_a), ("S2".to_string(), id_b)],
            expr: ast,
        };
        let mut xs: Vec<u32> = f.referenced_ids().iter().map(|s| s.id()).collect();
        xs.sort();
        assert_eq!(xs.len(), 2);
        assert_eq!(xs, vec![10001, 10002]);
    }

    #[test]
    fn expr_aliases_distinct_in_order() {
        let e = Expr::Abs(Box::new(Expr::Sub(
            Box::new(Expr::Sub(
                Box::new(Expr::SelfRef),
                Box::new(Expr::Ref("a".to_string())),
            )),
            Box::new(Expr::Add(
                Box::new(Expr::Ref("b".to_string())),
                Box::new(Expr::Ref("a".to_string())),
            )),
        )));
        let aliases = expr_aliases(&e);
        assert_eq!(aliases, vec!["a", "b"], "distinct aliases, first-seen order");
    }

    #[test]
    fn expr_to_string_minimal_parens() {
        // abs(self - a - b)  — no parens needed because Sub is left-associative
        let e = Expr::Abs(Box::new(Expr::Sub(
            Box::new(Expr::Sub(
                Box::new(Expr::SelfRef),
                Box::new(Expr::Ref("a".to_string())),
            )),
            Box::new(Expr::Ref("b".to_string())),
        )));
        assert_eq!(expr_to_string(&e), "abs(self - a - b)");
    }

    // -----------------------------------------------------------------------
    // Parser tests
    // -----------------------------------------------------------------------

    /// Helper: parse `s`, render with `expr_to_string`, compare to rendered `expected`.
    fn parse_ok_check(s: &str, expected: &Expr) {
        let got = parse_expr_str(s)
            .unwrap_or_else(|e| panic!("parse {:?} failed: {}", s, e));
        assert_eq!(
            expr_to_string(&got),
            expr_to_string(expected),
            "parse {:?}",
            s
        );
    }

    fn parse_err_check(s: &str) {
        assert!(
            parse_expr_str(s).is_err(),
            "expected parse error for {:?}",
            s
        );
    }

    #[test]
    fn parser_precedence_and_assoc() {
        // "self - a - b"  →  Sub(Sub(Self, a), b)  left-assoc
        parse_ok_check(
            "self - a - b",
            &Expr::Sub(
                Box::new(Expr::Sub(Box::new(Expr::SelfRef), Box::new(Expr::Ref("a".to_string())))),
                Box::new(Expr::Ref("b".to_string())),
            ),
        );
        // "self + a * b"  →  Add(Self, Mul(a, b))  * binds tighter
        parse_ok_check(
            "self + a * b",
            &Expr::Add(
                Box::new(Expr::SelfRef),
                Box::new(Expr::Mul(
                    Box::new(Expr::Ref("a".to_string())),
                    Box::new(Expr::Ref("b".to_string())),
                )),
            ),
        );
        // "(self + a) * b"  →  Mul(Add(Self, a), b)
        parse_ok_check(
            "(self + a) * b",
            &Expr::Mul(
                Box::new(Expr::Add(
                    Box::new(Expr::SelfRef),
                    Box::new(Expr::Ref("a".to_string())),
                )),
                Box::new(Expr::Ref("b".to_string())),
            ),
        );
        // "abs(self - a - b)"
        parse_ok_check(
            "abs(self - a - b)",
            &Expr::Abs(Box::new(Expr::Sub(
                Box::new(Expr::Sub(Box::new(Expr::SelfRef), Box::new(Expr::Ref("a".to_string())))),
                Box::new(Expr::Ref("b".to_string())),
            ))),
        );
        // "self * -1"  →  Mul(Self, Sub(Num 0, Num 1))
        parse_ok_check(
            "self * -1",
            &Expr::Mul(
                Box::new(Expr::SelfRef),
                Box::new(Expr::Sub(Box::new(Expr::Num(0.0)), Box::new(Expr::Num(1.0)))),
            ),
        );
        // "2.5 * self"
        parse_ok_check(
            "2.5 * self",
            &Expr::Mul(Box::new(Expr::Num(2.5)), Box::new(Expr::SelfRef)),
        );
    }

    #[test]
    fn parser_rejects_garbage() {
        parse_err_check("");
        parse_err_check("self +");
        parse_err_check("abs self");
        parse_err_check("(self + a");
        parse_err_check("self # a");
    }
}
