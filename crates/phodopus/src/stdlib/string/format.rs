use std::pin::Pin;

use fhex::ToHex;
use gc_arena::Collect;
use parser::{
    ConversionSpecifier, ConversionType, FormatElement, NumericParam, parse_format_string,
};
use thiserror::Error;

use crate::{
    BoxSequence, CallbackReturn, Context, Error, Execution, IntoValue, Sequence, SequencePoll,
    Stack, Value,
    meta_ops::{self, MetaResult},
};

use super::super::sandbox::{self, FUEL_PER_FORMAT_DIRECTIVE};

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

/// A resumable implementation of `string.format`.
///
/// The format string is parsed once into owned elements, and each `poll`
/// expands as many directives as the remaining Fuel allows before returning
/// [`SequencePoll::Pending`]. The partially-built output is preserved in the
/// sequence, so replenishing Fuel resumes exactly where the previous poll
/// stopped. Output growth is checked against [`MAX_STDLIB_STRING_BYTES`] before
/// every append, independent of any global heap quota.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct FormatSequence<'gc> {
    #[collect(require_static)]
    elements: Vec<FormatElement>,
    element_index: usize,
    verbatim_offset: usize,
    args: Vec<Value<'gc>>,
    arg_index: usize,
    #[collect(require_static)]
    res: String,
}

impl<'gc> FormatSequence<'gc> {
    pub(crate) fn create(
        ctx: Context<'gc>,
        format_str: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<CallbackReturn<'gc>, Error<'gc>> {
        let elements = parse_format_string(format_str)
            .map_err(|err| Error::from_value(FormatError::to_lua(ctx, err)))?;
        Ok(CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            FormatSequence {
                elements,
                element_index: 0,
                verbatim_offset: 0,
                args,
                arg_index: 0,
                res: String::new(),
            },
        )))
    }
}

impl<'gc> Sequence<'gc> for FormatSequence<'gc> {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let seq = self.as_mut().get_mut();

        // `elements` is borrowed for the whole loop; `seq.res`, `seq.arg_index`,
        // `seq.element_index`, and `seq.verbatim_offset` are disjoint fields.
        let elements = &seq.elements;
        // Safety of disjoint field access: all fields below are distinct.
        let res = &mut seq.res;
        let arg_index = &mut seq.arg_index;
        let element_index = &mut seq.element_index;
        let verbatim_offset = &mut seq.verbatim_offset;
        let args = &seq.args;
        let fuel = exec.fuel();

        while *element_index < elements.len() {
            match &elements[*element_index] {
                FormatElement::Verbatim(text) => {
                    let remaining = &text[*verbatim_offset..];
                    if remaining.is_empty() {
                        *element_index += 1;
                        *verbatim_offset = 0;
                        continue;
                    }

                    let chunk_len = verbatim_chunk_len(
                        remaining,
                        sandbox::work_batch(fuel, sandbox::FUEL_PER_OUTPUT_BYTE),
                    );
                    sandbox::checked_output_growth(res.len(), chunk_len).ok_or_else(|| {
                        Error::from_value("resulting string too large".into_value(ctx))
                    })?;
                    // Charge the projected output buffer against the hard memory quota as well as
                    // the 16 MiB ceiling, matching `string.rep`. Without this, a hostile format
                    // could build a multi-megabyte buffer and only be refused at the execution
                    // boundary once it was interned into the arena.
                    ctx.check_memory(res.len().saturating_add(chunk_len))?;
                    res.push_str(&remaining[..chunk_len]);
                    fuel.consume(sandbox::output_cost(chunk_len));
                    *verbatim_offset += chunk_len;
                }
                FormatElement::Format(spec) => {
                    if spec.conversion_type == ConversionType::PercentSign {
                        if sandbox::checked_output_growth(res.len(), 1).is_none() {
                            return Err(Error::from_value(
                                "resulting string too large".into_value(ctx),
                            ));
                        }
                        ctx.check_memory(res.len().saturating_add(1))?;
                        res.push('%');
                        fuel.consume(FUEL_PER_FORMAT_DIRECTIVE);
                        *element_index += 1;
                        *verbatim_offset = 0;
                        continue;
                    }

                    if *arg_index >= args.len() {
                        return Err(Error::from_value(FormatError::to_lua(
                            ctx,
                            FormatError::NotEnoughArgs,
                        )));
                    }
                    let arg = args[*arg_index];
                    *arg_index += 1;
                    let spec = *spec;

                    let expansion = format_value(ctx, arg, &spec)
                        .map_err(|err| Error::from_value(FormatError::to_lua(ctx, err)))?;
                    if sandbox::checked_output_growth(res.len(), expansion.len()).is_none() {
                        return Err(Error::from_value(
                            "resulting string too large".into_value(ctx),
                        ));
                    }
                    ctx.check_memory(res.len().saturating_add(expansion.len()))?;
                    res.push_str(&expansion);
                    fuel.consume(
                        FUEL_PER_FORMAT_DIRECTIVE
                            .saturating_add(sandbox::output_cost(expansion.len())),
                    );
                    *element_index += 1;
                    *verbatim_offset = 0;
                }
            }

            if !fuel.should_continue() {
                return Ok(SequencePoll::Pending);
            }
        }

        if *arg_index != args.len() {
            return Err(Error::from_value(FormatError::to_lua(
                ctx,
                FormatError::TooManyArgs,
            )));
        }

        let result = ctx.intern(res.as_bytes());
        stack.replace(ctx, result);
        Ok(SequencePoll::Return)
    }
}

/// Returns the length of the largest UTF-8 boundary-respecting prefix of `text`
/// no longer than `budget` bytes. Always at least one character so callers make
/// forward progress.
fn verbatim_chunk_len(text: &str, budget: usize) -> usize {
    if text.len() <= budget {
        return text.len();
    }
    let mut boundary = budget.max(1);
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    if boundary == 0 {
        text.chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(text.len())
    } else {
        boundary
    }
}

impl FormatError {
    fn to_lua<'gc>(ctx: Context<'gc>, err: FormatError) -> crate::Value<'gc> {
        err.to_string().into_value(ctx)
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
