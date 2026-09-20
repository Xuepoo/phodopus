use crate::{Context, Error, IntoValue};

use super::{
    calculate_padding, get_align_size_for_option, get_format_size, parse_number, Endianness,
    FormatState, MAX_STRING_PACK_BYTES,
};

pub fn process<'gc>(fmt: &str, ctx: Context<'gc>) -> Result<usize, Error<'gc>> {
    let mut state = FormatState::default();
    let mut total_size: usize = 0;
    let mut chars = fmt.chars().peekable();

    while let Some(format_char) = chars.next() {
        match format_char {
            '<' => state.endianness = Endianness::Little,
            '>' => state.endianness = Endianness::Big,
            '=' => state.endianness = Endianness::Native,
            '!' => {
                let num_opt = parse_number(&mut chars)
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
                total_size = total_size
                    .checked_add(1)
                    .filter(|&s| s <= MAX_STRING_PACK_BYTES)
                    .ok_or_else(|| {
                        Error::from_value("resulting string too large".into_value(ctx))
                    })?;
            }
            'X' => {
                let op = chars.next().ok_or_else(|| {
                    Error::from_value("'X' must be followed by an option character".into_value(ctx))
                })?;
                let num_opt = if matches!(op, 'i' | 'I' | 's' | 'c') {
                    parse_number(&mut chars)
                        .map_err(|err| Error::from_value(err.into_value(ctx)))?
                } else {
                    None
                };
                let align_size = get_align_size_for_option(op, num_opt)
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?;
                let padding = calculate_padding(total_size, align_size, state.max_alignment);
                total_size = total_size
                    .checked_add(padding)
                    .filter(|&s| s <= MAX_STRING_PACK_BYTES)
                    .ok_or_else(|| {
                        Error::from_value("resulting string too large".into_value(ctx))
                    })?;
            }
            op @ ('b' | 'B' | 'h' | 'H' | 'l' | 'L' | 'j' | 'J' | 'T' | 'f' | 'd' | 'n') => {
                let data_size = get_format_size(op, None).unwrap();
                let padding = calculate_padding(total_size, data_size, state.max_alignment);
                total_size = total_size
                    .checked_add(padding)
                    .and_then(|s| s.checked_add(data_size))
                    .filter(|&s| s <= MAX_STRING_PACK_BYTES)
                    .ok_or_else(|| {
                        Error::from_value("resulting string too large".into_value(ctx))
                    })?;
            }
            'i' | 'I' => {
                let num_opt = parse_number(&mut chars)
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?;
                let size = num_opt.unwrap_or(4);
                if size < 1 || size > 16 {
                    return Err(Error::from_value(
                        format!("integral size {} out of limits [1, 16]", size).into_value(ctx),
                    ));
                }
                let padding = calculate_padding(total_size, size, state.max_alignment);
                total_size = total_size
                    .checked_add(padding)
                    .and_then(|s| s.checked_add(size))
                    .filter(|&s| s <= MAX_STRING_PACK_BYTES)
                    .ok_or_else(|| {
                        Error::from_value("resulting string too large".into_value(ctx))
                    })?;
            }
            'c' => {
                let num_opt = parse_number(&mut chars)
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?;
                let n = num_opt.ok_or_else(|| {
                    Error::from_value("missing size for format option 'c'".into_value(ctx))
                })?;
                total_size = total_size
                    .checked_add(n)
                    .filter(|&s| s <= MAX_STRING_PACK_BYTES)
                    .ok_or_else(|| {
                        Error::from_value("resulting string too large".into_value(ctx))
                    })?;
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
                return Err(Error::from_value(
                    format!("invalid conversion option '{}' in format string", invalid)
                        .into_value(ctx),
                ));
            }
        }
    }

    Ok(total_size)
}
