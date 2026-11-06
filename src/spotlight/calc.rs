//! The Spotlight calculator: `2+3*4`, `sqrt(16)`, `2^10`, `sin(pi/2)`, `15%`.

/// The result of `input`, or `None` when it is not an expression worth calculating.
pub fn evaluate(input: &str) -> Option<f64> {
    let input = input.trim().trim_start_matches('=').trim();
    if input.is_empty() || !looks_like_math(input) {
        return None;
    }
    let mut parser = Parser { chars: input.chars().collect(), pos: 0, depth: 0 };
    let value = parser.expression()?;
    parser.skip_spaces();
    if parser.pos != parser.chars.len() || !value.is_finite() {
        return None;
    }
    Some(value)
}

/// A lone number is a search, not a calculation.
fn looks_like_math(input: &str) -> bool {
    let has_digit_or_constant = input.chars().any(|c| c.is_ascii_digit()) || contains_word(input, "pi");
    let has_operator = input.chars().any(|c| "+-*/^%×÷(".contains(c));
    has_digit_or_constant && has_operator
}

fn contains_word(input: &str, word: &str) -> bool {
    input.to_lowercase().split(|c: char| !c.is_alphabetic()).any(|w| w == word)
}

/// "3.14", "1 234,5" stays machine-readable: plain digits, a dot and at most ten decimals.
pub fn format_result(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    if value.abs() >= 1e15 || value.abs() < 1e-6 {
        return format!("{value:e}");
    }
    let mut text = format!("{value:.10}");
    while text.ends_with('0') {
        text.pop();
    }
    text.trim_end_matches('.').to_string()
}

const MAX_DEPTH: u32 = 64;

struct Parser {
    chars: Vec<char>,
    pos: usize,
    depth: u32,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, wanted: &[char]) -> Option<char> {
        self.skip_spaces();
        let next = self.peek()?;
        if wanted.contains(&next) {
            self.pos += 1;
            Some(next)
        } else {
            None
        }
    }

    fn expression(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let mut value = self.term()?;
        while let Some(op) = self.eat(&['+', '-']) {
            let right = self.term()?;
            value = if op == '+' { value + right } else { value - right };
        }
        self.depth -= 1;
        Some(value)
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if let Some(op) = self.eat(&['*', '×', '/', '÷']) {
                let right = self.unary()?;
                value = if op == '*' || op == '×' { value * right } else { value / right };
            } else if self.eat(&['%']).is_some() {
                // "15%" is 0.15; "7 % 3" is the remainder.
                self.skip_spaces();
                if self.peek().is_some_and(|c| c.is_ascii_digit() || c == '(') {
                    let right = self.unary()?;
                    value %= right;
                } else {
                    value /= 100.0;
                }
            } else {
                return Some(value);
            }
        }
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat(&['-']).is_some() {
            return Some(-self.unary()?);
        }
        if self.eat(&['+']).is_some() {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat(&['^']).is_some() {
            // Right associative: 2^3^2 is 2^9.
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        self.skip_spaces();
        let next = self.peek()?;
        if next == '(' {
            self.pos += 1;
            let value = self.expression()?;
            self.eat(&[')'])?;
            return Some(value);
        }
        if next.is_ascii_digit() || next == '.' || next == ',' {
            return self.number();
        }
        if next.is_alphabetic() {
            return self.word();
        }
        None
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.' || c == ',') {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().map(|c| if *c == ',' { '.' } else { *c }).collect();
        text.parse().ok()
    }

    fn word(&mut self) -> Option<f64> {
        let start = self.pos;
        while self.peek().is_some_and(char::is_alphanumeric) {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().flat_map(|c| c.to_lowercase()).collect();
        match name.as_str() {
            "pi" => return Some(std::f64::consts::PI),
            "e" => return Some(std::f64::consts::E),
            _ => {}
        }
        self.eat(&['('])?;
        let argument = self.expression()?;
        self.eat(&[')'])?;
        Some(match name.as_str() {
            "sqrt" => argument.sqrt(),
            "sin" => argument.sin(),
            "cos" => argument.cos(),
            "tan" => argument.tan(),
            "asin" => argument.asin(),
            "acos" => argument.acos(),
            "atan" => argument.atan(),
            "ln" => argument.ln(),
            "log" => argument.log10(),
            "abs" => argument.abs(),
            "round" => argument.round(),
            "floor" => argument.floor(),
            "ceil" => argument.ceil(),
            "exp" => argument.exp(),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(input: &str, expected: f64) {
        let value = evaluate(input).unwrap_or_else(|| panic!("{input} did not evaluate"));
        assert!((value - expected).abs() < 1e-9, "{input} = {value}, expected {expected}");
    }

    #[test]
    fn follows_operator_precedence() {
        close("2+3*4", 14.0);
        close("(2+3)*4", 20.0);
        close("10/4", 2.5);
        close("2^10", 1024.0);
        close("2^3^2", 512.0);
        close("-2^2", -4.0);
        close("2*-3", -6.0);
    }

    #[test]
    fn knows_functions_and_constants() {
        close("sqrt(16)+1", 5.0);
        close("sin(pi/2)", 1.0);
        close("log(1000)*1", 3.0);
        close("abs(-3)+0", 3.0);
        close("=1+1", 2.0);
    }

    #[test]
    fn handles_percent_and_european_decimals() {
        close("15%", 0.15);
        close("7 % 3", 1.0);
        close("200*15%", 30.0);
        close("1,5+1", 2.5);
        close("6×7", 42.0);
        close("9÷3", 3.0);
    }

    #[test]
    fn ignores_what_is_not_math() {
        for text in ["", "chrome", "42", "hello world", "2024", "notepad++", "1+", "(1+2", "1+2)", "foo(3)", "1/0"] {
            assert!(evaluate(text).is_none(), "{text} should not evaluate");
        }
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        assert!(evaluate(&deep).is_none());
    }

    #[test]
    fn results_are_formatted_cleanly() {
        assert_eq!(format_result(14.0), "14");
        assert_eq!(format_result(2.5), "2.5");
        assert_eq!(format_result(0.1 + 0.2), "0.3");
        assert_eq!(format_result(-3.0), "-3");
    }
}
