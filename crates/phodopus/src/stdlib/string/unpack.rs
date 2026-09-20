use std::pin::Pin;

use gc_arena::Collect;

use crate::{
    BoxSequence, CallbackReturn, Context, Error, Execution, IntoValue, Sequence, SequencePoll,
    Stack, Value,
};

use super::{Endianness, FormatCursor, FormatState, calculate_padding, get_align_size_for_option};

/// Fuel charged for advancing over one format-string byte.
const FUEL_PER_FORMAT_BYTE: i32 = 1;

/// A resumable implementation of `string.unpack`.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct UnpackSequence<'gc> {
    fmt: FormatCursor,
    state: FormatState,
    #[collect(require_static)]
    bytes: Vec<u8>,
    #[collect(require_static)]
    source: Vec<u8>,
    pos: usize,
    values: Vec<Value<'gc>>,
}

impl<'gc> UnpackSequence<'gc> {
    pub(crate) fn create(
        ctx: Context<'gc>,
        fmt: &str,
        bytes: &[u8],
        start_pos: usize,
    ) -> CallbackReturn<'gc> {
        CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            UnpackSequence {
                fmt: FormatCursor::create(fmt),
                state: FormatState::default(),
                bytes: bytes.to_vec(),
                source: Vec::new(),
                pos: start_pos,
                values: Vec::new(),
            },
        ))
    }
}

impl<'gc> Sequence<'gc> for UnpackSequence<'gc> {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let seq = self.as_mut().get_mut();
        run(seq, ctx, exec.fuel())?;

        if !seq.fmt.is_done() {
            return Ok(SequencePoll::Pending);
        }

        stack.clear();
        for value in seq.values.drain(..) {
            stack.push_back(value);
        }
        stack.push_back(Value::Integer(seq.pos as i64 + 1));
        Ok(SequencePoll::Return)
    }
}

fn run<'gc>(
    seq: &mut UnpackSequence<'gc>,
    ctx: Context<'gc>,
    fuel: &mut crate::Fuel,
) -> Result<(), Error<'gc>> {
    while let Some(format_char) = seq.fmt.next_char() {
        fuel.consume(FUEL_PER_FORMAT_BYTE);
        process_option(seq, ctx, format_char)?;

        if !fuel.should_continue() && !seq.fmt.is_done() {
            return Ok(());
        }
    }
    Ok(())
}

fn process_option<'gc>(
    seq: &mut UnpackSequence<'gc>,
    ctx: Context<'gc>,
    format_char: char,
) -> Result<(), Error<'gc>> {
    let state = &mut seq.state;
    let bytes = &seq.bytes;
    let pos = &mut seq.pos;
    let values = &mut seq.values;
    let fmt = &mut seq.fmt;
    // `source` is unused by the current model; keep the field for parity with
    // the previous in-place implementation.
    let _ = &seq.source;

    match format_char {
        '<' => state.endianness = Endianness::Little,
        '>' => state.endianness = Endianness::Big,
        '=' => state.endianness = Endianness::Native,
        '!' => {
            let n = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .unwrap_or(std::mem::size_of::<usize>());
            if n < 1 || n > 16 || !n.is_power_of_two() {
                return Err(format!(
                    "alignment option '!' requires a power of 2 between 1 and 16 (got {})",
                    n
                )
                .into_value(ctx)
                .into());
            }
            state.max_alignment = n;
        }
        ' ' | '\t' | '\r' | '\n' => {}
        'x' => {
            skip_padding(bytes, pos, 1).map_err(|err| Error::from_value(err.into_value(ctx)))?;
        }
        'X' => {
            let op = fmt.next_char().ok_or_else(|| {
                Error::from_value("'X' must be followed by an option character".into_value(ctx))
            })?;
            let num_opt = if matches!(op, 'i' | 'I' | 's' | 'c') {
                fmt.parse_number()
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?
            } else {
                None
            };
            let align_size = get_align_size_for_option(op, num_opt)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let padding = calculate_padding(*pos, align_size, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
        }
        'b' => {
            let padding = calculate_padding(*pos, 1, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_int(bytes, pos, 1, state.endianness, 'b')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'B' => {
            let padding = calculate_padding(*pos, 1, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_uint(bytes, pos, 1, state.endianness, 'B')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'h' => {
            let padding = calculate_padding(*pos, 2, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_int(bytes, pos, 2, state.endianness, 'h')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'H' => {
            let padding = calculate_padding(*pos, 2, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_uint(bytes, pos, 2, state.endianness, 'H')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'l' | 'j' => {
            let padding = calculate_padding(*pos, 8, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_int(bytes, pos, 8, state.endianness, format_char)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'L' | 'J' | 'T' => {
            let padding = calculate_padding(*pos, 8, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_uint(bytes, pos, 8, state.endianness, format_char)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'i' => {
            let size = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .unwrap_or(4);
            if size < 1 || size > 16 {
                return Err(format!("integral size {} out of limits [1, 16]", size)
                    .into_value(ctx)
                    .into());
            }
            let padding = calculate_padding(*pos, size, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_int(bytes, pos, size, state.endianness, 'i')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'I' => {
            let size = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .unwrap_or(4);
            if size < 1 || size > 16 {
                return Err(format!("integral size {} out of limits [1, 16]", size)
                    .into_value(ctx)
                    .into());
            }
            let padding = calculate_padding(*pos, size, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_uint(bytes, pos, size, state.endianness, 'I')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Integer(val));
        }
        'f' => {
            let padding = calculate_padding(*pos, 4, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_float(bytes, pos, state.endianness)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Number(val as f64));
        }
        'd' | 'n' => {
            let padding = calculate_padding(*pos, 8, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let val = read_double(bytes, pos, state.endianness, format_char)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(Value::Number(val));
        }
        'c' => {
            let n = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .ok_or_else(|| {
                    Error::from_value("missing size for format option 'c'".into_value(ctx))
                })?;
            let slice = read_exact(bytes, pos, n, 'c')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(ctx.intern(slice).into_value(ctx));
        }
        'z' => {
            let remaining = &bytes[*pos..];
            match remaining.iter().position(|&b| b == 0) {
                Some(null_pos) => {
                    let str_bytes = &remaining[..null_pos];
                    values.push(ctx.intern(str_bytes).into_value(ctx));
                    *pos += null_pos + 1;
                }
                None => {
                    return Err(Error::from_value(
                        "missing null terminator for 'z' format".into_value(ctx),
                    ));
                }
            }
        }
        's' => {
            let len_size = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .unwrap_or(std::mem::size_of::<usize>());
            if len_size < 1 || len_size > 16 {
                return Err(format!("integral size {} out of limits [1, 16]", len_size)
                    .into_value(ctx)
                    .into());
            }
            let padding = calculate_padding(*pos, len_size, state.max_alignment);
            skip_padding(bytes, pos, padding)
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let len_slice = read_exact(bytes, pos, len_size, 's')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let mut le_bytes = [0u8; 16];
            match state.endianness {
                Endianness::Little => le_bytes[..len_size].copy_from_slice(len_slice),
                Endianness::Big => {
                    for (i, &b) in len_slice.iter().rev().enumerate() {
                        le_bytes[i] = b;
                    }
                }
                Endianness::Native => {
                    if cfg!(target_endian = "little") {
                        le_bytes[..len_size].copy_from_slice(len_slice);
                    } else {
                        for (i, &b) in len_slice.iter().rev().enumerate() {
                            le_bytes[i] = b;
                        }
                    }
                }
            }
            let str_len_u128 = u128::from_le_bytes(le_bytes);
            let str_len = usize::try_from(str_len_u128)
                .map_err(|_| Error::from_value("string length too large".into_value(ctx)))?;
            let str_slice = read_exact(bytes, pos, str_len, 's')
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            values.push(ctx.intern(str_slice).into_value(ctx));
        }
        invalid => {
            return Err(
                format!("invalid conversion option '{}' in format string", invalid)
                    .into_value(ctx)
                    .into(),
            );
        }
    }
    Ok(())
}

fn read_exact<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
    count: usize,
    op: char,
) -> Result<&'a [u8], std::string::String> {
    let start = *pos;
    let end = start
        .checked_add(count)
        .ok_or_else(|| "data string too short".to_string())?;
    if end > bytes.len() {
        return Err(format!("data string too short for format '{}'", op));
    }
    *pos = end;
    Ok(&bytes[start..end])
}

fn skip_padding(bytes: &[u8], pos: &mut usize, padding: usize) -> Result<(), std::string::String> {
    if padding > 0 {
        let end = pos
            .checked_add(padding)
            .ok_or_else(|| "data string too short".to_string())?;
        if end > bytes.len() {
            return Err("data string too short".to_string());
        }
        *pos = end;
    }
    Ok(())
}

fn read_int(
    bytes: &[u8],
    pos: &mut usize,
    size: usize,
    endianness: Endianness,
    op: char,
) -> Result<i64, std::string::String> {
    let slice = read_exact(bytes, pos, size, op)?;
    let mut le_bytes = [0u8; 16];
    match endianness {
        Endianness::Little => le_bytes[..size].copy_from_slice(slice),
        Endianness::Big => {
            for (i, &b) in slice.iter().rev().enumerate() {
                le_bytes[i] = b;
            }
        }
        Endianness::Native => {
            if cfg!(target_endian = "little") {
                le_bytes[..size].copy_from_slice(slice);
            } else {
                for (i, &b) in slice.iter().rev().enumerate() {
                    le_bytes[i] = b;
                }
            }
        }
    }
    if size < 16 {
        let sign = if (le_bytes[size - 1] & 0x80) != 0 {
            0xff
        } else {
            0x00
        };
        for i in size..16 {
            le_bytes[i] = sign;
        }
    }
    let val128 = i128::from_le_bytes(le_bytes);
    i64::try_from(val128)
        .map_err(|_| format!("{}-byte integer does not fit into Lua Integer", size))
}

fn read_uint(
    bytes: &[u8],
    pos: &mut usize,
    size: usize,
    endianness: Endianness,
    op: char,
) -> Result<i64, std::string::String> {
    let slice = read_exact(bytes, pos, size, op)?;
    let mut le_bytes = [0u8; 16];
    match endianness {
        Endianness::Little => le_bytes[..size].copy_from_slice(slice),
        Endianness::Big => {
            for (i, &b) in slice.iter().rev().enumerate() {
                le_bytes[i] = b;
            }
        }
        Endianness::Native => {
            if cfg!(target_endian = "little") {
                le_bytes[..size].copy_from_slice(slice);
            } else {
                for (i, &b) in slice.iter().rev().enumerate() {
                    le_bytes[i] = b;
                }
            }
        }
    }
    let val128 = u128::from_le_bytes(le_bytes);
    if size <= 8 {
        Ok(val128 as u64 as i64)
    } else {
        if val128 > i64::MAX as u128 {
            return Err(format!(
                "unsigned value {} read for format '{}' does not fit in `integer`",
                val128, op
            ));
        }
        Ok(val128 as i64)
    }
}

fn read_float(
    bytes: &[u8],
    pos: &mut usize,
    endianness: Endianness,
) -> Result<f32, std::string::String> {
    let slice = read_exact(bytes, pos, 4, 'f')?;
    let arr: [u8; 4] = slice.try_into().unwrap();
    Ok(match endianness {
        Endianness::Little => f32::from_le_bytes(arr),
        Endianness::Big => f32::from_be_bytes(arr),
        Endianness::Native => f32::from_ne_bytes(arr),
    })
}

fn read_double(
    bytes: &[u8],
    pos: &mut usize,
    endianness: Endianness,
    op: char,
) -> Result<f64, std::string::String> {
    let slice = read_exact(bytes, pos, 8, op)?;
    let arr: [u8; 8] = slice.try_into().unwrap();
    Ok(match endianness {
        Endianness::Little => f64::from_le_bytes(arr),
        Endianness::Big => f64::from_be_bytes(arr),
        Endianness::Native => f64::from_ne_bytes(arr),
    })
}
