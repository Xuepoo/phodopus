//! Fixed Lua pattern engine for quantified-inner captures (issue #56).
//!
//! Upstream `lsonar 0.2.4` parses `(.-)` as `Capture { inner: [Quantified] }`
//! but its `Capture` arm matches `inner` once with an empty continuation and
//! never retries longer expansions when the outer continuation (`%s*$`)
//! fails. The idiomatic trim `^%s*(.-)%s*$` therefore returns `nil` instead
//! of the trimmed string.
//!
//! This module reuses `lsonar`'s parser and AST but matches over a desugared
//! form where every capture becomes explicit `CaptureStart` / `CaptureEnd`
//! markers. Quantifiers inside a capture then see the capture-end marker plus
//! the outer continuation as their `remaining`, so the existing
//! greedy/non-greedy backtracking naturally expands the capture until the
//! outer pattern matches (PUC Lua 5.1 semantics). `Quantified` bodies are
//! sequences (usually one element) so a quantified capture such as `(a)+`
//! repeats its full marker span.

use std::ops::Range;
use std::rc::Rc;

use lsonar::{AstNode, LUA_MAXCAPTURES, Quantifier};

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

/// Fixed `string.find`: same 1-based contract as `lsonar::find`.
pub(crate) fn find(
    text: &str,
    pattern: &str,
    init: Option<isize>,
    plain: bool,
) -> Result<Option<(usize, usize, Vec<String>)>, lsonar::Error> {
    let text_bytes = text.as_bytes();
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
            .position(|window| window == pattern.as_bytes())
        {
            let zero_start = start_byte_index + relative;
            let zero_end = zero_start + pattern.len();
            return Ok(Some((zero_start.saturating_add(1), zero_end, vec![])));
        }
        return Ok(None);
    }

    let mut parser = lsonar::Parser::new(pattern)?;
    let ast = parser.parse()?;

    match find_first_match(&ast, text_bytes, start_byte_index)? {
        Some((match_range, capture_ranges)) => {
            let start = match_range.start.saturating_add(1);
            let end = match_range.end;
            let captures: Vec<String> = capture_ranges
                .into_iter()
                .filter_map(|maybe_range| {
                    maybe_range
                        .map(|range| String::from_utf8_lossy(&text_bytes[range]).into_owned())
                })
                .collect();
            Ok(Some((start, end, captures)))
        }
        None => Ok(None),
    }
}

/// Fixed `string.match`: same contract as `lsonar::match`.
pub(crate) fn pattern_match(
    text: &str,
    pattern: &str,
    init: Option<isize>,
) -> Result<Option<Vec<String>>, lsonar::Error> {
    let text_bytes = text.as_bytes();
    let byte_len = text_bytes.len();
    let start_byte_index = calculate_start_index(byte_len, init);

    let mut parser = lsonar::Parser::new(pattern)?;
    let ast = parser.parse()?;

    match find_first_match(&ast, text_bytes, start_byte_index)? {
        Some((match_range, capture_ranges)) => {
            let captures: Vec<String> = capture_ranges
                .into_iter()
                .filter_map(|maybe_range| {
                    maybe_range
                        .map(|range| String::from_utf8_lossy(&text_bytes[range]).into_owned())
                })
                .collect();
            if captures.is_empty() {
                let full = String::from_utf8_lossy(&text_bytes[match_range.start..match_range.end])
                    .into_owned();
                Ok(Some(vec![full]))
            } else {
                Ok(Some(captures))
            }
        }
        None => Ok(None),
    }
}
