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
use super::pattern_engine;

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
    fn new(text: &[u8], pattern: &[u8], init: Option<i64>) -> Result<Self, lsonar::Error> {
        let is_empty_pattern = pattern.is_empty();
        let pattern_ast = if is_empty_pattern {
            Vec::new()
        } else {
            pattern_engine::parse_pattern(pattern)?
        };

        let bytes = text.to_vec();
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

    fn next(&mut self) -> Option<Result<Vec<Vec<u8>>, lsonar::Error>> {
        if self.current_pos > self.bytes.len() {
            return None;
        }

        if self.is_empty_pattern {
            let result = Some(Ok(vec![Vec::new()]));
            self.current_pos += 1;
            return result;
        }

        match pattern_engine::find_first_match(&self.pattern_ast, &self.bytes, self.current_pos) {
            Ok(Some((match_range, captures))) => {
                if match_range.start == match_range.end {
                    self.current_pos = match_range.end + 1;
                    if self.current_pos > self.bytes.len() {
                        return None;
                    }
                } else {
                    self.current_pos = match_range.end;
                }

                let result: Vec<Vec<u8>> = if captures.iter().any(|c| c.is_some()) {
                    captures
                        .into_iter()
                        .filter_map(|maybe_range| {
                            maybe_range.map(|range| self.bytes[range].to_vec())
                        })
                        .collect()
                } else {
                    vec![self.bytes[match_range.start..match_range.end].to_vec()]
                };

                Some(Ok(result))
            }
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }

    /// Upper bound on the number of candidate start positions the search loop
    /// in `pattern_engine::find_first_match` may try for this call.
    fn attempt_bound(&self) -> usize {
        self.bytes
            .len()
            .saturating_sub(self.current_pos)
            .saturating_add(1)
    }

    /// Number of bytes remaining in the scan window for the next search.
    fn scan_window(&self) -> usize {
        self.bytes.len().saturating_sub(self.current_pos)
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

            // Lua strings are arbitrary bytes (Lua 5.1 semantics); never
            // validate them as UTF-8 here. Patterns may embed isolated
            // non-UTF8 bytes such as `\128` (0x80).
            let s_bytes = s.as_bytes();
            let pattern_bytes = pattern.as_bytes();

            if let Some(i) = init {
                let len = s_bytes.len() as i64;
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
                Some(i) if i > 0 => (i - 1).min(s_bytes.len() as i64) as usize,
                Some(i) if i < 0 => {
                    let abs = i.unsigned_abs() as usize;
                    s_bytes.len().saturating_sub(abs)
                }
                _ => 0,
            };
            let attempts = s_bytes.len().saturating_sub(start_pos).saturating_add(1);
            exec.fuel().consume(
                sandbox::scanned_cost(s_bytes.len())
                    .saturating_add(sandbox::count_pattern_attempts(attempts)),
            );

            let Some((start, end, captures)) =
                pattern_engine::find(s_bytes, pattern_bytes, init.map(|i| i as isize), plain)
                    .map_err(|err| {
                        let err = err.to_string();
                        err.into_value(ctx)
                    })?
            else {
                stack.replace(ctx, Value::Nil);
                return Ok(CallbackReturn::Return);
            };

            stack.clear();
            stack.into_back(ctx, start as i64);
            stack.into_back(ctx, end as i64);

            for capture in captures {
                stack.into_back(ctx, ctx.intern(&capture));
            }

            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "match",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let (s, pattern, init) = stack.consume::<(String, String, Option<i64>)>(ctx)?;

            let s_bytes = s.as_bytes();
            let pattern_bytes = pattern.as_bytes();

            if let Some(i) = init {
                let len = s_bytes.len() as i64;
                if i > len + 1 {
                    stack.replace(ctx, Value::Nil);
                    return Ok(CallbackReturn::Return);
                }
            }

            let start_pos = match init {
                Some(i) if i > 0 => (i - 1).min(s_bytes.len() as i64) as usize,
                Some(i) if i < 0 => {
                    let abs = i.unsigned_abs() as usize;
                    s_bytes.len().saturating_sub(abs)
                }
                _ => 0,
            };
            let attempts = s_bytes.len().saturating_sub(start_pos).saturating_add(1);
            exec.fuel().consume(
                sandbox::scanned_cost(s_bytes.len())
                    .saturating_add(sandbox::count_pattern_attempts(attempts)),
            );

            let Some(captures) =
                pattern_engine::pattern_match(s_bytes, pattern_bytes, init.map(|i| i as isize))
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
                stack.into_back(ctx, ctx.intern(&capture));
            }

            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "gmatch",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, pattern, init) = stack.consume::<(String, String, Option<i64>)>(ctx)?;

            let state = GMatchState(Rc::new(Mutex::new(
                GMatchInner::new(s.as_bytes(), pattern.as_bytes(), init).map_err(|err| {
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
                    // remaining scan window, its attempt bound, and the produced
                    // captures so repeated `gmatch` calls are accounted
                    // proportionally to the documented model.
                    let scan_window = inner.scan_window();
                    let attempts = inner.attempt_bound();
                    exec.fuel().consume(
                        sandbox::scanned_cost(scan_window)
                            .saturating_add(sandbox::count_pattern_attempts(attempts)),
                    );
                    match inner.next() {
                        Some(Ok(captures)) => {
                            let produced: usize = captures.iter().map(|c| c.len()).sum();
                            exec.fuel().consume(sandbox::output_cost(produced));
                            for capture in captures {
                                stack.into_back(ctx, ctx.intern(&capture));
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

            GsubSequence::create(ctx, s.as_bytes(), pattern.as_bytes(), repl, n)
        }),
    );
}

/// Owned replacement mode for the resumable `gsub`.
#[derive(Collect)]
#[collect(no_drop)]
enum ReplMode<'gc> {
    String(#[collect(require_static)] Vec<u8>),
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
/// Note: a single `pattern_engine::find_first_match` call is not preemptible
/// below the engine's `MAX_RECURSION_DEPTH` bound and the pattern length; the
/// sequence charges an attempt bound of `remaining_window + 1` per call so the
/// cost is accounted deterministically and the loop yields between matches.
///
/// Byte orientation (bitty-terminal/bitty#1826): text, pattern, replacement,
/// and result are all raw bytes. Lua strings are not necessarily UTF-8.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct GsubSequence<'gc> {
    #[collect(require_static)]
    text_bytes: Vec<u8>,
    #[collect(require_static)]
    pattern_ast: Vec<lsonar::AstNode>,
    repl: ReplMode<'gc>,
    max_replacements: usize,
    last_pos: usize,
    replacements: usize,
    #[collect(require_static)]
    result: Vec<u8>,
}

impl<'gc> GsubSequence<'gc> {
    /// Builds the sequence, returning a Lua error for an invalid replacement
    /// argument or a malformed pattern at call time (matching the one-shot
    /// implementation).
    pub(crate) fn create(
        ctx: Context<'gc>,
        text: &[u8],
        pattern: &[u8],
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
            pattern_engine::parse_pattern(pattern)
                .map_err(|err| Error::from_value(err.to_string().into_value(ctx)))?
        };

        let repl = match repl {
            Value::String(s) => ReplMode::String(s.as_bytes().to_vec()),
            Value::Integer(_) | Value::Number(_) => {
                ReplMode::String(repl.display().to_string().into_bytes())
            }
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
                text_bytes: text.to_vec(),
                pattern_ast,
                repl,
                max_replacements,
                last_pos: 0,
                replacements: 0,
                result: Vec::new(),
            },
        )))
    }
}

/// Appends `s` to `result`, enforcing the checked 16 MiB output ceiling and
/// charging one Fuel per appended output byte. `result` is a native (untracked)
/// buffer, so the projected size is also charged against the hard memory quota
/// before it grows; otherwise a hostile `gsub` expansion would build a
/// multi-megabyte buffer that only the GC boundary would ever observe.
fn push_checked<'gc>(
    result: &mut Vec<u8>,
    s: &[u8],
    fuel: &mut crate::Fuel,
    ctx: Context<'gc>,
) -> Result<(), Error<'gc>> {
    if sandbox::checked_output_growth(result.len(), s.len()).is_none() {
        return Err(Error::from_value(
            "resulting string too large".into_value(ctx),
        ));
    }
    ctx.check_memory(result.len().saturating_add(s.len()))?;
    fuel.consume(sandbox::output_cost(s.len()));
    result.extend_from_slice(s);
    Ok(())
}

/// Appends `chunk` to `out`, enforcing the checked 16 MiB ceiling on the
/// intermediate replacement buffer. This bounds a `%0`-heavy replacement whose
/// expansion is directives times match length before it is copied into the
/// result, so it can never grow an unchecked buffer that exceeds the ceiling.
fn push_replacement(out: &mut Vec<u8>, chunk: &[u8]) -> Result<(), StdString> {
    if sandbox::checked_output_growth(out.len(), chunk.len()).is_none() {
        return Err("resulting string too large".to_string());
    }
    out.extend_from_slice(chunk);
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

        // Destructure so the immutable borrow of `text_bytes`/`pattern_ast`
        // and the mutable borrow of `result` are disjoint (no per-iteration
        // clone).
        let GsubSequence {
            text_bytes,
            pattern_ast,
            repl,
            max_replacements,
            last_pos,
            replacements,
            result,
        } = &mut *seq;

        if *max_replacements == 0 {
            push_checked(result, text_bytes.as_slice(), fuel, ctx)?;
            let interned = ctx.intern(result.as_slice());
            stack.clear();
            stack.into_back(ctx, interned);
            stack.into_back(ctx, 0i64);
            return Ok(SequencePoll::Return);
        }

        while *replacements < *max_replacements {
            let attempts = text_bytes.len().saturating_sub(*last_pos).saturating_add(1);
            fuel.consume(sandbox::count_pattern_attempts(attempts));

            let match_opt = pattern_engine::find_first_match(pattern_ast, text_bytes, *last_pos)
                .map_err(|err| Error::from_value(err.to_string().into_value(ctx)))?;

            let Some((match_range, captures)) = match_opt else {
                break;
            };

            push_checked(result, &text_bytes[*last_pos..match_range.start], fuel, ctx)?;

            let full_match: &[u8] = &text_bytes[match_range.start..match_range.end];
            let captures_owned: Vec<Option<Vec<u8>>> = captures
                .iter()
                .map(|maybe_range| {
                    maybe_range
                        .as_ref()
                        .map(|range| text_bytes[range.start..range.end].to_vec())
                })
                .collect();
            let captures_ref: Vec<&[u8]> = captures_owned
                .iter()
                .filter_map(|opt| opt.as_deref())
                .collect();

            let replacement = match repl {
                ReplMode::String(repl_bytes) => {
                    process_replacement_bytes(repl_bytes, full_match, &captures_ref)
                        .map_err(|err| Error::from_value(err.into_value(ctx)))?
                }
                ReplMode::Table(table) => {
                    let key_bytes: &[u8] = if !captures_ref.is_empty() {
                        captures_ref[0]
                    } else {
                        full_match
                    };
                    let key = ctx.intern(key_bytes);
                    let val = table.get_value(ctx, key);
                    match val {
                        Value::String(s) => s.as_bytes().to_vec(),
                        Value::Integer(i) => i.to_string().into_bytes(),
                        Value::Number(n) => n.to_string().into_bytes(),
                        Value::Nil | Value::Boolean(false) => full_match.to_vec(),
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
            push_checked(result, &replacement, fuel, ctx)?;

            *last_pos = match_range.end;
            *replacements += 1;

            if match_range.start == match_range.end {
                if *last_pos >= text_bytes.len() {
                    break;
                }
                // Lua patterns are byte-oriented: advance one byte past an
                // empty match (not one UTF-8 char).
                push_checked(result, &text_bytes[*last_pos..*last_pos + 1], fuel, ctx)?;
                *last_pos += 1;
            }

            if !fuel.should_continue() {
                return Ok(SequencePoll::Pending);
            }
        }

        if *last_pos < text_bytes.len() {
            push_checked(result, &text_bytes[*last_pos..], fuel, ctx)?;
        }

        let interned = ctx.intern(result.as_slice());
        stack.clear();
        stack.into_back(ctx, interned);
        stack.into_back(ctx, *replacements as i64);
        Ok(SequencePoll::Return)
    }
}

fn process_replacement_bytes(
    repl: &[u8],
    full_match: &[u8],
    captures: &[&[u8]],
) -> Result<Vec<u8>, StdString> {
    let bytes = repl;
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
                    push_replacement(&mut out_bytes, b"%")?;
                }
                b'0' => {
                    push_replacement(&mut out_bytes, full_match)?;
                }
                d @ b'1'..=b'9' => {
                    let idx = (d - b'1') as usize;
                    if captures.is_empty() {
                        if d == b'1' {
                            push_replacement(&mut out_bytes, full_match)?;
                        } else {
                            return Err(format!("invalid capture index %{}", d as char));
                        }
                    } else if idx < captures.len() {
                        push_replacement(&mut out_bytes, captures[idx])?;
                    } else {
                        return Err(format!("invalid capture index %{}", d as char));
                    }
                }
                _ => return Err("invalid use of '%' in replacement string".to_string()),
            }
            i += 1;
        } else {
            push_replacement(&mut out_bytes, &bytes[i..i + 1])?;
            i += 1;
        }
    }
    Ok(out_bytes)
}
