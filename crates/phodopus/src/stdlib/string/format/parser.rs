use super::FormatError;

pub const MAX_WIDTH: usize = 1000;
pub const MAX_PRECISION: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormatElement {
    /// Some characters that are copied to the output as-is
    Verbatim(String),
    /// A format specifier
    Format(ConversionSpecifier),
}

/// Width / precision parameter
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericParam {
    Unspecified,
    Literal(usize),
}

impl NumericParam {
    pub fn unwrap_or(self, default: usize) -> usize {
        match self {
            NumericParam::Unspecified => default,
            NumericParam::Literal(val) => val,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversionSpecifier {
    /// flag `#`: alternate form
    pub alt_form: bool,
    /// flag `0`: left-pad with zeros
    pub zero_pad: bool,
    /// flag `-`: left-adjust
    pub left_adj: bool,
    /// flag `' '`: indicate sign with a space
    pub space_sign: bool,
    /// flag `+`: always show sign
    pub force_sign: bool,
    /// field width
    pub width: NumericParam,
    /// field precision
    pub precision: NumericParam,
    /// conversion data type
    pub conversion_type: ConversionType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionType {
    /// `d`, `i`
    DecInt,
    /// `u`
    UnsignedDecInt,
    /// `o`
    OctInt,
    /// `x`
    HexIntLower,
    /// `X`
    HexIntUpper,
    /// `e`
    SciFloatLower,
    /// `E`
    SciFloatUpper,
    /// `f`
    DecFloat,
    /// `g`
    CompactFloatLower,
    /// `G`
    CompactFloatUpper,
    /// `a`
    HexFloatLower,
    /// `A`
    HexFloatUpper,
    /// `c`
    Char,
    /// `s`
    String,
    /// `q`
    QuotedString,
    /// `p`
    Pointer,
    /// `%`
    PercentSign,
}

pub(crate) fn parse_format_string(fmt: &str) -> Result<Vec<FormatElement>, FormatError> {
    let mut res = Vec::new();
    let mut rem = fmt;

    while !rem.is_empty() {
        if let Some((verbatim_prefix, rest)) = rem.split_once('%') {
            if !verbatim_prefix.is_empty() {
                res.push(FormatElement::Verbatim(verbatim_prefix.to_string()));
            }
            let (spec, rest) = take_conversion_specifier(rest)?;
            res.push(FormatElement::Format(spec));
            rem = rest;
        } else {
            res.push(FormatElement::Verbatim(rem.to_string()));
            break;
        }
    }

    Ok(res)
}

fn take_conversion_specifier(s: &str) -> Result<(ConversionSpecifier, &str), FormatError> {
    if s.is_empty() {
        return Err(FormatError::ParseError);
    }

    // Fast-path literal '%%'
    if s.starts_with('%') {
        return Ok((
            ConversionSpecifier {
                alt_form: false,
                zero_pad: false,
                left_adj: false,
                space_sign: false,
                force_sign: false,
                width: NumericParam::Unspecified,
                precision: NumericParam::Unspecified,
                conversion_type: ConversionType::PercentSign,
            },
            &s[1..],
        ));
    }

    let mut spec = ConversionSpecifier {
        alt_form: false,
        zero_pad: false,
        left_adj: false,
        space_sign: false,
        force_sign: false,
        width: NumericParam::Unspecified,
        precision: NumericParam::Unspecified,
        conversion_type: ConversionType::DecInt,
    };

    let mut s = s;

    // Parse flags in any order
    loop {
        match s.chars().next() {
            Some('#') => spec.alt_form = true,
            Some('0') => spec.zero_pad = true,
            Some('-') => spec.left_adj = true,
            Some(' ') => spec.space_sign = true,
            Some('+') => spec.force_sign = true,
            _ => break,
        }
        s = &s[1..];
    }

    // Field width
    let (w, rest) = take_numeric_param(s)?;
    if let Some(w) = w {
        if w > MAX_WIDTH {
            return Err(FormatError::ParseError);
        }
        spec.width = NumericParam::Literal(w);
    }
    s = rest;

    // Field precision
    if s.starts_with('.') {
        s = &s[1..];
        let (p, rest) = take_numeric_param(s)?;
        let p = p.unwrap_or(0);
        if p > MAX_PRECISION {
            return Err(FormatError::ParseError);
        }
        spec.precision = NumericParam::Literal(p);
        s = rest;
    }

    let spec_char = s.chars().next().ok_or(FormatError::ParseError)?;
    s = &s[spec_char.len_utf8()..];

    spec.conversion_type = match spec_char {
        'i' | 'd' => ConversionType::DecInt,
        'u' => ConversionType::UnsignedDecInt,
        'o' => ConversionType::OctInt,
        'x' => ConversionType::HexIntLower,
        'X' => ConversionType::HexIntUpper,
        'e' => ConversionType::SciFloatLower,
        'E' => ConversionType::SciFloatUpper,
        'f' => ConversionType::DecFloat,
        'g' => ConversionType::CompactFloatLower,
        'G' => ConversionType::CompactFloatUpper,
        'c' => ConversionType::Char,
        's' => ConversionType::String,
        'a' => ConversionType::HexFloatLower,
        'A' => ConversionType::HexFloatUpper,
        'q' => {
            if spec.alt_form
                || spec.zero_pad
                || spec.left_adj
                || spec.space_sign
                || spec.force_sign
                || matches!(spec.width, NumericParam::Literal(_))
                || matches!(spec.precision, NumericParam::Literal(_))
            {
                return Err(FormatError::ParseError);
            }
            ConversionType::QuotedString
        }
        'p' => ConversionType::Pointer,
        _ => return Err(FormatError::ParseError),
    };

    Ok((spec, s))
}

fn take_numeric_param(mut s: &str) -> Result<(Option<usize>, &str), FormatError> {
    let mut val: usize = 0;
    let mut has_digits = false;
    while let Some(c) = s.chars().next() {
        if let Some(d) = c.to_digit(10) {
            has_digits = true;
            val = val
                .checked_mul(10)
                .and_then(|v| v.checked_add(d as usize))
                .ok_or(FormatError::ParseError)?;
            if val > MAX_WIDTH.max(MAX_PRECISION) {
                return Err(FormatError::ParseError);
            }
            s = &s[c.len_utf8()..];
        } else {
            break;
        }
    }

    if has_digits {
        Ok((Some(val), s))
    } else {
        Ok((None, s))
    }
}
