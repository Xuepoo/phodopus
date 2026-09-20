use std::pin::Pin;
use std::rc::Rc;
use std::string::String as StdString;
use std::sync::Mutex;

use gc_arena::Collect;

use crate::{
    BoxSequence, Callback, CallbackReturn, Context, Error, Execution, IntoValue, Sequence,
    SequencePoll, Stack, String, Table, Value,
};

use super::super::sandbox;

#[derive(Collect, Clone)]
#[collect(require_static)]
struct GMatchState(Rc<Mutex<GMatchInner>>);

struct GMatchInner {
    bytes: Vec<u8>,
    pattern_ast: Vec<lsonar::AstNode>,
    current_pos: usize,
    is_empty_pattern: bool,
}

impl GMatchInner {
    fn new(text: &str, pattern: &str, init: Option<i64>) -> Result<Self, lsonar::Error> {
        let is_empty_pattern = pattern.is_empty();
        let pattern_ast = if is_empty_pattern {
            Vec::new()
        } else {
            let mut parser = lsonar::Parser::new(pattern)?;
            parser.parse()?
        };

        let bytes = text.as_bytes().to_vec();
        let text_len = bytes.len();
        let current_pos = match init {
            Some(i) if i > 0 => {
                let idx = (i - 1) as usize;
                if idx > text_len { text_len + 1 } else { idx }
            }
            Some(i) if i < 0 => {
                let abs_i = (-i) as usize;
                if abs_i > text_len {
                    0
                } else {
                    text_len.saturating_sub(abs_i)
                }
            }
            _ => 0,
        };

        Ok(Self {
            bytes,
            pattern_ast,
            current_pos,
            is_empty_pattern,
        })
    }

    fn next(&mut self) -> Option<Result<Vec<StdString>, lsonar::Error>> {
        if self.current_pos > self.bytes.len() {
            return None;
        }

        if self.is_empty_pattern {
            let result = Some(Ok(vec![StdString::new()]));
            self.current_pos += 1;
            return result;
        }

        match lsonar::engine::find_first_match(&self.pattern_ast, &self.bytes, self.current_pos) {
            Ok(Some((match_range, captures))) => {
                if match_range.start == match_range.end {
                    self.current_pos = match_range.end + 1;
                    if self.current_pos > self.bytes.len() {
                        return None;
                    }
                } else {
                    self.current_pos = match_range.end;
                }

                let result: Vec<StdString> = if captures.iter().any(|c| c.is_some()) {
                    captures
                        .into_iter()
                        .filter_map(|maybe_range| {
                            maybe_range.map(|range| {
                                StdString::from_utf8_lossy(&self.bytes[range]).into_owned()
                            })
                        })
                        .collect()
                } else {
                    vec![
                        StdString::from_utf8_lossy(&self.bytes[match_range.start..match_range.end])
                            .into_owned(),
                    ]
                };

                Some(Ok(result))
            }
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }

    /// Upper bound on the number of candidate start positions the search loop
    /// in `lsonar::engine::find_first_match` may try for this call.
    fn attempt_bound(&self) -> usize {
        self.bytes
            .len()
            .saturating_sub(self.current_pos)
            .saturating_add(1)
    }
}

pub fn load_patterns<'gc>(ctx: Context<'gc>, string: &Table<'gc>) {
    string.set_field(
        ctx,
        "find",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let (s, pattern, init, plain) =
                stack.consume::<(String, String, Option<i64>, Option<bool>)>(ctx)?;
            let plain = plain.unwrap_or(false);

            let s_str = s.to_str()?;
            let pattern_str = pattern.to_str()?;

            if let Some(i) = init {
                let len = s_str.len() as i64;
                if i > len + 1 {
                    stack.replace(ctx, Value::Nil);
                    return Ok(CallbackReturn::Return);
                }
            }

            // `string.find` performs at most one unanchored search, whose inner
            // (non-preemptible) call is bounded by the input length. Charge the
            // scanned bytes plus the pattern-attempt bound so the cost is
            // recorded even though the call itself cannot yield.
            let start_pos = match init {
                Some(i) if i > 0 => (i - 1).min(s_str.len() as i64) as usize,
                Some(i) if i < 0 => {
                    let abs = i.unsigned_abs() as usize;
                    s_str.len().saturating_sub(abs)
                }
                _ => 0,
            };
            let attempts = s_str.len().saturating_sub(start_pos).saturating_add(1);
            exec.fuel().consume(
                sandbox::scanned_cost(s_str.len())
                    .saturating_add(sandbox::count_pattern_attempts(attempts)),
            );

            let Some((start, end, captures)) =
                lsonar::find(s_str, pattern_str, init.map(|i| i as isize), plain).map_err(
                    |err| {
                        let err = err.to_string();
                        err.into_value(ctx)
                    },
                )?
            else {
                stack.replace(ctx, Value::Nil);
                return Ok(CallbackReturn::Return);
            };

            stack.clear();
            stack.into_back(ctx, start as i64);
            stack.into_back(ctx, end as i64);

            for capture in captures {
                stack.into_back(ctx, capture);
            }

            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "match",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let (s, pattern, init) = stack.consume::<(String, String, Option<i64>)>(ctx)?;

            let s_str = s.to_str()?;
            let pattern_str = pattern.to_str()?;

            if let Some(i) = init {
                let len = s_str.len() as i64;
                if i > len + 1 {
                    stack.replace(ctx, Value::Nil);
                    return Ok(CallbackReturn::Return);
                }
            }

            let start_pos = match init {
                Some(i) if i > 0 => (i - 1).min(s_str.len() as i64) as usize,
                Some(i) if i < 0 => {
                    let abs = i.unsigned_abs() as usize;
                    s_str.len().saturating_sub(abs)
                }
                _ => 0,
            };
            let attempts = s_str.len().saturating_sub(start_pos).saturating_add(1);
            exec.fuel().consume(
                sandbox::scanned_cost(s_str.len())
                    .saturating_add(sandbox::count_pattern_attempts(attempts)),
            );

            let Some(captures) = lsonar::r#match(s_str, pattern_str, init.map(|i| i as isize))
                .map_err(|err| {
                    let err = err.to_string();
                    err.into_value(ctx)
                })?
            else {
                stack.replace(ctx, Value::Nil);
                return Ok(CallbackReturn::Return);
            };

            stack.clear();
            for capture in captures {
                stack.into_back(ctx, capture);
            }

            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "gmatch",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, pattern, init) = stack.consume::<(String, String, Option<i64>)>(ctx)?;

            let s_str = s.to_str()?;
            let pattern_str = pattern.to_str()?;

            let state = GMatchState(Rc::new(Mutex::new(
                GMatchInner::new(s_str, pattern_str, init).map_err(|err| {
                    let err = err.to_string();
                    err.into_value(ctx)
                })?,
            )));

            let gmatch_cb =
                Callback::from_fn_with(&ctx, state, |state, ctx, mut exec, mut stack| {
                    stack.clear();
                    let mut inner = state.0.lock().map_err(|err| {
                        let err = err.to_string();
                        err.into_value(ctx)
                    })?;
                    // Each iteration performs one unanchored search. Charge the
                    // remaining scan window and its attempt bound so repeated
                    // `gmatch` calls are accounted proportionally.
                    let attempts = inner.attempt_bound();
                    exec.fuel()
                        .consume(sandbox::count_pattern_attempts(attempts));
                    match inner.next() {
                        Some(Ok(captures)) => {
                            for capture in captures {
                                stack.into_back(ctx, capture);
                            }
                            Ok(CallbackReturn::Return)
                        }
                        Some(Err(err)) => {
                            let err = err.to_string();
                            Err(err.into_value(ctx).into())
                        }
                        None => Ok(CallbackReturn::Return),
                    }
                });

            stack.replace(ctx, gmatch_cb);
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "gsub",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, pattern, repl, n) =
                stack.consume::<(String, String, Value, Option<i64>)>(ctx)?;

            let s_str = s.to_str()?;
            let pattern_str = pattern.to_str()?;

            GsubSequence::create(ctx, s_str, pattern_str, repl, n)
        }),
    );
}

/// Owned replacement mode for the resumable `gsub`.
#[derive(Collect)]
#[collect(no_drop)]
enum ReplMode<'gc> {
    String(#[collect(require_static)] StdString),
    Table(Table<'gc>),
}

/// A resumable implementation of `string.gsub`.
///
/// Each `poll` performs as many search+replace iterations as the remaining Fuel
/// allows and then returns [`SequencePoll::Pending`] with the output buffer,
/// cursor, replacement count, and replacement mode preserved. Output growth is
/// checked against `MAX_STDLIB_STRING_BYTES` before every append, independent of
/// any global heap quota.
///
/// Note: a single `lsonar::engine::find_first_match` call is not preemptible
/// below the engine's `MAX_RECURSION_DEPTH` bound and the pattern length; the
/// sequence charges an attempt bound of `remaining_window + 1` per call so the
/// cost is accounted deterministically and the loop yields between matches.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct GsubSequence<'gc> {
    #[collect(require_static)]
    text: StdString,
    #[collect(require_static)]
    text_bytes: Vec<u8>,
    #[collect(require_static)]
    pattern_ast: Vec<lsonar::AstNode>,
    repl: ReplMode<'gc>,
    max_replacements: usize,
    last_pos: usize,
    replacements: usize,
    #[collect(require_static)]
    result: StdString,
}

impl<'gc> GsubSequence<'gc> {
    /// Builds the sequence, returning a Lua error for an invalid replacement
    /// argument or a malformed pattern at call time (matching the one-shot
    /// implementation).
    pub(crate) fn create(
        ctx: Context<'gc>,
        text: &str,
        pattern: &str,
        repl: Value<'gc>,
        n: Option<i64>,
    ) -> Result<CallbackReturn<'gc>, Error<'gc>> {
        let max_replacements = match n {
            Some(n) if n <= 0 => 0,
            Some(n) => n as usize,
            None => usize::MAX,
        };

        let pattern_ast = if pattern.is_empty() {
            Vec::new()
        } else {
            let mut parser = lsonar::Parser::new(pattern)
                .map_err(|err| Error::from_value(err.to_string().into_value(ctx)))?;
            parser
                .parse()
                .map_err(|err| Error::from_value(err.to_string().into_value(ctx)))?
        };

        let repl = match repl {
            Value::String(s) => ReplMode::String(s.display_lossy().to_string()),
            Value::Integer(_) | Value::Number(_) => ReplMode::String(repl.display().to_string()),
            Value::Table(t) => ReplMode::Table(t),
            Value::Function(_) => {
                return Err("function replacement currently unsupported in gsub"
                    .into_value(ctx)
                    .into());
            }
            _ => {
                return Err(format!(
                    "bad argument #3 to 'gsub' (string/function/table expected, got {})",
                    repl.type_name()
                )
                .into_value(ctx)
                .into());
            }
        };

        Ok(CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            GsubSequence {
                text: text.to_string(),
                text_bytes: text.as_bytes().to_vec(),
                pattern_ast,
                repl,
                max_replacements,
                last_pos: 0,
                replacements: 0,
                result: StdString::new(),
            },
        )))
    }
}

/// Appends `s` to `result`, enforcing the checked 16 MiB output ceiling.
fn push_checked<'gc>(result: &mut StdString, s: &str, ctx: Context<'gc>) -> Result<(), Error<'gc>> {
    if sandbox::checked_output_growth(result.len(), s.len()).is_none() {
        return Err(Error::from_value(
            "resulting string too large".into_value(ctx),
        ));
    }
    result.push_str(s);
    Ok(())
}

impl<'gc> Sequence<'gc> for GsubSequence<'gc> {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let seq = self.as_mut().get_mut();
        let fuel = exec.fuel();

        // Destructure so the immutable borrow of `text`/`pattern_ast` and the
        // mutable borrow of `result` are disjoint (no per-iteration clone).
        let GsubSequence {
            text,
            text_bytes,
            pattern_ast,
            repl,
            max_replacements,
            last_pos,
            replacements,
            result,
        } = &mut *seq;

        if *max_replacements == 0 {
            push_checked(result, text, ctx)?;
            let interned = ctx.intern(result.as_bytes());
            stack.clear();
            stack.into_back(ctx, interned);
            stack.into_back(ctx, 0i64);
            return Ok(SequencePoll::Return);
        }

        while *replacements < *max_replacements {
            let attempts = text_bytes.len().saturating_sub(*last_pos).saturating_add(1);
            fuel.consume(sandbox::count_pattern_attempts(attempts));

            let match_opt = lsonar::engine::find_first_match(pattern_ast, text_bytes, *last_pos)
                .map_err(|err| Error::from_value(err.to_string().into_value(ctx)))?;

            let Some((match_range, captures)) = match_opt else {
                break;
            };

            push_checked(result, &text[*last_pos..match_range.start], ctx)?;

            let full_match = &text[match_range.start..match_range.end];
            let captures_str: Vec<&str> = captures
                .iter()
                .filter_map(|maybe_range| {
                    maybe_range
                        .as_ref()
                        .map(|range| &text[range.start..range.end])
                })
                .collect();

            let replacement = match repl {
                ReplMode::String(repl_str) => {
                    process_replacement_string(repl_str, full_match, &captures_str)
                        .map_err(|err| Error::from_value(err.into_value(ctx)))?
                }
                ReplMode::Table(table) => {
                    let key_str = if !captures_str.is_empty() {
                        captures_str[0]
                    } else {
                        full_match
                    };
                    let key = ctx.intern(key_str.as_bytes());
                    let val = table.get_value(ctx, key);
                    match val {
                        Value::String(s) => s.display_lossy().to_string(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::Nil | Value::Boolean(false) => full_match.to_string(),
                        _ => {
                            return Err(format!(
                                "invalid replacement value (a {})",
                                val.type_name()
                            )
                            .into_value(ctx)
                            .into());
                        }
                    }
                }
            };
            push_checked(result, &replacement, ctx)?;

            *last_pos = match_range.end;
            *replacements += 1;

            if match_range.start == match_range.end {
                if *last_pos >= text_bytes.len() {
                    break;
                }
                let advance_by = text[*last_pos..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                push_checked(result, &text[*last_pos..*last_pos + advance_by], ctx)?;
                *last_pos += advance_by;
            }

            if !fuel.should_continue() {
                return Ok(SequencePoll::Pending);
            }
        }

        if *last_pos < text_bytes.len() {
            push_checked(result, &text[*last_pos..], ctx)?;
        }

        let interned = ctx.intern(result.as_bytes());
        stack.clear();
        stack.into_back(ctx, interned);
        stack.into_back(ctx, *replacements as i64);
        Ok(SequencePoll::Return)
    }
}

fn process_replacement_string(
    repl: &str,
    full_match: &str,
    captures: &[&str],
) -> Result<StdString, StdString> {
    let bytes = repl.as_bytes();
    let mut out_bytes = Vec::with_capacity(repl.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            i += 1;
            if i >= bytes.len() {
                return Err("invalid use of '%' in replacement string".to_string());
            }
            match bytes[i] {
                b'%' => {
                    out_bytes.push(b'%');
                }
                b'0' => {
                    out_bytes.extend_from_slice(full_match.as_bytes());
                }
                d @ b'1'..=b'9' => {
                    let idx = (d - b'1') as usize;
                    if captures.is_empty() {
                        if d == b'1' {
                            out_bytes.extend_from_slice(full_match.as_bytes());
                        } else {
                            return Err(format!("invalid capture index %{}", d as char));
                        }
                    } else if idx < captures.len() {
                        out_bytes.extend_from_slice(captures[idx].as_bytes());
                    } else {
                        return Err(format!("invalid capture index %{}", d as char));
                    }
                }
                _ => return Err("invalid use of '%' in replacement string".to_string()),
            }
            i += 1;
        } else {
            out_bytes.push(bytes[i]);
            i += 1;
        }
    }
    StdString::from_utf8(out_bytes).map_err(|e| e.to_string())
}
