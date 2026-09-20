use fhex::ToHex;
use parser::{
    ConversionSpecifier, ConversionType, FormatElement, NumericParam, parse_format_string,
};
use thiserror::Error;

use crate::{
    Context, Value,
    meta_ops::{self, MetaResult},
};

mod parser;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("Error parsing the format string")]
    ParseError,
    #[error("Incorrect type passed as an argument")]
    WrongType,
    #[error("Too many arguments passed")]
    TooManyArgs,
    #[error("Too few arguments passed")]
    NotEnoughArgs,
    #[error("Other error (should never happen)")]
    Unknown,
}

pub fn format<'gc>(
    ctx: &Context<'gc>,
    format_str: &str,
    args: &[Value<'gc>],
) -> Result<String, FormatError> {
    let format_elements = parse_format_string(format_str)?;
    let mut res = String::new();
    let mut remaining_args = args;

    let mut pop_arg = || {
        if remaining_args.is_empty() {
            Err(FormatError::NotEnoughArgs)
        } else {
            let a = remaining_args[0];
            remaining_args = &remaining_args[1..];
            Ok(a)
        }
    };

    for element in format_elements {
        match element {
            FormatElement::Verbatim(s) => {
                res.push_str(s);
            }
            FormatElement::Format(spec) => {
                if spec.conversion_type == ConversionType::PercentSign {
                    res.push('%');
                } else {
                    let arg = pop_arg()?;
                    res.push_str(&format_value(*ctx, arg, &spec)?);
                }
            }
        }
    }

    if remaining_args.is_empty() {
        Ok(res)
    } else {
        Err(FormatError::TooManyArgs)
    }
}

pub fn format_value<'gc>(
    ctx: Context<'gc>,
    value: Value<'gc>,
    spec: &ConversionSpecifier,
) -> Result<String, FormatError> {
    match spec.conversion_type {
        ConversionType::String => {
            let s = match value {
                Value::Nil => "nil".to_string(),
                Value::Boolean(b) => (if b { "true" } else { "false" }).to_string(),
                Value::Integer(i) => i.to_string(),
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.display_lossy().to_string(),
                Value::Table(_) | Value::Function(_) | Value::Thread(_) | Value::UserData(_) => {
                    match meta_ops::tostring(ctx, value) {
                        Ok(meta_result) => match meta_result {
                            MetaResult::Value(val) => val.display().to_string(),
                            MetaResult::Call(_) => return Err(FormatError::Unknown),
                        },
                        Err(_) => format!("{:p}", value_pointer(&value)),
                    }
                }
            };
            format_string(&s, spec)
        }

        ConversionType::Pointer => {
            let ptr_str = format!("{:p}", value_pointer(&value));
            format_string(&ptr_str, spec)
        }

        ConversionType::DecInt => {
            let i = match value.to_integer() {
                Some(i) => i,
                None => return Err(FormatError::WrongType),
            };
            format_signed_integer(i, spec)
        }

        ConversionType::UnsignedDecInt => {
            let i = match value.to_integer() {
                Some(i) => i,
                None => return Err(FormatError::WrongType),
            };
            format_unsigned_integer(i as u64, 10, false, spec)
        }

        ConversionType::OctInt => {
            let i = match value.to_integer() {
                Some(i) => i,
                None => return Err(FormatError::WrongType),
            };
            format_unsigned_integer(i as u64, 8, false, spec)
        }

        ConversionType::HexIntLower => {
            let i = match value.to_integer() {
                Some(i) => i,
                None => return Err(FormatError::WrongType),
            };
            format_unsigned_integer(i as u64, 16, false, spec)
        }

        ConversionType::HexIntUpper => {
            let i = match value.to_integer() {
                Some(i) => i,
                None => return Err(FormatError::WrongType),
            };
            format_unsigned_integer(i as u64, 16, true, spec)
        }

        ConversionType::DecFloat
        | ConversionType::SciFloatLower
        | ConversionType::SciFloatUpper
        | ConversionType::CompactFloatLower
        | ConversionType::CompactFloatUpper
        | ConversionType::HexFloatLower
        | ConversionType::HexFloatUpper => {
            let f = match value {
                Value::Integer(i) => i as f64,
                Value::Number(n) => n,
                _ => {
                    if let Some(c) = value.to_constant() {
                        if let Some(f) = c.to_number() {
                            f
                        } else {
                            return Err(FormatError::WrongType);
                        }
                    } else {
                        return Err(FormatError::WrongType);
                    }
                }
            };
            format_float(f, spec)
        }

        ConversionType::Char => {
            let i = match value.to_integer() {
                Some(i) => i,
                None => return Err(FormatError::WrongType),
            };
            let c = if (0..=255).contains(&i) {
                (i as u8) as char
            } else if let Some(c) = char::from_u32(i as u32) {
                c
            } else {
                return Err(FormatError::WrongType);
            };
            format_char(c, spec)
        }

        ConversionType::QuotedString => match value {
            Value::Nil => Ok("nil".to_string()),
            Value::Boolean(b) => Ok((if b { "true" } else { "false" }).to_string()),
            Value::Integer(i) => Ok(i.to_string()),
            Value::Number(f) => {
                if f.is_nan() {
                    Ok("(0/0)".to_string())
                } else if f.is_infinite() {
                    if f.is_sign_negative() {
                        Ok("(-1/0)".to_string())
                    } else {
                        Ok("(1/0)".to_string())
                    }
                } else {
                    Ok(f.to_hex())
                }
            }
            Value::String(s) => Ok(format_quoted_bytes(s.as_bytes())),
            _ => Err(FormatError::WrongType),
        },

        ConversionType::PercentSign => Ok("%".to_string()),
    }
}

fn value_pointer<'gc>(value: &Value<'gc>) -> *const () {
    use crate::Function;
    use gc_arena::Gc;

    match value {
        Value::Table(t) => Gc::as_ptr(t.into_inner()) as *const (),
        Value::Function(Function::Closure(c)) => Gc::as_ptr(c.into_inner()) as *const (),
        Value::Function(Function::Callback(c)) => Gc::as_ptr(c.into_inner()) as *const (),
        Value::Thread(t) => Gc::as_ptr(t.into_inner()) as *const (),
        Value::UserData(u) => Gc::as_ptr(u.into_inner()) as *const (),
        Value::String(s) => s.as_bytes().as_ptr() as *const (),
        _ => std::ptr::null(),
    }
}

fn format_signed_integer(value: i64, spec: &ConversionSpecifier) -> Result<String, FormatError> {
    let is_neg = value < 0;
    let sign_prefix = if is_neg {
        "-"
    } else if spec.force_sign {
        "+"
    } else if spec.space_sign {
        " "
    } else {
        ""
    };

    let u = value.unsigned_abs(); // Completely safe against i64::MIN overflow
    let mut digits = if let NumericParam::Literal(0) = spec.precision {
        if u == 0 { String::new() } else { u.to_string() }
    } else {
        u.to_string()
    };

    if let NumericParam::Literal(prec) = spec.precision {
        if digits.len() < prec {
            let pad_count = prec - digits.len();
            let mut padded = String::with_capacity(prec);
            for _ in 0..pad_count {
                padded.push('0');
            }
            padded.push_str(&digits);
            digits = padded;
        }
    }

    let width = spec.width.unwrap_or(0);
    let total_len = sign_prefix.len() + digits.len();

    let result = if spec.left_adj {
        let mut s = String::with_capacity(total_len.max(width));
        s.push_str(sign_prefix);
        s.push_str(&digits);
        while s.len() < width {
            s.push(' ');
        }
        s
    } else if spec.zero_pad && matches!(spec.precision, NumericParam::Unspecified) {
        let mut s = String::with_capacity(total_len.max(width));
        s.push_str(sign_prefix);
        if total_len < width {
            for _ in 0..(width - total_len) {
                s.push('0');
            }
        }
        s.push_str(&digits);
        s
    } else {
        let mut s = String::with_capacity(total_len.max(width));
        if total_len < width {
            for _ in 0..(width - total_len) {
                s.push(' ');
            }
        }
        s.push_str(sign_prefix);
        s.push_str(&digits);
        s
    };

    Ok(result)
}

fn format_unsigned_integer(
    value: u64,
    base: u32,
    uppercase: bool,
    spec: &ConversionSpecifier,
) -> Result<String, FormatError> {
    let mut digits = match base {
        10 => {
            if let NumericParam::Literal(0) = spec.precision {
                if value == 0 {
                    String::new()
                } else {
                    value.to_string()
                }
            } else {
                value.to_string()
            }
        }
        8 => {
            if let NumericParam::Literal(0) = spec.precision {
                if value == 0 {
                    String::new()
                } else {
                    format!("{value:o}")
                }
            } else {
                format!("{value:o}")
            }
        }
        16 => {
            if let NumericParam::Literal(0) = spec.precision {
                if value == 0 {
                    String::new()
                } else if uppercase {
                    format!("{value:X}")
                } else {
                    format!("{value:x}")
                }
            } else if uppercase {
                format!("{value:X}")
            } else {
                format!("{value:x}")
            }
        }
        _ => return Err(FormatError::Unknown),
    };

    if let NumericParam::Literal(prec) = spec.precision {
        if digits.len() < prec {
            let pad_count = prec - digits.len();
            let mut padded = String::with_capacity(prec);
            for _ in 0..pad_count {
                padded.push('0');
            }
            padded.push_str(&digits);
            digits = padded;
        }
    }

    let prefix = if spec.alt_form {
        match base {
            8 => {
                if !digits.starts_with('0') {
                    "0"
                } else {
                    ""
                }
            }
            16 => {
                if value != 0 {
                    if uppercase { "0X" } else { "0x" }
                } else {
                    ""
                }
            }
            _ => "",
        }
    } else {
        ""
    };

    let width = spec.width.unwrap_or(0);
    let total_len = prefix.len() + digits.len();

    let result = if spec.left_adj {
        let mut s = String::with_capacity(total_len.max(width));
        s.push_str(prefix);
        s.push_str(&digits);
        while s.len() < width {
            s.push(' ');
        }
        s
    } else if spec.zero_pad && matches!(spec.precision, NumericParam::Unspecified) {
        let mut s = String::with_capacity(total_len.max(width));
        s.push_str(prefix);
        if total_len < width {
            for _ in 0..(width - total_len) {
                s.push('0');
            }
        }
        s.push_str(&digits);
        s
    } else {
        let mut s = String::with_capacity(total_len.max(width));
        if total_len < width {
            for _ in 0..(width - total_len) {
                s.push(' ');
            }
        }
        s.push_str(prefix);
        s.push_str(&digits);
        s
    };

    Ok(result)
}

fn format_float(value: f64, spec: &ConversionSpecifier) -> Result<String, FormatError> {
    let mut prefix = String::new();
    let is_negative = value.is_sign_negative();

    if is_negative {
        prefix.push('-');
    } else if spec.force_sign {
        prefix.push('+');
    } else if spec.space_sign {
        prefix.push(' ');
    }

    let is_upper = matches!(
        spec.conversion_type,
        ConversionType::SciFloatUpper
            | ConversionType::CompactFloatUpper
            | ConversionType::HexFloatUpper
    );

    let (number, can_zero_pad) = if !value.is_finite() {
        let name = if value.is_nan() {
            if is_upper { "NAN" } else { "nan" }
        } else if is_upper {
            "INF"
        } else {
            "inf"
        };
        (name.to_string(), false)
    } else {
        let abs = value.abs();
        let prec = spec.precision.unwrap_or(6);

        let s = match spec.conversion_type {
            ConversionType::DecFloat => {
                let mut num = format!("{:.prec$}", abs);
                if prec == 0 && spec.alt_form {
                    num.push('.');
                }
                num
            }
            ConversionType::SciFloatLower | ConversionType::SciFloatUpper => {
                let raw = format!("{:.prec$e}", abs);
                let (mantissa, exp_str) = raw.split_once('e').unwrap();
                let exp: i32 = exp_str.parse().unwrap();
                let exp_sym = if is_upper { 'E' } else { 'e' };
                if prec == 0 && spec.alt_form {
                    format!("{mantissa}.{exp_sym}{exp:+03}")
                } else {
                    format!("{mantissa}{exp_sym}{exp:+03}")
                }
            }
            ConversionType::CompactFloatLower | ConversionType::CompactFloatUpper => {
                let p = if prec == 0 { 1 } else { prec };
                let raw = format!("{:.prec$e}", abs, prec = p.saturating_sub(1));
                let (mantissa, exp_str) = raw.split_once('e').unwrap();
                let exp: i32 = exp_str.parse().unwrap();
                let exp_sym = if is_upper { 'E' } else { 'e' };

                if -4 <= exp && exp < (p as i32) {
                    let f_prec = (p as i32 - 1 - exp) as usize;
                    let mut num = format!("{:.f_prec$}", abs);
                    if !spec.alt_form {
                        if num.contains('.') {
                            num = num.trim_end_matches('0').to_string();
                            if num.ends_with('.') {
                                num.pop();
                            }
                        }
                    } else if !num.contains('.') {
                        num.push('.');
                    }
                    num
                } else {
                    let mut mant = mantissa.to_string();
                    if !spec.alt_form {
                        if mant.contains('.') {
                            mant = mant.trim_end_matches('0').to_string();
                            if mant.ends_with('.') {
                                mant.pop();
                            }
                        }
                    } else if !mant.contains('.') {
                        mant.push('.');
                    }
                    format!("{mant}{exp_sym}{exp:+03}")
                }
            }
            ConversionType::HexFloatLower | ConversionType::HexFloatUpper => {
                let mut hex = abs.to_hex();
                if is_upper {
                    hex = hex.to_uppercase();
                }
                if let NumericParam::Literal(p) = spec.precision {
                    let exp_char = if is_upper { 'P' } else { 'p' };
                    if let Some((sig, exp)) = hex.split_once(exp_char) {
                        let mut sig_parts = sig.split('.');
                        let int_part = sig_parts.next().unwrap_or(sig);
                        let frac_part = sig_parts.next().unwrap_or("");
                        let mut new_frac = frac_part.to_string();
                        if new_frac.len() < p {
                            while new_frac.len() < p {
                                new_frac.push('0');
                            }
                        } else if new_frac.len() > p {
                            new_frac.truncate(p);
                        }
                        if p == 0 {
                            if spec.alt_form {
                                hex = format!("{int_part}.{exp_char}{exp}");
                            } else {
                                hex = format!("{int_part}{exp_char}{exp}");
                            }
                        } else {
                            hex = format!("{int_part}.{new_frac}{exp_char}{exp}");
                        }
                    }
                }
                hex
            }
            _ => return Err(FormatError::WrongType),
        };
        (s, true)
    };

    let width = spec.width.unwrap_or(0);
    let total_len = prefix.len() + number.len();

    let result = if spec.left_adj {
        let mut s = String::with_capacity(total_len.max(width));
        s.push_str(&prefix);
        s.push_str(&number);
        while s.len() < width {
            s.push(' ');
        }
        s
    } else if spec.zero_pad && can_zero_pad {
        let mut s = String::with_capacity(total_len.max(width));
        s.push_str(&prefix);
        if total_len < width {
            for _ in 0..(width - total_len) {
                s.push('0');
            }
        }
        s.push_str(&number);
        s
    } else {
        let mut s = String::with_capacity(total_len.max(width));
        if total_len < width {
            for _ in 0..(width - total_len) {
                s.push(' ');
            }
        }
        s.push_str(&prefix);
        s.push_str(&number);
        s
    };

    Ok(result)
}

fn format_string(input: &str, spec: &ConversionSpecifier) -> Result<String, FormatError> {
    let content = if let NumericParam::Literal(prec) = spec.precision {
        if input.len() > prec {
            let mut boundary = prec;
            while boundary > 0 && !input.is_char_boundary(boundary) {
                boundary -= 1;
            }
            &input[..boundary]
        } else {
            input
        }
    } else {
        input
    };

    let width = spec.width.unwrap_or(0);
    let result = if spec.left_adj {
        let mut s = String::with_capacity(content.len().max(width));
        s.push_str(content);
        while s.len() < width {
            s.push(' ');
        }
        s
    } else {
        let mut s = String::with_capacity(content.len().max(width));
        if content.len() < width {
            for _ in 0..(width - content.len()) {
                s.push(' ');
            }
        }
        s.push_str(content);
        s
    };

    Ok(result)
}

fn format_char(c: char, spec: &ConversionSpecifier) -> Result<String, FormatError> {
    let width = spec.width.unwrap_or(0);
    let char_len = c.len_utf8();

    let result = if spec.left_adj {
        let mut s = String::with_capacity(char_len.max(width));
        s.push(c);
        while s.len() < width {
            s.push(' ');
        }
        s
    } else {
        let mut s = String::with_capacity(char_len.max(width));
        if char_len < width {
            for _ in 0..(width - char_len) {
                s.push(' ');
            }
        }
        s.push(c);
        s
    };

    Ok(result)
}

fn format_quoted_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut quoted = String::with_capacity(bytes.len() + 2);
    quoted.push('"');
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            b'\n' => quoted.push_str("\\n"),
            b'\r' => quoted.push_str("\\r"),
            b'\0' => {
                if i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                    quoted.push_str("\\000");
                } else {
                    quoted.push_str("\\0");
                }
            }
            b if (1..=31).contains(&b) || b == 127 => {
                if i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                    let _ = write!(quoted, "\\{:03}", b);
                } else {
                    let _ = write!(quoted, "\\{}", b);
                }
            }
            32..=126 => {
                quoted.push(b as char);
            }
            _ => {
                if let Ok(s) = std::str::from_utf8(&bytes[i..]) {
                    let c = s.chars().next().unwrap();
                    quoted.push(c);
                    i += c.len_utf8();
                    continue;
                } else if i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                    let _ = write!(quoted, "\\{:03}", b);
                } else {
                    let _ = write!(quoted, "\\{}", b);
                }
            }
        }
        i += 1;
    }
    quoted.push('"');
    quoted
}
