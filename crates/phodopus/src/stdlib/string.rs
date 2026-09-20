use crate::{
    Callback, CallbackReturn, Context, Error, FromValue, IntoValue, MetaMethod, String, Table,
    Value,
};

use super::sandbox;

mod format;
mod pack;
mod packsize;
mod patterns;
mod unpack;

/// Maximum buffer allocation size for `string.rep` (16 MiB sandbox ceiling).
pub const MAX_STRING_REP_BYTES: usize = 16 * 1024 * 1024;

/// Maximum buffer allocation size for `string.pack` (16 MiB sandbox ceiling).
pub const MAX_STRING_PACK_BYTES: usize = 16 * 1024 * 1024;

/// A resumable byte cursor over a `string.pack` / `string.unpack` format string.
///
/// The format string is copied into an owned buffer so the cursor can live in a
/// heap-allocated `Sequence` without holding a GC reference. Advancing is
/// charged to `Execution::fuel` by the owning sequence, which returns
/// `SequencePoll::Pending` when the current Fuel slice is exhausted and resumes
/// at the same byte offset.
#[derive(Debug, Clone, gc_arena::Collect)]
#[collect(require_static)]
pub(crate) struct FormatCursor {
    bytes: Vec<u8>,
    pos: usize,
}

impl FormatCursor {
    pub(crate) fn create(fmt: &str) -> Self {
        Self {
            bytes: fmt.as_bytes().to_vec(),
            pos: 0,
        }
    }

    pub(crate) fn next_char(&mut self) -> Option<char> {
        let rest = std::str::from_utf8(&self.bytes[self.pos..]).ok()?;
        let ch = rest.chars().next()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    pub(crate) fn is_done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Parses an ASCII decimal number at the cursor, consuming its digits.
    pub(crate) fn parse_number(&mut self) -> Result<Option<usize>, std::string::String> {
        let start = self.pos;
        let mut val: usize = 0;
        while let Some(&byte) = self.bytes.get(self.pos) {
            if !byte.is_ascii_digit() {
                break;
            }
            let digit = usize::from(byte - b'0');
            val = val
                .checked_mul(10)
                .and_then(|v| v.checked_add(digit))
                .ok_or_else(|| "invalid number in format string".to_string())?;
            self.pos += 1;
        }
        if self.pos == start {
            Ok(None)
        } else {
            Ok(Some(val))
        }
    }
}

pub fn load_string<'gc>(ctx: Context<'gc>) {
    let string = Table::new(&ctx);

    string.set_field(
        ctx,
        "len",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let string = stack.consume::<String>(ctx)?;
            let len = string.len();
            stack.replace(ctx, len);
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "byte",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let (string, i, j) = stack.consume::<(String, Option<i64>, Option<i64>)>(ctx)?;
            let i = i.unwrap_or(1);
            let substr = sub(string.as_bytes(), i, j.or(Some(i)))?;
            exec.fuel().consume(sandbox::output_cost(substr.len()));
            stack.extend(substr.iter().map(|b| Value::Integer(i64::from(*b))));
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "char",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let bytes = stack
                .into_iter()
                .map(|c| u8::from_value(ctx, c))
                .collect::<Result<Vec<_>, _>>()?;
            if sandbox::checked_output_growth(0, bytes.len()).is_none() {
                return Err("resulting string too large".into_value(ctx).into());
            }
            exec.fuel().consume(sandbox::output_cost(bytes.len()));
            let string = ctx.intern(&bytes);
            stack.replace(ctx, string);
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "sub",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let (string, i, j) = stack.consume::<(String, i64, Option<i64>)>(ctx)?;
            let raw = sub(string.as_bytes(), i, j)?;
            exec.fuel().consume(sandbox::output_cost(raw.len()));
            let substr = ctx.intern(raw);
            stack.replace(ctx, substr);
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "lower",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let string = stack.consume::<String>(ctx)?;
            exec.fuel()
                .consume(sandbox::output_cost(string.len() as usize));
            let lowered = ctx.intern(
                &string
                    .as_bytes()
                    .iter()
                    .map(u8::to_ascii_lowercase)
                    .collect::<Vec<_>>(),
            );
            stack.replace(ctx, lowered);
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "reverse",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let string = stack.consume::<String>(ctx)?;
            exec.fuel()
                .consume(sandbox::output_cost(string.len() as usize));
            let reversed = ctx.intern(&string.as_bytes().iter().copied().rev().collect::<Vec<_>>());
            stack.replace(ctx, reversed);
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "upper",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let string = stack.consume::<String>(ctx)?;
            exec.fuel()
                .consume(sandbox::output_cost(string.len() as usize));
            let uppered = ctx.intern(
                &string
                    .as_bytes()
                    .iter()
                    .map(u8::to_ascii_uppercase)
                    .collect::<Vec<_>>(),
            );
            stack.replace(ctx, uppered);
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "format",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let format_val = stack
                .pop_front()
                .ok_or_else(|| "bad argument #1 to 'format' (string expected, got no value)")
                .map_err(|err| err.into_value(ctx))?;
            let formatstring = String::from_value(ctx, format_val)?;
            let formatstring = formatstring.to_str()?;

            let args: Vec<Value> = stack.into_iter().collect();

            format::FormatSequence::create(ctx, formatstring, args)
        }),
    );

    string.set_field(
        ctx,
        "rep",
        Callback::from_fn(&ctx, |ctx, mut exec, mut stack| {
            let (s, n, sep) = stack.consume::<(String, i64, Option<String>)>(ctx)?;

            if n <= 0 {
                stack.replace(ctx, ctx.intern_static(b""));
                return Ok(CallbackReturn::Return);
            }

            if n == 1 {
                stack.replace(ctx, s);
                return Ok(CallbackReturn::Return);
            }

            let n = usize::try_from(n)
                .map_err(|_| Error::from_value("resulting string too large".into_value(ctx)))?;

            let s_bytes = s.as_bytes();
            let sep_bytes = sep.as_ref().map(|s| s.as_bytes()).unwrap_or(b"");

            let s_total_len = s_bytes.len().checked_mul(n);
            let sep_total_len = sep_bytes.len().checked_mul(n - 1);

            let required_cap = match (s_total_len, sep_total_len) {
                (Some(s_total), Some(sep_total)) => s_total.checked_add(sep_total),
                _ => None,
            };

            let capacity = required_cap
                .filter(|&cap| cap <= MAX_STRING_REP_BYTES)
                .ok_or_else(|| Error::from_value("resulting string too large".into_value(ctx)))?;

            if capacity == 0 {
                stack.replace(ctx, ctx.intern_static(b""));
                return Ok(CallbackReturn::Return);
            }

            // `string.rep` is proven constant-bounded: the checked capacity above
            // caps the result at `MAX_STRING_REP_BYTES`, so the copy loop cannot
            // exceed a fixed wall slice. Charge the bytes it writes so that the
            // work is still accounted deterministically.
            exec.fuel().consume(sandbox::output_cost(capacity));

            let mut result = Vec::with_capacity(capacity);
            result.extend_from_slice(s_bytes);
            if sep_bytes.is_empty() {
                for _ in 1..n {
                    result.extend_from_slice(s_bytes);
                }
            } else {
                for _ in 1..n {
                    result.extend_from_slice(sep_bytes);
                    result.extend_from_slice(s_bytes);
                }
            }

            stack.replace(ctx, ctx.intern(&result));
            Ok(CallbackReturn::Return)
        }),
    );

    string.set_field(
        ctx,
        "pack",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let fmt_val = stack
                .pop_front()
                .ok_or_else(|| "bad argument #1 to 'pack' (string expected, got no value)")
                .map_err(|err| err.into_value(ctx))?;
            let fmt = String::from_value(ctx, fmt_val)?;
            let fmt_str = fmt.to_str()?;

            let args: Vec<Value> = stack.into_iter().collect();

            Ok(pack::PackSequence::create(ctx, fmt_str, args))
        }),
    );

    string.set_field(
        ctx,
        "unpack",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (fmt, s, init) = stack.consume::<(String, String, Option<i64>)>(ctx)?;

            let fmt_str = fmt.to_str()?;
            let bytes = s.as_bytes();
            let init = init.unwrap_or(1);

            let len = bytes.len();
            let pos = if init > 0 {
                init as usize
            } else if init < 0 {
                let abs_init = init.unsigned_abs() as usize;
                if abs_init > len {
                    0
                } else {
                    len - abs_init + 1
                }
            } else {
                0
            };

            if pos < 1 || pos > len + 1 {
                return Err(Error::from_value(
                    "initial position out of string".into_value(ctx),
                ));
            }

            let start_pos = pos - 1;

            Ok(unpack::UnpackSequence::create(
                ctx, fmt_str, bytes, start_pos,
            ))
        }),
    );

    string.set_field(
        ctx,
        "packsize",
        Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let fmt = stack.consume::<String>(ctx)?;
            let fmt_str = fmt.to_str()?;

            Ok(packsize::PacksizeSequence::create(ctx, fmt_str))
        }),
    );

    patterns::load_patterns(ctx, &string);

    ctx.string_metatable()
        .set(ctx, MetaMethod::Index, string)
        .unwrap();

    ctx.set_global("string", string);
}

fn sub(string: &[u8], i: i64, j: Option<i64>) -> Result<&[u8], std::num::TryFromIntError> {
    let i = match i {
        i if i > 0 => i.saturating_sub(1).try_into()?,
        0 => 0,
        i => string.len().saturating_sub(i.unsigned_abs().try_into()?),
    };
    let j = if let Some(j) = j {
        if j >= 0 {
            j.try_into()?
        } else {
            let j: usize = j.unsigned_abs().try_into()?;
            string.len().saturating_sub(j.saturating_sub(1))
        }
    } else {
        string.len()
    }
    .clamp(0, string.len());

    Ok(if i >= j || i >= string.len() {
        &[]
    } else {
        &string[i..j]
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, gc_arena::Collect)]
#[collect(require_static)]
pub(crate) enum Endianness {
    Little,
    Big,
    Native,
}

impl Default for Endianness {
    fn default() -> Self {
        Endianness::Native
    }
}

#[derive(Debug, Clone, Copy, gc_arena::Collect)]
#[collect(require_static)]
pub(crate) struct FormatState {
    pub endianness: Endianness,
    pub max_alignment: usize,
}

impl Default for FormatState {
    fn default() -> Self {
        FormatState {
            endianness: Endianness::Native,
            max_alignment: 1,
        }
    }
}

pub(crate) fn calculate_padding(
    current_pos: usize,
    data_size: usize,
    max_alignment: usize,
) -> usize {
    if max_alignment <= 1 || data_size <= 1 {
        return 0;
    }
    let alignment = std::cmp::min(data_size, max_alignment);
    if !alignment.is_power_of_two() {
        return 0;
    }
    (alignment - (current_pos % alignment)) % alignment
}

pub(crate) fn get_format_size(format_char: char, num_opt: Option<usize>) -> Option<usize> {
    match format_char {
        'b' | 'B' | 'x' => Some(1),
        'h' | 'H' => Some(std::mem::size_of::<i16>()),
        'l' | 'L' | 'j' => Some(std::mem::size_of::<i64>()),
        'J' => Some(std::mem::size_of::<u64>()),
        'T' => Some(std::mem::size_of::<usize>()),
        'i' | 'I' => Some(num_opt.unwrap_or(std::mem::size_of::<i32>())),
        'f' => Some(std::mem::size_of::<f32>()),
        'd' | 'n' => Some(std::mem::size_of::<f64>()),
        'c' => num_opt,
        'z' | 's' => None,
        _ => None,
    }
}

pub(crate) fn get_align_size_for_option(
    op: char,
    num_opt: Option<usize>,
) -> Result<usize, std::string::String> {
    match op {
        'b' | 'B' | 'x' => Ok(1),
        'h' | 'H' => Ok(2),
        'l' | 'L' | 'j' | 'J' | 'T' => Ok(8),
        'f' => Ok(4),
        'd' | 'n' => Ok(8),
        'i' | 'I' => {
            let n = num_opt.unwrap_or(4);
            if !(1..=16).contains(&n) {
                return Err(format!("integral size {} out of limits [1, 16]", n));
            }
            Ok(n)
        }
        's' => {
            let n = num_opt.unwrap_or(std::mem::size_of::<usize>());
            if !(1..=16).contains(&n) {
                return Err(format!("integral size {} out of limits [1, 16]", n));
            }
            Ok(n)
        }
        'c' | 'z' => Ok(1),
        _ => Err(format!("invalid option '{}' following 'X'", op)),
    }
}
