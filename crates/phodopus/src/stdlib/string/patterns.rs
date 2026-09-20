use std::rc::Rc;
use std::string::String as StdString;
use std::sync::Mutex;

use gc_arena::Collect;

use crate::{Callback, CallbackReturn, Context, Error, IntoValue, String, Table, Value};

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
}

pub fn load_patterns<'gc>(ctx: Context<'gc>, string: &Table<'gc>) {
    string.set_field(
        ctx,
        "find",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
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
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
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

            let gmatch_cb = Callback::from_fn_with(&ctx, state, |state, ctx, _, mut stack| {
                stack.clear();
                let mut inner = state.0.lock().map_err(|err| {
                    let err = err.to_string();
                    err.into_value(ctx)
                })?;
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

            let (res, count) = gsub_impl(ctx, s_str, pattern_str, repl, n)?;

            stack.clear();
            stack.into_back(ctx, res);
            stack.into_back(ctx, count);

            Ok(CallbackReturn::Return)
        }),
    );
}

fn gsub_impl<'gc>(
    ctx: Context<'gc>,
    text: &str,
    pattern: &str,
    repl: Value<'gc>,
    n: Option<i64>,
) -> Result<(StdString, i64), Error<'gc>> {
    let max_replacements = match n {
        Some(n) if n <= 0 => 0,
        Some(n) => n as usize,
        None => usize::MAX,
    };

    if max_replacements == 0 {
        return Ok((text.to_owned(), 0));
    }

    let is_empty_pattern = pattern.is_empty();
    let pattern_ast = if is_empty_pattern {
        Vec::new()
    } else {
        let mut parser = lsonar::Parser::new(pattern).map_err(|err| {
            let err = err.to_string();
            err.into_value(ctx)
        })?;
        parser.parse().map_err(|err| {
            let err = err.to_string();
            err.into_value(ctx)
        })?
    };

    enum ReplMode<'a, 'gc> {
        String(&'a str),
        Table(Table<'gc>),
    }

    let mode = match repl {
        Value::String(s) => ReplMode::String(s.to_str()?),
        Value::Integer(_) | Value::Number(_) => {
            let s = repl.into_string(ctx).ok_or_else(|| {
                Error::from_value("failed to convert number to string".into_value(ctx))
            })?;
            ReplMode::String(s.to_str()?)
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

    let text_bytes = text.as_bytes();
    let byte_len = text_bytes.len();
    let mut result = StdString::new();
    let mut last_pos = 0;
    let mut replacements = 0;

    while replacements < max_replacements {
        let match_opt = lsonar::engine::find_first_match(&pattern_ast, text_bytes, last_pos)
            .map_err(|err| {
                let err = err.to_string();
                err.into_value(ctx)
            })?;

        match match_opt {
            Some((match_range, captures)) => {
                result.push_str(&text[last_pos..match_range.start]);

                let full_match = &text[match_range.start..match_range.end];
                let captures_str: Vec<&str> = captures
                    .iter()
                    .filter_map(|maybe_range| {
                        maybe_range
                            .as_ref()
                            .map(|range| &text[range.start..range.end])
                    })
                    .collect();

                match mode {
                    ReplMode::String(repl_str) => {
                        let replacement =
                            process_replacement_string(repl_str, full_match, &captures_str)
                                .map_err(|err| err.into_value(ctx))?;
                        result.push_str(&replacement);
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
                            Value::String(s) => {
                                result.push_str(s.to_str()?);
                            }
                            Value::Integer(i) => {
                                result.push_str(&i.to_string());
                            }
                            Value::Number(n) => {
                                result.push_str(&n.to_string());
                            }
                            Value::Nil | Value::Boolean(false) => {
                                result.push_str(full_match);
                            }
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
                }

                last_pos = match_range.end;
                replacements += 1;

                if match_range.start == match_range.end {
                    if last_pos >= byte_len {
                        break;
                    }
                    let advance_by = text[last_pos..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(1);
                    result.push_str(&text[last_pos..last_pos + advance_by]);
                    last_pos += advance_by;
                }
            }
            None => break,
        }
    }

    if last_pos < byte_len {
        result.push_str(&text[last_pos..]);
    }

    Ok((result, replacements as i64))
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
