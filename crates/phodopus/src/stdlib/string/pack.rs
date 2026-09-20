use std::pin::Pin;

use gc_arena::Collect;

use crate::{
    BoxSequence, CallbackReturn, Context, Error, Execution, IntoValue, Sequence, SequencePoll,
    Stack, Value,
};

use super::super::sandbox::FUEL_PER_FORMAT_BYTE;
use super::{
    Endianness, FormatCursor, FormatState, MAX_STRING_PACK_BYTES, calculate_padding,
    get_align_size_for_option,
};

/// A resumable implementation of `string.pack`.
///
/// The format string is walked once with a [`FormatCursor`]; each `poll`
/// consumes format bytes and appends output until the remaining Fuel runs out,
/// then returns [`SequencePoll::Pending`] with the writer, cursor, and argument
/// index preserved. Every append is checked against `MAX_STRING_PACK_BYTES`, so
/// the output can never exceed the 16 MiB sandbox ceiling.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct PackSequence<'gc> {
    fmt: FormatCursor,
    state: FormatState,
    #[collect(require_static)]
    writer: Vec<u8>,
    args: Vec<Value<'gc>>,
    arg_index: usize,
}

impl<'gc> PackSequence<'gc> {
    pub(crate) fn create(
        ctx: Context<'gc>,
        fmt: &str,
        args: Vec<Value<'gc>>,
    ) -> CallbackReturn<'gc> {
        CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            PackSequence {
                fmt: FormatCursor::create(fmt),
                state: FormatState::default(),
                writer: Vec::new(),
                args,
                arg_index: 0,
            },
        ))
    }
}

impl<'gc> Sequence<'gc> for PackSequence<'gc> {
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

        let result = ctx.intern(&seq.writer);
        stack.replace(ctx, result);
        Ok(SequencePoll::Return)
    }
}

/// Advances the pack machine until the format is exhausted or `fuel` runs out.
fn run<'gc>(
    seq: &mut PackSequence<'gc>,
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
    seq: &mut PackSequence<'gc>,
    ctx: Context<'gc>,
    format_char: char,
) -> Result<(), Error<'gc>> {
    let state = &mut seq.state;
    let writer = &mut seq.writer;
    let fmt = &mut seq.fmt;
    let args = &seq.args;
    let arg_index = &mut seq.arg_index;
    // 1-based argument number of the value about to be consumed.
    let arg_num = *arg_index + 2;

    match format_char {
        '<' => state.endianness = Endianness::Little,
        '>' => state.endianness = Endianness::Big,
        '=' => state.endianness = Endianness::Native,
        '!' => {
            let num_opt = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?;
            let n = num_opt.unwrap_or(std::mem::size_of::<usize>());
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
            push_zeros(writer, 1, ctx)?;
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
            let padding = calculate_padding(writer.len(), align_size, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
        }
        'b' => {
            let val = take_integer(args, arg_index, ctx)?;
            if val < i8::MIN as i64 || val > i8::MAX as i64 {
                return Err(integer_overflow(ctx, arg_num));
            }
            let padding = calculate_padding(writer.len(), 1, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            writer.push(val as u8);
        }
        'B' => {
            let val = take_integer(args, arg_index, ctx)?;
            if val < 0 || val > u8::MAX as i64 {
                return Err(unsigned_overflow(ctx, arg_num));
            }
            let padding = calculate_padding(writer.len(), 1, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            writer.push(val as u8);
        }
        'h' => {
            let val = take_integer(args, arg_index, ctx)?;
            if val < i16::MIN as i64 || val > i16::MAX as i64 {
                return Err(integer_overflow(ctx, arg_num));
            }
            let padding = calculate_padding(writer.len(), 2, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_int(writer, val, 2, state.endianness);
        }
        'H' => {
            let val = take_integer(args, arg_index, ctx)?;
            if val < 0 || val > u16::MAX as i64 {
                return Err(unsigned_overflow(ctx, arg_num));
            }
            let padding = calculate_padding(writer.len(), 2, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_uint(writer, val as u64 as u128, 2, state.endianness);
        }
        'l' | 'j' => {
            let val = take_integer(args, arg_index, ctx)?;
            let padding = calculate_padding(writer.len(), 8, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_int(writer, val, 8, state.endianness);
        }
        'L' | 'J' | 'T' => {
            let val = take_integer(args, arg_index, ctx)?;
            let padding = calculate_padding(writer.len(), 8, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_uint(writer, val as u64 as u128, 8, state.endianness);
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
            let val = take_integer(args, arg_index, ctx)?;
            if size < 8 {
                let min = -(1i64 << (size * 8 - 1));
                let max = (1i64 << (size * 8 - 1)) - 1;
                if val < min || val > max {
                    return Err(integer_overflow(ctx, arg_num));
                }
            }
            let padding = calculate_padding(writer.len(), size, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_int(writer, val, size, state.endianness);
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
            let val = take_integer(args, arg_index, ctx)?;
            if size < 8 {
                let max = (1i64 << (size * 8)) - 1;
                if val < 0 || val > max {
                    return Err(unsigned_overflow(ctx, arg_num));
                }
            } else if size > 8 && val < 0 {
                return Err(unsigned_overflow(ctx, arg_num));
            }
            let padding = calculate_padding(writer.len(), size, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_uint(writer, val as u64 as u128, size, state.endianness);
        }
        'f' => {
            let val = take_number(args, arg_index, ctx)?;
            let padding = calculate_padding(writer.len(), 4, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_float(writer, val as f32, state.endianness);
        }
        'd' | 'n' => {
            let val = take_number(args, arg_index, ctx)?;
            let padding = calculate_padding(writer.len(), 8, state.max_alignment);
            push_zeros(writer, padding, ctx)?;
            write_double(writer, val, state.endianness);
        }
        'c' => {
            let n = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .ok_or_else(|| {
                    Error::from_value("missing size for format option 'c'".into_value(ctx))
                })?;
            if n > MAX_STRING_PACK_BYTES {
                return Err("resulting string too large".into_value(ctx).into());
            }
            let s = take_string(args, arg_index, ctx)?;
            let bytes = s.as_bytes();
            if bytes.len() > n {
                return Err(format!(
                    "bad argument #{} to 'pack' (string longer than given size)",
                    arg_num
                )
                .into_value(ctx)
                .into());
            }
            ensure_capacity(writer, n, ctx)?;
            writer.extend_from_slice(bytes);
            writer.resize(writer.len() + (n - bytes.len()), 0);
        }
        'z' => {
            let s = take_string(args, arg_index, ctx)?;
            let bytes = s.as_bytes();
            if bytes.contains(&0) {
                return Err(format!(
                    "bad argument #{} to 'pack' (string contains zeros)",
                    arg_num
                )
                .into_value(ctx)
                .into());
            }
            ensure_capacity(writer, bytes.len() + 1, ctx)?;
            writer.extend_from_slice(bytes);
            writer.push(0);
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
            let s = take_string(args, arg_index, ctx)?;
            let bytes = s.as_bytes();
            let max_len = if len_size >= 16 {
                u128::MAX
            } else {
                (1u128 << (len_size * 8)) - 1
            };
            if (bytes.len() as u128) > max_len {
                return Err("string length does not result in a valid integer"
                    .into_value(ctx)
                    .into());
            }
            let padding = calculate_padding(writer.len(), len_size, state.max_alignment);
            ensure_capacity(writer, padding + len_size + bytes.len(), ctx)?;
            writer.resize(writer.len() + padding, 0);
            write_uint(writer, bytes.len() as u128, len_size, state.endianness);
            writer.extend_from_slice(bytes);
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

fn ensure_capacity<'gc>(
    writer: &mut Vec<u8>,
    additional: usize,
    ctx: Context<'gc>,
) -> Result<(), Error<'gc>> {
    if writer.len().saturating_add(additional) > MAX_STRING_PACK_BYTES {
        Err("resulting string too large".into_value(ctx).into())
    } else {
        // Charge the projected output buffer against the hard memory quota as well as the
        // 16 MiB ceiling, matching `string.rep`. Without this, a hostile `pack` (for example a
        // `c1000000` repeat) builds a multi-megabyte native buffer that only the GC boundary
        // would ever observe.
        ctx.check_memory(writer.len().saturating_add(additional))?;
        Ok(())
    }
}

fn push_zeros<'gc>(
    writer: &mut Vec<u8>,
    count: usize,
    ctx: Context<'gc>,
) -> Result<(), Error<'gc>> {
    ensure_capacity(writer, count, ctx)?;
    writer.resize(writer.len() + count, 0);
    Ok(())
}

fn integer_overflow<'gc>(ctx: Context<'gc>, arg_num: usize) -> Error<'gc> {
    format!("bad argument #{} to 'pack' (integer overflow)", arg_num)
        .into_value(ctx)
        .into()
}

fn unsigned_overflow<'gc>(ctx: Context<'gc>, arg_num: usize) -> Error<'gc> {
    format!("bad argument #{} to 'pack' (unsigned overflow)", arg_num)
        .into_value(ctx)
        .into()
}

fn take_integer<'gc>(
    args: &[Value<'gc>],
    arg_index: &mut usize,
    ctx: Context<'gc>,
) -> Result<i64, Error<'gc>> {
    let arg_num = *arg_index + 2;
    let val = get_arg(args, arg_index, ctx)?;
    val.to_integer().ok_or_else(|| {
        format!(
            "bad argument #{} to 'pack' (number has no integer representation)",
            arg_num
        )
        .into_value(ctx)
        .into()
    })
}

fn take_number<'gc>(
    args: &[Value<'gc>],
    arg_index: &mut usize,
    ctx: Context<'gc>,
) -> Result<f64, Error<'gc>> {
    let arg_num = *arg_index + 2;
    let val = get_arg(args, arg_index, ctx)?;
    val.to_number().ok_or_else(|| {
        format!(
            "bad argument #{} to 'pack' (number expected, got {})",
            arg_num,
            val.type_name()
        )
        .into_value(ctx)
        .into()
    })
}

fn take_string<'gc>(
    args: &[Value<'gc>],
    arg_index: &mut usize,
    ctx: Context<'gc>,
) -> Result<crate::String<'gc>, Error<'gc>> {
    let arg_num = *arg_index + 2;
    let val = get_arg(args, arg_index, ctx)?;
    val.into_string(ctx).ok_or_else(|| {
        format!(
            "bad argument #{} to 'pack' (string expected, got {})",
            arg_num,
            val.type_name()
        )
        .into_value(ctx)
        .into()
    })
}

fn get_arg<'gc>(
    args: &[Value<'gc>],
    index: &mut usize,
    ctx: Context<'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    if *index < args.len() {
        let val = args[*index];
        *index += 1;
        Ok(val)
    } else {
        Err(Error::from_value(
            format!("bad argument #{} to 'pack' (value expected)", *index + 2).into_value(ctx),
        ))
    }
}

fn write_bytes_endian(writer: &mut Vec<u8>, le_bytes: &[u8], endianness: Endianness) {
    match endianness {
        Endianness::Little => writer.extend_from_slice(le_bytes),
        Endianness::Big => {
            for &b in le_bytes.iter().rev() {
                writer.push(b);
            }
        }
        Endianness::Native => {
            if cfg!(target_endian = "little") {
                writer.extend_from_slice(le_bytes);
            } else {
                for &b in le_bytes.iter().rev() {
                    writer.push(b);
                }
            }
        }
    }
}

fn write_int(writer: &mut Vec<u8>, value: i64, size: usize, endianness: Endianness) {
    let mut bytes = [0u8; 16];
    let val_le = (value as i128).to_le_bytes();
    bytes[..size].copy_from_slice(&val_le[..size]);
    write_bytes_endian(writer, &bytes[..size], endianness);
}

fn write_uint(writer: &mut Vec<u8>, value: u128, size: usize, endianness: Endianness) {
    let mut bytes = [0u8; 16];
    let val_le = value.to_le_bytes();
    bytes[..size].copy_from_slice(&val_le[..size]);
    write_bytes_endian(writer, &bytes[..size], endianness);
}

fn write_float(writer: &mut Vec<u8>, value: f32, endianness: Endianness) {
    let bytes = match endianness {
        Endianness::Little => value.to_le_bytes(),
        Endianness::Big => value.to_be_bytes(),
        Endianness::Native => value.to_ne_bytes(),
    };
    writer.extend_from_slice(&bytes);
}

fn write_double(writer: &mut Vec<u8>, value: f64, endianness: Endianness) {
    let bytes = match endianness {
        Endianness::Little => value.to_le_bytes(),
        Endianness::Big => value.to_be_bytes(),
        Endianness::Native => value.to_ne_bytes(),
    };
    writer.extend_from_slice(&bytes);
}
