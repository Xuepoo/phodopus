//! Fixed Lua pattern engine for quantified-inner captures (issue #56).
//!
//! Upstream `lsonar 0.2.4` parses `(.-)` as `Capture { inner: [Quantified] }`
//! but its `Capture` arm matches `inner` once with an empty continuation and
//! never retries longer expansions when the outer continuation (`%s*$`)
//! fails. The idiomatic trim `^%s*(.-)%s*$` therefore returns `nil` instead
//! of the trimmed string.
//!
//! This module reuses `lsonar`'s AST but matches over a desugared
//! form where every capture becomes explicit `CaptureStart` / `CaptureEnd`
//! markers. Quantifiers inside a capture then see the capture-end marker plus
//! the outer continuation as their `remaining`, so the existing
//! greedy/non-greedy backtracking naturally expands the capture until the
//! outer pattern matches (PUC Lua 5.1 semantics). `Quantified` bodies are
//! sequences (usually one element) so a quantified capture such as `(a)+`
//! repeats its full marker span.
//!
//! Byte orientation (bitty-terminal/bitty#1826): Lua strings are arbitrary
//! bytes, not necessarily UTF-8. The upstream `lsonar::Parser::new` takes
//! `&str`, so a pattern embedding isolated bytes such as `\128` (0x80) cannot
//! even be represented and `String::to_str` rejects it before matching.
//! This module therefore parses patterns from `&[u8]` with a byte-faithful
//! replica of the upstream lexer/parser. The matcher already operates on
//! bytes; captures are returned as byte vectors so no lossy UTF-8 conversion
//! can corrupt them.

use std::iter::Peekable;
use std::ops::Range;
use std::rc::Rc;
use std::vec::IntoIter;

use lsonar::{AstNode, CharSet, LUA_MAXCAPTURES, Quantifier, Token};

/// Byte-faithful replica of the upstream `lsonar` lexer/parser over `&[u8]`.
///
/// Upstream `Lexer::new`/`Parser::new` take `&str`, which cannot represent a
/// pattern containing isolated non-UTF8 bytes (e.g. Lua `'\128'` = 0x80).
/// The logic below operates directly on bytes and produces the same
/// `AstNode` stream — including identical error strings — so valid Lua 5.1
/// byte-patterns parse instead of failing UTF-8 validation.
fn is_class_byte(c: u8) -> bool {
    matches!(
        c,
        b'a' | b'c'
            | b'd'
            | b'g'
            | b'l'
            | b'p'
            | b's'
            | b'u'
            | b'w'
            | b'x'
            | b'A'
            | b'C'
            | b'D'
            | b'G'
            | b'L'
            | b'P'
            | b'S'
            | b'U'
            | b'W'
            | b'X'
    )
}

fn is_escapable_magic_byte(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'.' | b'%' | b'[' | b']' | b'*' | b'+' | b'-' | b'?' | b'^' | b'$'
    )
}

struct ByteLexer<'a> {
    input: &'a [u8],
    pos: usize,
    capture_depth: usize,
    set_depth: usize,
}

impl<'a> ByteLexer<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            capture_depth: 0,
            set_depth: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let byte = self.peek();
        if byte.is_some() {
            self.pos += 1;
        }
        byte
    }

    fn next_token(&mut self) -> Result<Option<Token>, lsonar::Error> {
        let Some(byte) = self.advance() else {
            return Ok(None);
        };

        match byte {
            b'(' => {
                self.capture_depth += 1;
                Ok(Some(Token::LParen))
            }
            b')' => {
                if self.capture_depth > 0 {
                    self.capture_depth -= 1;
                    Ok(Some(Token::RParen))
                } else {
                    Ok(Some(Token::Literal(b')')))
                }
            }
            b'.' => Ok(Some(Token::Any)),
            b'[' => {
                self.set_depth += 1;
                Ok(Some(Token::LBracket))
            }
            b']' => {
                if self.set_depth > 0 {
                    self.set_depth -= 1;
                    Ok(Some(Token::RBracket))
                } else {
                    Ok(Some(Token::Literal(byte)))
                }
            }
            b'^' => Ok(Some(Token::Caret)),
            b'$' => Ok(Some(Token::Dollar)),
            b'*' => {
                if self.set_depth > 0 {
                    Ok(Some(Token::Literal(b'*')))
                } else {
                    Ok(Some(Token::Star))
                }
            }
            b'+' => {
                if self.set_depth > 0 {
                    Ok(Some(Token::Literal(b'+')))
                } else {
                    Ok(Some(Token::Plus))
                }
            }
            b'?' => {
                if self.set_depth > 0 {
                    Ok(Some(Token::Literal(b'?')))
                } else {
                    Ok(Some(Token::Question))
                }
            }
            b'-' => {
                if self.set_depth > 0 {
                    Ok(Some(Token::Literal(b'-')))
                } else {
                    Ok(Some(Token::Minus))
                }
            }
            b'%' => {
                if self.set_depth > 0 {
                    if let Some(next_byte) = self.peek() {
                        match next_byte {
                            byte if is_class_byte(next_byte) => {
                                self.advance();
                                Ok(Some(Token::Class(byte)))
                            }
                            byte if is_escapable_magic_byte(next_byte) => {
                                self.advance();
                                Ok(Some(Token::EscapedLiteral(byte)))
                            }
                            b'%' => {
                                self.advance();
                                Ok(Some(Token::EscapedLiteral(b'%')))
                            }
                            _ => Err(lsonar::Error::Lexer(format!(
                                "malformed pattern (invalid escape sequence in set: %{})",
                                next_byte
                            ))),
                        }
                    } else {
                        Err(lsonar::Error::Lexer(
                            "malformed pattern (ends with '%' inside set)".to_string(),
                        ))
                    }
                } else {
                    let Some(next_byte) = self.advance() else {
                        return Err(lsonar::Error::Lexer(
                            "malformed pattern (ends with '%')".to_string(),
                        ));
                    };
                    match next_byte {
                        c if is_escapable_magic_byte(c) => Ok(Some(Token::EscapedLiteral(c))),
                        c if is_class_byte(c) => Ok(Some(Token::Class(c))),
                        b'%' => {
                            self.advance();
                            Ok(Some(Token::EscapedLiteral(b'%')))
                        }
                        b'b' => {
                            let Some(d1) = self.advance() else {
                                return Err(lsonar::Error::Lexer(
                                    "malformed pattern (%b needs two characters)".to_string(),
                                ));
                            };
                            let Some(d2) = self.advance() else {
                                return Err(lsonar::Error::Lexer(
                                    "malformed pattern (%b needs two characters)".to_string(),
                                ));
                            };
                            Ok(Some(Token::Balanced(d1, d2)))
                        }
                        b'f' => Ok(Some(Token::Frontier)),
                        d @ b'1'..=b'9' => Ok(Some(Token::CaptureRef(d - b'0'))),
                        _ => Err(lsonar::Error::Lexer(format!(
                            "malformed pattern (invalid escape sequence in set: %{})",
                            next_byte
                        ))),
                    }
                }
            }
            _ => Ok(Some(Token::Literal(byte))),
        }
    }
}

const fn token_to_byte(token: &Token) -> u8 {
    match token {
        Token::Literal(b) => *b,
        Token::EscapedLiteral(b) => *b,
        Token::Any => b'.',
        Token::LParen => b'(',
        Token::RParen => b')',
        Token::LBracket => b'[',
        Token::RBracket => b']',
        Token::Caret => b'^',
        Token::Dollar => b'$',
        Token::Star => b'*',
        Token::Plus => b'+',
        Token::Question => b'?',
        Token::Minus => b'-',
        Token::Percent => b'%',
        Token::Class(c) => *c,
        Token::Balanced(_, _) => b'b',
        Token::Frontier => b'f',
        Token::CaptureRef(d) => b'0' + *d,
    }
}

struct ByteParser {
    tokens: Peekable<IntoIter<Token>>,
    capture_count: usize,
}

impl ByteParser {
    fn new(pattern: &[u8]) -> Result<Self, lsonar::Error> {
        let mut lexer = ByteLexer::new(pattern);
        let mut token_vec = Vec::new();
        loop {
            match lexer.next_token() {
                Ok(Some(token)) => token_vec.push(token),
                Ok(None) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(Self {
            tokens: token_vec.into_iter().peekable(),
            capture_count: 0,
        })
    }

    fn parse(&mut self) -> Result<Vec<AstNode>, lsonar::Error> {
        let ast = self.parse_sequence(None)?;

        if let Some(token) = self.tokens.peek() {
            return Err(lsonar::Error::Parser(format!(
                "malformed pattern (unexpected token {token:?} after end of pattern)"
            )));
        }

        if self.capture_count > LUA_MAXCAPTURES {
            return Err(lsonar::Error::Parser(format!(
                "pattern has too many captures (limit is {LUA_MAXCAPTURES})"
            )));
        }

        Ok(ast)
    }

    fn parse_sequence(&mut self, end_token: Option<&Token>) -> Result<Vec<AstNode>, lsonar::Error> {
        let mut ast = Vec::new();

        while self.tokens.peek().is_some() && self.tokens.peek() != end_token {
            ast.push(self.parse_item()?);
        }

        if end_token.is_some() && self.tokens.peek() != end_token {
            return Err(lsonar::Error::Parser(format!(
                "malformed pattern (unexpected end, expected {end_token:?})"
            )));
        }

        Ok(ast)
    }

    fn parse_item(&mut self) -> Result<AstNode, lsonar::Error> {
        let mut base_item = self.parse_base()?;

        let quantifier = match self.tokens.peek() {
            Some(Token::Star) => Some(Quantifier::Star),
            Some(Token::Plus) => Some(Quantifier::Plus),
            Some(Token::Question) => Some(Quantifier::Question),
            Some(Token::Minus) => Some(Quantifier::Minus),
            _ => None,
        };

        if let Some(q) = quantifier {
            self.tokens.next();

            match base_item {
                AstNode::AnchorStart | AstNode::AnchorEnd | AstNode::Frontier(_) => {
                    return Err(lsonar::Error::Parser(
                        "pattern item cannot be quantified".to_string(),
                    ));
                }
                _ => {}
            }

            base_item = AstNode::Quantified {
                item: Box::new(base_item),
                quantifier: q,
            };
        }

        Ok(base_item)
    }

    fn parse_base(&mut self) -> Result<AstNode, lsonar::Error> {
        let Some(token) = self.tokens.next() else {
            return Err(lsonar::Error::Parser(
                "unexpected end of pattern".to_string(),
            ));
        };

        match token {
            Token::Literal(b')') => Err(lsonar::Error::Parser(
                "malformed pattern (unexpected ')')".to_string(),
            )),
            Token::Literal(b']') => Err(lsonar::Error::Parser(
                "malformed pattern (unexpected ']')".to_string(),
            )),
            Token::Literal(b) => Ok(AstNode::Literal(b)),
            Token::EscapedLiteral(b) => Ok(AstNode::Literal(b)),
            Token::Any => Ok(AstNode::Any),
            Token::Caret => Ok(AstNode::AnchorStart),
            Token::Dollar => Ok(AstNode::AnchorEnd),

            Token::Class(c) => {
                let negated = c.is_ascii_uppercase();
                let base_byte = if negated { c.to_ascii_lowercase() } else { c };
                if b"acdglpsuwx".contains(&base_byte) {
                    Ok(AstNode::Class(base_byte, negated))
                } else {
                    Ok(AstNode::Literal(c))
                }
            }

            Token::LBracket => self.parse_set(),

            Token::LParen => self.parse_capture(),

            Token::Balanced(d1, d2) => Ok(AstNode::Balanced(d1, d2)),
            Token::Frontier => {
                if self.tokens.peek() != Some(&Token::LBracket) {
                    return Err(lsonar::Error::Parser(
                        "malformed pattern (missing '[' after %f)".to_string(),
                    ));
                }
                self.tokens.next();
                let set_node = self.parse_set()?;
                if let AstNode::Set(charset) = set_node {
                    Ok(AstNode::Frontier(charset))
                } else {
                    unreachable!("parse_set should return AstNode::Set");
                }
            }

            Token::RParen => Err(lsonar::Error::Parser(
                "invalid pattern (unexpected ')')".to_string(),
            )),
            Token::RBracket => Err(lsonar::Error::Parser(
                "invalid pattern (unexpected ']')".to_string(),
            )),
            Token::Star | Token::Plus | Token::Question => Err(lsonar::Error::Parser(format!(
                "invalid pattern (quantifier '{}' must follow an item)",
                token_to_byte(&token)
            ))),
            Token::Minus => Ok(AstNode::Literal(b'-')),
            Token::Percent => Err(lsonar::Error::Parser(
                "internal error: Percent token should not reach parser base".to_string(),
            )),
            Token::CaptureRef(n) => Ok(AstNode::CaptureRef(n as usize)),
        }
    }

    fn parse_set(&mut self) -> Result<AstNode, lsonar::Error> {
        let mut set = CharSet::new();
        let mut negated = false;

        if self.tokens.peek() == Some(&Token::Caret) {
            self.tokens.next();
            negated = true;
        }

        if self.tokens.peek() == Some(&Token::RBracket) {
            self.tokens.next();
            if negated {
                set.invert();
            }
            return Ok(AstNode::Set(set));
        }

        if self.tokens.peek() == Some(&Token::RBracket) {
            self.tokens.next();
            set.add_byte(b']');
        }

        while self.tokens.peek().is_some() && self.tokens.peek() != Some(&Token::RBracket) {
            match self.tokens.peek().cloned() {
                Some(Token::Class(c)) => {
                    self.tokens.next();
                    set.add_class(c)?;
                }
                Some(Token::Literal(b)) => {
                    let current_byte = b;
                    self.tokens.next();

                    if self.tokens.peek() == Some(&Token::Literal(b'-')) {
                        let mut iter_clone = self.tokens.clone();
                        iter_clone.next();

                        if let Some(Token::Literal(next_b)) = iter_clone.peek() {
                            let next_b_val = *next_b;
                            self.tokens.next();
                            self.tokens.next();
                            set.add_range(current_byte, next_b_val)?;
                        } else {
                            set.add_byte(current_byte);
                        }
                    } else {
                        set.add_byte(current_byte);
                    }
                }
                Some(Token::Minus) => {
                    self.tokens.next();
                    set.add_byte(b'-');
                }
                Some(Token::Percent) => {
                    self.tokens.next();
                    set.add_byte(b'%');
                }
                Some(_) => {
                    let token = self.tokens.next().unwrap();
                    let byte = token_to_byte(&token);
                    set.add_byte(byte);
                }
                None => unreachable!(),
            }
        }

        if self.tokens.peek() == Some(&Token::RBracket) {
            self.tokens.next();
        } else {
            return Err(lsonar::Error::Parser(
                "malformed pattern (unfinished character class)".to_string(),
            ));
        }

        if negated {
            set.invert();
        }

        Ok(AstNode::Set(set))
    }

    fn parse_capture(&mut self) -> Result<AstNode, lsonar::Error> {
        self.capture_count += 1;
        let index = self.capture_count;
        if index > LUA_MAXCAPTURES {
            return Err(lsonar::Error::Parser(format!(
                "pattern has too many captures (limit is {LUA_MAXCAPTURES})"
            )));
        }

        let inner_ast = self.parse_sequence(Some(&Token::RParen))?;

        if self.tokens.next() != Some(Token::RParen) {
            return Err(lsonar::Error::Parser(
                "malformed pattern (unclosed capture group)".to_string(),
            ));
        }

        Ok(AstNode::Capture {
            index,
            inner: inner_ast,
        })
    }
}

/// Parse a Lua pattern from raw bytes (Lua 5.1 semantics).
///
/// Unlike `lsonar::Parser::new(&str)`, this accepts patterns containing
/// isolated non-UTF8 bytes such as `\128` (0x80).
pub(crate) fn parse_pattern(pattern: &[u8]) -> Result<Vec<AstNode>, lsonar::Error> {
    let mut parser = ByteParser::new(pattern)?;
    parser.parse()
}

/// Flat pattern with explicit capture boundaries.
#[derive(Clone, Debug)]
enum Pat {
    Literal(u8),
    Any,
    Class(u8, bool),
    Set(lsonar::CharSet),
    Balanced(u8, u8),
    Frontier(lsonar::CharSet),
    AnchorStart,
    AnchorEnd,
    CaptureStart(usize),
    CaptureEnd(usize),
    CaptureRef,
    Quantified { body: Vec<Pat>, quant: Quantifier },
}

fn convert_ast(ast: &[AstNode]) -> Vec<Pat> {
    let mut out = Vec::with_capacity(ast.len().saturating_mul(2));
    for node in ast {
        convert_single(node, &mut out);
    }
    out
}

fn convert_single(node: &AstNode, out: &mut Vec<Pat>) {
    match node {
        AstNode::Literal(b) => out.push(Pat::Literal(*b)),
        AstNode::Any => out.push(Pat::Any),
        AstNode::Class(c, negated) => out.push(Pat::Class(*c, *negated)),
        AstNode::Set(charset) => out.push(Pat::Set(charset.clone())),
        AstNode::Balanced(first, second) => out.push(Pat::Balanced(*first, *second)),
        AstNode::Frontier(charset) => out.push(Pat::Frontier(charset.clone())),
        AstNode::AnchorStart => out.push(Pat::AnchorStart),
        AstNode::AnchorEnd => out.push(Pat::AnchorEnd),
        AstNode::CaptureRef(_) => out.push(Pat::CaptureRef),
        AstNode::Capture { index, inner } => {
            let capture_index = index.saturating_sub(1);
            out.push(Pat::CaptureStart(capture_index));
            for child in inner {
                convert_single(child, out);
            }
            out.push(Pat::CaptureEnd(capture_index));
        }
        AstNode::Quantified { item, quantifier } => {
            let mut body = Vec::new();
            convert_single(item, &mut body);
            out.push(Pat::Quantified {
                body,
                quant: *quantifier,
            });
        }
    }
}

#[derive(Clone)]
struct State {
    input: Rc<[u8]>,
    current_pos: usize,
    search_start_pos: usize,
    captures: Vec<Option<Range<usize>>>,
    capture_starts: Vec<Option<usize>>,
    recursion_depth: u32,
}

const MAX_RECURSION_DEPTH: u32 = 500;

impl State {
    fn new(input_slice: &[u8], start_pos: usize) -> Self {
        Self {
            input: Rc::from(input_slice),
            current_pos: start_pos,
            search_start_pos: start_pos,
            captures: vec![None; LUA_MAXCAPTURES],
            capture_starts: vec![None; LUA_MAXCAPTURES],
            recursion_depth: 0,
        }
    }

    #[inline]
    fn current_byte(&self) -> Option<u8> {
        self.input.get(self.current_pos).copied()
    }

    #[inline]
    fn previous_byte(&self) -> Option<u8> {
        if self.current_pos > 0 {
            self.input.get(self.current_pos - 1).copied()
        } else {
            None
        }
    }

    #[inline]
    fn check_class(&self, class_byte: u8, negated: bool) -> bool {
        if let Some(byte) = self.current_byte() {
            let matches = match class_byte {
                b'a' => byte.is_ascii_alphabetic(),
                b'c' => byte.is_ascii_control(),
                b'd' => byte.is_ascii_digit(),
                b'g' => byte.is_ascii_graphic() && byte != b' ',
                b'l' => byte.is_ascii_lowercase(),
                b'p' => byte.is_ascii_punctuation(),
                b's' => byte.is_ascii_whitespace(),
                b'u' => byte.is_ascii_uppercase(),
                b'w' => byte.is_ascii_alphanumeric(),
                b'x' => byte.is_ascii_hexdigit(),
                _ => false,
            };
            matches ^ negated
        } else {
            false
        }
    }
}

/// Fixed `find_first_match`: same contract as
/// `lsonar::engine::find_first_match` but with capture backtracking.
pub(crate) fn find_first_match(
    pattern_ast: &[AstNode],
    input: &[u8],
    start_index: usize,
) -> Result<Option<(Range<usize>, Vec<Option<Range<usize>>>)>, lsonar::Error> {
    let flat = convert_ast(pattern_ast);
    let input_len = input.len();

    for start in start_index..=input_len {
        let initial = State::new(input, start);
        if let Some(final_state) = match_recursive(&flat, initial) {
            let full_match = start..final_state.current_pos;
            return Ok(Some((full_match, final_state.captures)));
        }

        if let Some(AstNode::AnchorStart) = pattern_ast.first() {
            if start == start_index {
                break;
            }
        }
        if pattern_ast.len() == 1 {
            if let Some(AstNode::AnchorEnd) = pattern_ast.first() {
                if start < input_len {
                    continue;
                }
            }
        }
    }

    Ok(None)
}

fn match_recursive(pattern: &[Pat], mut state: State) -> Option<State> {
    if state.recursion_depth > MAX_RECURSION_DEPTH {
        return None;
    }
    state.recursion_depth += 1;

    if pattern.is_empty() {
        return Some(state);
    }

    let node = pattern.first()?;
    let remaining = pattern.get(1..).unwrap_or(&[]);

    match node {
        Pat::Literal(expected) => {
            if state.current_byte() == Some(*expected) {
                state.current_pos += 1;
                match_recursive(remaining, state)
            } else {
                None
            }
        }
        Pat::Any => {
            if state.current_byte().is_some() {
                state.current_pos += 1;
                match_recursive(remaining, state)
            } else {
                None
            }
        }
        Pat::Class(class, negated) => {
            if state.check_class(*class, *negated) {
                state.current_pos += 1;
                match_recursive(remaining, state)
            } else {
                None
            }
        }
        Pat::Set(charset) => {
            if let Some(byte) = state.current_byte() {
                if charset.contains(byte) {
                    state.current_pos += 1;
                    match_recursive(remaining, state)
                } else {
                    None
                }
            } else {
                None
            }
        }
        Pat::AnchorStart => {
            if state.current_pos == state.search_start_pos {
                match_recursive(remaining, state)
            } else {
                None
            }
        }
        Pat::AnchorEnd => {
            if state.current_pos == state.input.len() {
                match_recursive(remaining, state)
            } else {
                None
            }
        }
        Pat::CaptureStart(index) => {
            if let Some(slot) = state.capture_starts.get_mut(*index) {
                *slot = Some(state.current_pos);
            }
            if let Some(slot) = state.captures.get_mut(*index) {
                *slot = None;
            }
            match_recursive(remaining, state)
        }
        Pat::CaptureEnd(index) => {
            let start = state.capture_starts.get(*index)?.clone()?;
            if let Some(slot) = state.captures.get_mut(*index) {
                *slot = Some(start..state.current_pos);
            }
            match_recursive(remaining, state)
        }
        Pat::CaptureRef => None,
        Pat::Balanced(open, close) => {
            if state.current_byte() != Some(*open) {
                return None;
            }
            let mut balance = 1;
            let mut pos = state.current_pos + 1;
            while pos < state.input.len() {
                if state.input[pos] == *close {
                    balance -= 1;
                    if balance == 0 {
                        state.current_pos = pos + 1;
                        return match_recursive(remaining, state);
                    }
                } else if state.input[pos] == *open {
                    balance += 1;
                }
                pos += 1;
            }
            None
        }
        Pat::Frontier(charset) => {
            let prev_in = state.previous_byte().is_some_and(|b| charset.contains(b));
            let next_in = state.current_byte().is_some_and(|b| charset.contains(b));
            if !prev_in && next_in {
                match_recursive(remaining, state)
            } else {
                None
            }
        }
        Pat::Quantified { body, quant } => match quant {
            Quantifier::Star | Quantifier::Plus => {
                let minimum = if *quant == Quantifier::Plus { 1 } else { 0 };
                match_greedy(body, remaining, state, minimum)
            }
            Quantifier::Question => {
                if let Some(after_one) = match_recursive(body, state.clone()) {
                    if let Some(final_state) = match_recursive(remaining, after_one) {
                        return Some(final_state);
                    }
                }
                match_recursive(remaining, state)
            }
            Quantifier::Minus => match_non_greedy(body, remaining, state),
        },
    }
}

fn match_greedy(body: &[Pat], remaining: &[Pat], initial: State, minimum: usize) -> Option<State> {
    let mut current = initial;
    let mut candidates = Vec::new();

    for _ in 0..minimum {
        let next = match_recursive(body, current.clone())?;
        if next.current_pos == current.current_pos {
            return None;
        }
        current = next;
    }
    candidates.push(current.clone());

    loop {
        if let Some(next) = match_recursive(body, current.clone()) {
            if next.current_pos == current.current_pos {
                candidates.push(next);
                break;
            }
            current = next;
            candidates.push(current.clone());
            if candidates.len() > 10_000 {
                break;
            }
        } else {
            break;
        }
    }

    while let Some(candidate) = candidates.pop() {
        if let Some(final_state) = match_recursive(remaining, candidate) {
            return Some(final_state);
        }
    }

    None
}

fn match_non_greedy(body: &[Pat], remaining: &[Pat], initial: State) -> Option<State> {
    let mut current = initial;

    loop {
        if let Some(final_state) = match_recursive(remaining, current.clone()) {
            return Some(final_state);
        }

        let next = match_recursive(body, current.clone())?;
        if next.current_pos == current.current_pos {
            if let Some(final_state) = match_recursive(remaining, next) {
                return Some(final_state);
            }
            return None;
        }
        current = next;
    }
}

fn calculate_start_index(text_len: usize, init: Option<isize>) -> usize {
    match init {
        Some(i) if i > 0 => {
            let adjusted = i - 1;
            let index = adjusted as usize;
            if index >= text_len { text_len } else { index }
        }
        Some(i) if i < 0 => {
            let distance = i.unsigned_abs() as usize;
            if distance > text_len {
                0
            } else {
                text_len.saturating_sub(distance)
            }
        }
        _ => 0,
    }
}

/// Fixed `string.find`: same 1-based contract as `lsonar::find`, byte-oriented.
pub(crate) fn find(
    text: &[u8],
    pattern: &[u8],
    init: Option<isize>,
    plain: bool,
) -> Result<Option<(usize, usize, Vec<Vec<u8>>)>, lsonar::Error> {
    let text_bytes = text;
    let byte_len = text_bytes.len();
    let start_byte_index = calculate_start_index(byte_len, init);

    if plain {
        if pattern.is_empty() {
            return Ok(Some((
                start_byte_index.saturating_add(1),
                start_byte_index,
                vec![],
            )));
        }
        if start_byte_index >= byte_len {
            return Ok(None);
        }
        if let Some(relative) = text_bytes[start_byte_index..]
            .windows(pattern.len())
            .position(|window| window == pattern)
        {
            let zero_start = start_byte_index + relative;
            let zero_end = zero_start + pattern.len();
            return Ok(Some((zero_start.saturating_add(1), zero_end, vec![])));
        }
        return Ok(None);
    }

    let ast = parse_pattern(pattern)?;

    match find_first_match(&ast, text_bytes, start_byte_index)? {
        Some((match_range, capture_ranges)) => {
            let start = match_range.start.saturating_add(1);
            let end = match_range.end;
            let captures: Vec<Vec<u8>> = capture_ranges
                .into_iter()
                .filter_map(|maybe_range| maybe_range.map(|range| text_bytes[range].to_vec()))
                .collect();
            Ok(Some((start, end, captures)))
        }
        None => Ok(None),
    }
}

/// Fixed `string.match`: same contract as `lsonar::match`, byte-oriented.
pub(crate) fn pattern_match(
    text: &[u8],
    pattern: &[u8],
    init: Option<isize>,
) -> Result<Option<Vec<Vec<u8>>>, lsonar::Error> {
    let text_bytes = text;
    let byte_len = text_bytes.len();
    let start_byte_index = calculate_start_index(byte_len, init);

    let ast = parse_pattern(pattern)?;

    match find_first_match(&ast, text_bytes, start_byte_index)? {
        Some((match_range, capture_ranges)) => {
            let captures: Vec<Vec<u8>> = capture_ranges
                .into_iter()
                .filter_map(|maybe_range| maybe_range.map(|range| text_bytes[range].to_vec()))
                .collect();
            if captures.is_empty() {
                let full = text_bytes[match_range.start..match_range.end].to_vec();
                Ok(Some(vec![full]))
            } else {
                Ok(Some(captures))
            }
        }
        None => Ok(None),
    }
}
