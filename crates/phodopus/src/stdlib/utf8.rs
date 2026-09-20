use crate::{Callback, CallbackReturn, Context, IntoValue, String, Table, Value};

use super::sandbox;

mod sequences;

use sequences::{CodepointSequence, LenSequence, OffsetSequence};

#[inline]
fn iscont(b: u8) -> bool {
    (b & 0xC0) == 0x80
}

#[inline]
pub(crate) fn u_posrelat(pos: i64, len: usize) -> i64 {
    if pos >= 0 {
        pos
    } else if (pos as u64).wrapping_neg() > len as u64 {
        0
    } else {
        len as i64 + pos + 1
    }
}

pub(crate) fn decode_utf8(bytes: &[u8]) -> Option<(char, usize)> {
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
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
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
                let encoded = ch.encode_utf8(&mut buf).as_bytes();
                if sandbox::checked_output_growth(bytes.len(), encoded.len()).is_none() {
                    return Err("resulting string too large".into_value(ctx).into());
                }
                // `bytes` is a native (untracked) buffer, so charge the projected size against
                // the hard quota before it grows; otherwise a hostile `utf8.char` with thousands
                // of codepoints builds a multi-kilobyte buffer the GC boundary never observes.
                ctx.check_memory(bytes.len().saturating_add(encoded.len()))?;
                exec.fuel().consume(sandbox::output_cost(encoded.len()));
                bytes.extend_from_slice(encoded);
            }

            let result = ctx.intern(&bytes);
            stack.replace(ctx, result);
            Ok(CallbackReturn::Return)
        }),
    );

    utf8.set_field(
        ctx,
        "charpattern",
        ctx.intern(b"[\0-\x7F\xC2-\xF4][\x80-\xBF]*"),
    );

    utf8.set_field(
        ctx,
        "codes",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let s = stack.consume::<String>(ctx)?;
            let bytes = s.as_bytes();
            if !bytes.is_empty() && iscont(bytes[0]) {
                return Err("bad argument #1 to 'codes' (invalid UTF-8 code)"
                    .into_value(ctx)
                    .into());
            }

            let iter_fn = Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
                let (s, n): (String, i64) = stack.consume(ctx)?;
                let bytes = s.as_bytes();
                let len = bytes.len();

                let n = if n <= 0 {
                    0
                } else {
                    let mut pos = (n as usize) - 1;
                    if pos < len {
                        pos += 1;
                        while pos < len && iscont(bytes[pos]) {
                            pos += 1;
                        }
                    }
                    pos
                };

                // The scan above is bounded by the input length; charge it so
                // repeated `gmatch`-style iteration is accounted.
                exec.fuel().consume(sandbox::scanned_cost(n));

                if n >= len {
                    stack.replace(ctx, (Value::Nil, Value::Nil));
                    return Ok(CallbackReturn::Return);
                }

                let (c, char_len) = match decode_utf8(&bytes[n..]) {
                    Some(res) => res,
                    None => return Err("invalid UTF-8 code".into_value(ctx).into()),
                };

                let next = n + char_len;
                if next < len && iscont(bytes[next]) {
                    return Err("invalid UTF-8 code".into_value(ctx).into());
                }

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

            let posi = u_posrelat(i.unwrap_or(1), len);
            let pose = u_posrelat(j.unwrap_or(posi), len);

            if posi < 1 {
                return Err("bad argument #2 to 'utf8.codepoint' (out of bounds)"
                    .into_value(ctx)
                    .into());
            }
            if pose > len as i64 {
                return Err("bad argument #3 to 'utf8.codepoint' (out of bounds)"
                    .into_value(ctx)
                    .into());
            }

            if posi > pose {
                return Ok(CallbackReturn::Return);
            }

            Ok(CodepointSequence::create(ctx, bytes, posi, pose))
        }),
    );

    utf8.set_field(
        ctx,
        "len",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, i, j) = stack.consume::<(String, Option<i64>, Option<i64>)>(ctx)?;
            let bytes = s.as_bytes();
            let len = bytes.len();

            let posi = u_posrelat(i.unwrap_or(1), len);
            let posj = u_posrelat(j.unwrap_or(-1), len);

            if posi < 1 || posi > (len as i64) + 1 {
                return Err(
                    "bad argument #2 to 'utf8.len' (initial position out of bounds)"
                        .into_value(ctx)
                        .into(),
                );
            }
            if posj > len as i64 {
                return Err(
                    "bad argument #3 to 'utf8.len' (final position out of bounds)"
                        .into_value(ctx)
                        .into(),
                );
            }

            if posi > posj {
                stack.replace(ctx, 0);
                return Ok(CallbackReturn::Return);
            }

            Ok(LenSequence::create(ctx, bytes, posi, posj))
        }),
    );

    utf8.set_field(
        ctx,
        "offset",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (s, n, i) = stack.consume::<(String, i64, Option<i64>)>(ctx)?;
            let bytes = s.as_bytes();
            let len = bytes.len();

            let def_pos = if n >= 0 { 1 } else { (len as i64) + 1 };
            let posi = u_posrelat(i.unwrap_or(def_pos), len);

            if posi < 1 || posi > (len as i64) + 1 {
                return Err("bad argument #3 to 'utf8.offset' (position out of bounds)"
                    .into_value(ctx)
                    .into());
            }

            let pos = (posi - 1) as usize;

            if n == 0 {
                return Ok(OffsetSequence::create(ctx, bytes, pos, 0, 0));
            }

            if pos < len && iscont(bytes[pos]) {
                return Err("initial position is a continuation byte"
                    .into_value(ctx)
                    .into());
            }

            if n < 0 {
                Ok(OffsetSequence::create(ctx, bytes, pos, n, -1))
            } else {
                Ok(OffsetSequence::create(ctx, bytes, pos, n, 1))
            }
        }),
    );

    ctx.set_global("utf8", utf8);
}
