use crate::{Callback, CallbackReturn, Context, IntoValue, String, Table, Value};

fn convert_index(i: i64, len: usize) -> Option<usize> {
    let val = match i {
        0 => 0,
        v @ 1.. => v - 1,
        v @ ..=-1 => (len as i64 + v).max(0),
    };
    usize::try_from(val).ok()
}

fn convert_index_end(i: i64, len: usize) -> Option<usize> {
    let val = match i {
        v @ 0.. => v,
        v @ ..=-1 => (len as i64 + v + 1).max(0),
    };
    usize::try_from(val).ok()
}

fn decode_utf8(bytes: &[u8]) -> Option<(char, usize)> {
    if bytes.is_empty() {
        return None;
    }
    let b0 = bytes[0];
    let len = match b0 {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    if bytes.len() < len {
        return None;
    }
    let s = std::str::from_utf8(&bytes[..len]).ok()?;
    let mut chars = s.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some((c, len))
}

pub fn load_utf8<'gc>(ctx: Context<'gc>) {
    let utf8 = Table::new(&ctx);

    utf8.set_field(
        ctx,
        "char",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let mut bytes = Vec::with_capacity(stack.len() * 4);
            for (idx, val) in stack.into_iter().enumerate() {
                let code = match val.to_integer() {
                    Some(c) => c,
                    None => {
                        return Err(format!(
                            "bad argument #{} to 'char' (number expected, got {})",
                            idx + 1,
                            val.type_name()
                        )
                        .into_value(ctx)
                        .into());
                    }
                };

                let ch = match u32::try_from(code).ok().and_then(char::from_u32) {
                    Some(c) => c,
                    None => {
                        return Err(format!(
                            "bad argument #{} to 'char' (value out of range)",
                            idx + 1
                        )
                        .into_value(ctx)
                        .into());
                    }
                };

                let mut buf = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }

            let result = ctx.intern(&bytes);
            stack.replace(ctx, result);
            Ok(CallbackReturn::Return)
        }),
    );

    utf8.set_field(ctx, "charpattern", r"[\0-\x7F\xC2-\xF4][\x80-\xBF]*");

    utf8.set_field(
        ctx,
        "codes",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let s = stack.consume::<String>(ctx)?;
            let bytes = s.as_bytes();
            if !bytes.is_empty() && (bytes[0] & 0xC0) == 0x80 {
                return Err("invalid UTF-8 code".into_value(ctx).into());
            }

            let iter_fn = Callback::from_fn(&ctx, |ctx, _, mut stack| {
                let (s, n): (String, i64) = stack.consume(ctx)?;
                let bytes = s.as_bytes();
                let len = bytes.len();

                let n = if n <= 0 {
                    0
                } else {
                    let mut pos = (n - 1) as usize;
                    if pos < len {
                        pos += 1;
                        while pos < len && (bytes[pos] & 0xC0) == 0x80 {
                            pos += 1;
                        }
                    }
                    pos
                };

                if n >= len {
                    stack.replace(ctx, (Value::Nil, Value::Nil));
                    return Ok(CallbackReturn::Return);
                }

                let (c, _) = match decode_utf8(&bytes[n..]) {
                    Some(res) => res,
                    None => return Err("invalid UTF-8 code".into_value(ctx).into()),
                };

                let pos = (n as i64) + 1;
                let codepoint = c as u32 as i64;
                stack.replace(ctx, (pos, codepoint));
                Ok(CallbackReturn::Return)
            });

            stack.replace(ctx, (iter_fn, s, 0));
            Ok(CallbackReturn::Return)
        }),
    );

    utf8.set_field(
        ctx,
        "codepoint",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, i, j) = stack.consume::<(String, Option<i64>, Option<i64>)>(ctx)?;
            let bytes = s.as_bytes();
            let len = bytes.len();

            let i = i.unwrap_or(1);
            let j = j.unwrap_or(i);

            let posi = if i >= 0 {
                i
            } else if (-i) as usize > len {
                0
            } else {
                len as i64 + i + 1
            };

            let pose = if j >= 0 {
                j
            } else if (-j) as usize > len {
                0
            } else {
                len as i64 + j + 1
            };

            if posi < 1 {
                return Err("bad argument #2 (out of range)".into_value(ctx).into());
            }
            if pose > len as i64 {
                return Err("bad argument #3 (out of range)".into_value(ctx).into());
            }

            if posi > pose {
                return Ok(CallbackReturn::Return);
            }

            let start = (posi - 1) as usize;
            let end = pose as usize;

            let mut pos = start;
            while pos < end {
                let (c, char_len) = match decode_utf8(&bytes[pos..]) {
                    Some(res) => res,
                    None => {
                        return Err(format!(
                            "bad argument #1 to 'codepoint' (invalid byte sequence at {})",
                            pos + 1
                        )
                        .into_value(ctx)
                        .into());
                    }
                };
                stack.push_back(Value::Integer(c as u32 as i64));
                pos += char_len;
            }

            Ok(CallbackReturn::Return)
        }),
    );

    utf8.set_field(
        ctx,
        "len",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, i, j) = stack.consume::<(String, Option<i64>, Option<i64>)>(ctx)?;
            let bytes = s.as_bytes();
            let len = bytes.len();

            let start = convert_index(i.unwrap_or(1), len).unwrap_or(usize::MAX);
            let end = convert_index_end(j.unwrap_or(len as i64), len).unwrap_or(usize::MAX);

            if len == 0 || start >= len || start >= end {
                stack.replace(ctx, 0);
                return Ok(CallbackReturn::Return);
            }

            let end_inclusive = (end - 1).min(len - 1);
            let mut pos = start;
            let mut count = 0i64;

            while pos <= end_inclusive && pos < len {
                match decode_utf8(&bytes[pos..]) {
                    Some((_, char_len)) => {
                        count += 1;
                        pos += char_len;
                    }
                    None => {
                        stack.replace(ctx, (Value::Nil, (pos as i64) + 1));
                        return Ok(CallbackReturn::Return);
                    }
                }
            }

            stack.replace(ctx, count);
            Ok(CallbackReturn::Return)
        }),
    );

    utf8.set_field(
        ctx,
        "offset",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, n, i) = stack.consume::<(String, i64, Option<i64>)>(ctx)?;
            let bytes = s.as_bytes();
            let len = bytes.len();

            let i = i.unwrap_or(if n >= 0 { 1 } else { len as i64 + 1 });

            if i == 0 || i < -(len as i64) || i > len as i64 + 1 {
                return Err("bad argument #3 to 'offset' (position out of bounds)"
                    .into_value(ctx)
                    .into());
            }

            let position = if i > 0 {
                (i - 1) as usize
            } else {
                (len as i64 + i) as usize
            };

            if n != 0 && position < len && (bytes[position] & 0xC0) == 0x80 {
                return Err("initial position is a continuation byte"
                    .into_value(ctx)
                    .into());
            }

            if n == 0 {
                if position >= len {
                    stack.replace(ctx, Value::Nil);
                    return Ok(CallbackReturn::Return);
                }

                let mut pos = position;
                while pos > 0 && (bytes[pos] & 0xC0) == 0x80 {
                    pos -= 1;
                }

                stack.replace(ctx, (pos as i64) + 1);
                return Ok(CallbackReturn::Return);
            }

            if n > 0 {
                let mut count = 0i64;
                let mut pos = position;

                while count < n && pos < len {
                    if (bytes[pos] & 0xC0) != 0x80 {
                        count += 1;
                    }

                    if count == n {
                        break;
                    }

                    pos += 1;
                }

                if count == n {
                    stack.replace(ctx, (pos as i64) + 1);
                } else if count == n - 1 && pos == len {
                    stack.replace(ctx, (pos as i64) + 1);
                } else {
                    stack.replace(ctx, Value::Nil);
                }
                return Ok(CallbackReturn::Return);
            }

            if n < 0 {
                let target = -n;
                let mut count = 0i64;
                let mut pos = position;

                while count < target {
                    if pos == 0 {
                        stack.replace(ctx, Value::Nil);
                        return Ok(CallbackReturn::Return);
                    }
                    pos -= 1;
                    if (bytes[pos] & 0xC0) != 0x80 {
                        count += 1;
                    }
                }
                stack.replace(ctx, (pos as i64) + 1);
                return Ok(CallbackReturn::Return);
            }

            Ok(CallbackReturn::Return)
        }),
    );

    ctx.set_global("utf8", utf8);
}
