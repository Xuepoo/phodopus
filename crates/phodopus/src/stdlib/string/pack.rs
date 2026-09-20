use crate::{Context, Error, IntoValue, Value};

use super::{
    calculate_padding, get_align_size_for_option, parse_number, Endianness, FormatState,
    MAX_STRING_PACK_BYTES,
};

pub fn process<'gc>(
    fmt: &str,
    ctx: Context<'gc>,
    args: &[Value<'gc>],
) -> Result<Vec<u8>, Error<'gc>> {
    let mut state = FormatState::default();
    let mut writer = Vec::new();
    let mut current_argument_index = 0;
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
                if writer.len() + 1 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.push(0);
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
                let padding = calculate_padding(writer.len(), align_size, state.max_alignment);
                if writer.len() + padding > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
            }
            'b' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                if val < i8::MIN as i64 || val > i8::MAX as i64 {
                    return Err(Error::from_value(
                        format!("bad argument #{} to 'pack' (integer overflow)", arg_num)
                            .into_value(ctx),
                    ));
                }
                let padding = calculate_padding(writer.len(), 1, state.max_alignment);
                if writer.len() + padding + 1 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                writer.push(val as u8);
            }
            'B' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                if val < 0 || val > u8::MAX as i64 {
                    return Err(Error::from_value(
                        format!("bad argument #{} to 'pack' (unsigned overflow)", arg_num)
                            .into_value(ctx),
                    ));
                }
                let padding = calculate_padding(writer.len(), 1, state.max_alignment);
                if writer.len() + padding + 1 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                writer.push(val as u8);
            }
            'h' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                if val < i16::MIN as i64 || val > i16::MAX as i64 {
                    return Err(Error::from_value(
                        format!("bad argument #{} to 'pack' (integer overflow)", arg_num)
                            .into_value(ctx),
                    ));
                }
                let padding = calculate_padding(writer.len(), 2, state.max_alignment);
                if writer.len() + padding + 2 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_int(&mut writer, val, 2, state.endianness);
            }
            'H' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                if val < 0 || val > u16::MAX as i64 {
                    return Err(Error::from_value(
                        format!("bad argument #{} to 'pack' (unsigned overflow)", arg_num)
                            .into_value(ctx),
                    ));
                }
                let padding = calculate_padding(writer.len(), 2, state.max_alignment);
                if writer.len() + padding + 2 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_uint(&mut writer, val as u64 as u128, 2, state.endianness);
            }
            'l' | 'j' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                let padding = calculate_padding(writer.len(), 8, state.max_alignment);
                if writer.len() + padding + 8 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_int(&mut writer, val, 8, state.endianness);
            }
            'L' | 'J' | 'T' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                let padding = calculate_padding(writer.len(), 8, state.max_alignment);
                if writer.len() + padding + 8 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_uint(&mut writer, val as u64 as u128, 8, state.endianness);
            }
            'i' => {
                let num_opt = parse_number(&mut chars)
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?;
                let size = num_opt.unwrap_or(4);
                if size < 1 || size > 16 {
                    return Err(Error::from_value(
                        format!("integral size {} out of limits [1, 16]", size).into_value(ctx),
                    ));
                }
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                if size < 8 {
                    let min = -(1i64 << (size * 8 - 1));
                    let max = (1i64 << (size * 8 - 1)) - 1;
                    if val < min || val > max {
                        return Err(Error::from_value(
                            format!("bad argument #{} to 'pack' (integer overflow)", arg_num)
                                .into_value(ctx),
                        ));
                    }
                }
                let padding = calculate_padding(writer.len(), size, state.max_alignment);
                if writer.len() + padding + size > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_int(&mut writer, val, size, state.endianness);
            }
            'I' => {
                let num_opt = parse_number(&mut chars)
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?;
                let size = num_opt.unwrap_or(4);
                if size < 1 || size > 16 {
                    return Err(Error::from_value(
                        format!("integral size {} out of limits [1, 16]", size).into_value(ctx),
                    ));
                }
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_integer().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number has no integer representation)",
                            arg_num
                        )
                        .into_value(ctx),
                    )
                })?;
                if size < 8 {
                    let max = (1i64 << (size * 8)) - 1;
                    if val < 0 || val > max {
                        return Err(Error::from_value(
                            format!("bad argument #{} to 'pack' (unsigned overflow)", arg_num)
                                .into_value(ctx),
                        ));
                    }
                } else if size > 8 && val < 0 {
                    return Err(Error::from_value(
                        format!("bad argument #{} to 'pack' (unsigned overflow)", arg_num)
                            .into_value(ctx),
                    ));
                }
                let padding = calculate_padding(writer.len(), size, state.max_alignment);
                if writer.len() + padding + size > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_uint(&mut writer, val as u64 as u128, size, state.endianness);
            }
            'f' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_number().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number expected, got {})",
                            arg_num,
                            arg_val.type_name()
                        )
                        .into_value(ctx),
                    )
                })?;
                let padding = calculate_padding(writer.len(), 4, state.max_alignment);
                if writer.len() + padding + 4 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_float(&mut writer, val as f32, state.endianness);
            }
            'd' | 'n' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let val = arg_val.to_number().ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (number expected, got {})",
                            arg_num,
                            arg_val.type_name()
                        )
                        .into_value(ctx),
                    )
                })?;
                let padding = calculate_padding(writer.len(), 8, state.max_alignment);
                if writer.len() + padding + 8 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_double(&mut writer, val, state.endianness);
            }
            'c' => {
                let num_opt = parse_number(&mut chars)
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?;
                let n = num_opt.ok_or_else(|| {
                    Error::from_value("missing size for format option 'c'".into_value(ctx))
                })?;
                if n > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let s = arg_val.into_string(ctx).ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (string expected, got {})",
                            arg_num,
                            arg_val.type_name()
                        )
                        .into_value(ctx),
                    )
                })?;
                let bytes = s.as_bytes();
                if bytes.len() > n {
                    return Err(Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (string longer than given size)",
                            arg_num
                        )
                        .into_value(ctx),
                    ));
                }
                if writer.len() + n > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.extend_from_slice(bytes);
                writer.resize(writer.len() + (n - bytes.len()), 0);
            }
            'z' => {
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let s = arg_val.into_string(ctx).ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (string expected, got {})",
                            arg_num,
                            arg_val.type_name()
                        )
                        .into_value(ctx),
                    )
                })?;
                let bytes = s.as_bytes();
                if bytes.contains(&0) {
                    return Err(Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (string contains zeros)",
                            arg_num
                        )
                        .into_value(ctx),
                    ));
                }
                if writer.len() + bytes.len() + 1 > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.extend_from_slice(bytes);
                writer.push(0);
            }
            's' => {
                let num_opt = parse_number(&mut chars)
                    .map_err(|err| Error::from_value(err.into_value(ctx)))?;
                let len_size = num_opt.unwrap_or(std::mem::size_of::<usize>());
                if len_size < 1 || len_size > 16 {
                    return Err(Error::from_value(
                        format!("integral size {} out of limits [1, 16]", len_size).into_value(ctx),
                    ));
                }
                let (arg_val, arg_num) = get_arg(args, &mut current_argument_index, ctx)?;
                let s = arg_val.into_string(ctx).ok_or_else(|| {
                    Error::from_value(
                        format!(
                            "bad argument #{} to 'pack' (string expected, got {})",
                            arg_num,
                            arg_val.type_name()
                        )
                        .into_value(ctx),
                    )
                })?;
                let bytes = s.as_bytes();
                let max_len = if len_size >= 16 {
                    u128::MAX
                } else {
                    (1u128 << (len_size * 8)) - 1
                };
                if (bytes.len() as u128) > max_len {
                    return Err(Error::from_value(
                        "string length does not result in a valid integer".into_value(ctx),
                    ));
                }
                let padding = calculate_padding(writer.len(), len_size, state.max_alignment);
                if writer.len() + padding + len_size + bytes.len() > MAX_STRING_PACK_BYTES {
                    return Err(Error::from_value(
                        "resulting string too large".into_value(ctx),
                    ));
                }
                writer.resize(writer.len() + padding, 0);
                write_uint(&mut writer, bytes.len() as u128, len_size, state.endianness);
                writer.extend_from_slice(bytes);
            }
            invalid => {
                return Err(Error::from_value(
                    format!("invalid conversion option '{}' in format string", invalid)
                        .into_value(ctx),
                ));
            }
        }
    }

    Ok(writer)
}

fn get_arg<'gc>(
    args: &[Value<'gc>],
    index: &mut usize,
    ctx: Context<'gc>,
) -> Result<(Value<'gc>, usize), Error<'gc>> {
    let arg_num = *index + 2;
    if *index < args.len() {
        let val = args[*index];
        *index += 1;
        Ok((val, arg_num))
    } else {
        Err(Error::from_value(
            format!("bad argument #{} to 'pack' (value expected)", arg_num).into_value(ctx),
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
