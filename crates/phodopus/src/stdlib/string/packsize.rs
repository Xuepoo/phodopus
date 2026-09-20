use std::pin::Pin;

use gc_arena::Collect;

use crate::{
    BoxSequence, CallbackReturn, Context, Error, Execution, IntoValue, Sequence, SequencePoll,
    Stack,
};

use super::super::sandbox::FUEL_PER_FORMAT_BYTE;
use super::{
    Endianness, FormatCursor, FormatState, MAX_STRING_PACK_BYTES, calculate_padding,
    get_align_size_for_option, get_format_size,
};

/// A resumable implementation of `string.packsize`.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct PacksizeSequence {
    fmt: FormatCursor,
    state: FormatState,
    total_size: usize,
}

impl PacksizeSequence {
    pub(crate) fn create<'gc>(ctx: Context<'gc>, fmt: &str) -> CallbackReturn<'gc> {
        CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            PacksizeSequence {
                fmt: FormatCursor::create(fmt),
                state: FormatState::default(),
                total_size: 0,
            },
        ))
    }
}

impl<'gc> Sequence<'gc> for PacksizeSequence {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let seq = self.as_mut().get_mut();
        let fuel = exec.fuel();

        while let Some(format_char) = seq.fmt.next_char() {
            fuel.consume(FUEL_PER_FORMAT_BYTE);
            process_option(seq, ctx, format_char)?;

            if !fuel.should_continue() && !seq.fmt.is_done() {
                return Ok(SequencePoll::Pending);
            }
        }

        stack.replace(ctx, seq.total_size as i64);
        Ok(SequencePoll::Return)
    }
}

fn process_option<'gc>(
    seq: &mut PacksizeSequence,
    ctx: Context<'gc>,
    format_char: char,
) -> Result<(), Error<'gc>> {
    let state = &mut seq.state;
    let total_size = &mut seq.total_size;
    let fmt = &mut seq.fmt;

    let add = |ctx: Context<'gc>, total_size: &mut usize, amount: usize| {
        *total_size = total_size
            .checked_add(amount)
            .filter(|&s| s <= MAX_STRING_PACK_BYTES)
            .ok_or_else(|| Error::from_value("resulting string too large".into_value(ctx)))?;
        Ok::<(), Error<'gc>>(())
    };

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
        'x' => add(ctx, total_size, 1)?,
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
            let padding = calculate_padding(*total_size, align_size, state.max_alignment);
            add(ctx, total_size, padding)?;
        }
        op @ ('b' | 'B' | 'h' | 'H' | 'l' | 'L' | 'j' | 'J' | 'T' | 'f' | 'd' | 'n') => {
            let data_size = get_format_size(op, None).unwrap();
            let padding = calculate_padding(*total_size, data_size, state.max_alignment);
            add(ctx, total_size, padding)?;
            add(ctx, total_size, data_size)?;
        }
        'i' | 'I' => {
            let size = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .unwrap_or(4);
            if size < 1 || size > 16 {
                return Err(format!("integral size {} out of limits [1, 16]", size)
                    .into_value(ctx)
                    .into());
            }
            let padding = calculate_padding(*total_size, size, state.max_alignment);
            add(ctx, total_size, padding)?;
            add(ctx, total_size, size)?;
        }
        'c' => {
            let n = fmt
                .parse_number()
                .map_err(|err| Error::from_value(err.into_value(ctx)))?
                .ok_or_else(|| {
                    Error::from_value("missing size for format option 'c'".into_value(ctx))
                })?;
            add(ctx, total_size, n)?;
        }
        'z' => {
            return Err(Error::from_value(
                "variable-length format ('z')".into_value(ctx),
            ));
        }
        's' => {
            return Err(Error::from_value(
                "variable-length format ('s')".into_value(ctx),
            ));
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
