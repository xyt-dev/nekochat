// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A bounded expression compiler. Sampling executes stack instructions without allocation.

use super::Checked;

pub(super) fn identifier(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[derive(Clone, Copy)]
enum Function { Sin, Cos, Tan, Asin, Acos, Atan, Sqrt, Abs, Exp, Log, Floor, Ceil, Round, Min, Max, Pow }

impl Function {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "sin" => Self::Sin, "cos" => Self::Cos, "tan" => Self::Tan,
            "asin" => Self::Asin, "acos" => Self::Acos, "atan" => Self::Atan,
            "sqrt" => Self::Sqrt, "abs" => Self::Abs, "exp" => Self::Exp,
            "log" | "ln" => Self::Log, "floor" => Self::Floor,
            "ceil" => Self::Ceil, "round" => Self::Round,
            "min" => Self::Min, "max" => Self::Max, "pow" => Self::Pow,
            _ => return None,
        })
    }

    fn arity(self) -> usize {
        match self { Self::Min | Self::Max | Self::Pow => 2, _ => 1 }
    }

    fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            Self::Sin => a.sin(), Self::Cos => a.cos(), Self::Tan => a.tan(),
            Self::Asin => a.asin(), Self::Acos => a.acos(), Self::Atan => a.atan(),
            Self::Sqrt => a.sqrt(), Self::Abs => a.abs(), Self::Exp => a.exp(),
            Self::Log => a.ln(), Self::Floor => a.floor(), Self::Ceil => a.ceil(),
            Self::Round => a.round(), Self::Min => if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) },
            Self::Max => if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) },
            Self::Pow => a.powf(b),
        }
    }
}

#[derive(Clone, Copy)]
enum Op { Literal(f64), Variable(usize), Neg, Add, Sub, Mul, Div, Rem, Pow, Function(Function) }

pub(super) struct Expression { code: Vec<Op> }

impl Expression {
    pub(super) fn parse(source: &str, names: &[&str]) -> Checked<Self> {
        if source.is_empty() || source.len() > 256 || !source.is_ascii() {
            return Err("Plot expressions must contain 1–256 ASCII characters".into());
        }
        let mut parser = Parser { source: source.as_bytes(), pos: 0, names, code: Vec::with_capacity(source.len()) };
        parser.expr(0, 0)?;
        parser.space();
        if parser.pos != parser.source.len() { return Err(format!("Unexpected expression token at byte {}", parser.pos)); }
        // Validate stack use once; the evaluator then needs neither checks nor allocations.
        let mut stack = 0usize;
        for op in &parser.code {
            match op {
                Op::Literal(_) | Op::Variable(_) => stack += 1,
                Op::Neg => if stack == 0 { return Err("Invalid expression".into()); },
                Op::Function(f) => {
                    if stack < f.arity() { return Err("Invalid function arguments".into()); }
                    stack -= f.arity() - 1;
                }
                _ => {
                    if stack < 2 { return Err("Invalid expression operands".into()); }
                    stack -= 1;
                }
            }
            if stack > 128 { return Err("Expression stack exceeds its limit".into()); }
        }
        if stack != 1 { return Err("Invalid expression".into()); }
        Ok(Self { code: parser.code })
    }

    pub(super) fn evaluate(&self, variables: &[f64; 9], stack: &mut [f64; 128]) -> f64 {
        let mut count = 0;
        for op in &self.code {
            match *op {
                Op::Literal(v) => { stack[count] = v; count += 1; }
                Op::Variable(i) => { stack[count] = variables[i]; count += 1; }
                Op::Neg => stack[count - 1] = -stack[count - 1],
                Op::Function(f) => {
                    let (a, b) = if f.arity() == 2 {
                        count -= 1; (stack[count - 1], stack[count])
                    } else { (stack[count - 1], 0.0) };
                    stack[count - 1] = f.apply(a, b);
                }
                op => {
                    count -= 1;
                    let (a, b) = (stack[count - 1], stack[count]);
                    stack[count - 1] = match op {
                        Op::Add => a + b, Op::Sub => a - b, Op::Mul => a * b,
                        Op::Div => a / b, Op::Rem => a % b, Op::Pow => a.powf(b),
                        _ => unreachable!(),
                    };
                }
            }
        }
        stack[0]
    }
}

struct Parser<'s, 'n> { source: &'s [u8], pos: usize, names: &'n [&'n str], code: Vec<Op> }

impl Parser<'_, '_> {
    fn space(&mut self) {
        while self.source.get(self.pos).is_some_and(u8::is_ascii_whitespace) { self.pos += 1; }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        if self.source.get(self.pos) == Some(&c) { self.pos += 1; true } else { false }
    }

    fn expr(&mut self, binding: u8, depth: usize) -> Checked<()> {
        if depth > 64 { return Err("Expression nesting exceeds 64 levels".into()); }
        self.space();
        match self.source.get(self.pos).copied() {
            Some(b'+') => { self.pos += 1; self.expr(5, depth + 1)?; }
            Some(b'-') => { self.pos += 1; self.expr(5, depth + 1)?; self.code.push(Op::Neg); }
            Some(b'(') => {
                self.pos += 1; self.expr(0, depth + 1)?;
                if !self.eat(b')') { return Err("Missing closing expression parenthesis".into()); }
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.literal()?,
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => self.name(depth)?,
            _ => return Err(format!("Expected expression at byte {}", self.pos)),
        }
        loop {
            self.space();
            let (left, right, op, width) = match self.source.get(self.pos) {
                Some(b'+') => (1, 2, Op::Add, 1), Some(b'-') => (1, 2, Op::Sub, 1),
                Some(b'*') if self.source.get(self.pos + 1) == Some(&b'*') => (7, 7, Op::Pow, 2),
                Some(b'*') => (3, 4, Op::Mul, 1), Some(b'/') => (3, 4, Op::Div, 1),
                Some(b'%') => (3, 4, Op::Rem, 1), Some(b'^') => (7, 7, Op::Pow, 1),
                _ => break,
            };
            if left < binding { break; }
            self.pos += width;
            self.expr(right, depth + 1)?;
            self.code.push(op);
        }
        Ok(())
    }

    fn literal(&mut self) -> Checked<()> {
        let start = self.pos;
        while self.source.get(self.pos).is_some_and(u8::is_ascii_digit) { self.pos += 1; }
        if self.source.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            while self.source.get(self.pos).is_some_and(u8::is_ascii_digit) { self.pos += 1; }
        }
        if matches!(self.source.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.source.get(self.pos), Some(b'+' | b'-')) { self.pos += 1; }
            while self.source.get(self.pos).is_some_and(u8::is_ascii_digit) { self.pos += 1; }
        }
        let value: f64 = std::str::from_utf8(&self.source[start..self.pos]).expect("ASCII expression")
            .parse().map_err(|_| "Invalid numeric expression literal")?;
        if !value.is_finite() { return Err("Expression literals must be finite".into()); }
        self.code.push(Op::Literal(value));
        Ok(())
    }

    fn name(&mut self, depth: usize) -> Checked<()> {
        let start = self.pos;
        while self.source.get(self.pos).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') { self.pos += 1; }
        let name = std::str::from_utf8(&self.source[start..self.pos]).expect("ASCII expression");
        if self.eat(b'(') {
            let function = Function::parse(name).ok_or_else(|| format!("Unknown plot function: {name}"))?;
            self.expr(0, depth + 1)?;
            if function.arity() == 2 {
                if !self.eat(b',') { return Err(format!("{name} requires two arguments")); }
                self.expr(0, depth + 1)?;
            }
            if !self.eat(b')') { return Err(format!("Invalid arguments for {name}")); }
            self.code.push(Op::Function(function));
        } else {
            self.code.push(match name {
                "x" => Op::Variable(0),
                "pi" => Op::Literal(std::f64::consts::PI),
                "e" => Op::Literal(std::f64::consts::E),
                "tau" => Op::Literal(std::f64::consts::TAU),
                _ => Op::Variable(self.names.iter().position(|n| *n == name)
                    .ok_or_else(|| format!("Unknown plot variable: {name}"))? + 1),
            });
        }
        Ok(())
    }
}
