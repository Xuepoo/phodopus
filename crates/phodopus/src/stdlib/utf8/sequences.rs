//! Resumable UTF-8 scans.
//!
//! Each sequence scans its input one code point at a time, charging Fuel
//! proportional to the bytes examined and returning
//! [`SequencePoll::Pending`](crate::SequencePoll::Pending) as soon as the
//! current Fuel slice runs out. Because code point decoding is a pure function
//! of the remaining byte slice, resumption is exact: the saved byte offset
//! fully describes the work that remains.

use std::pin::Pin;

use gc_arena::Collect;

use crate::{
    BoxSequence, CallbackReturn, Context, Error, Execution, IntoValue, Sequence, SequencePoll,
    Stack, Value,
};

use super::super::sandbox;
use super::{decode_utf8, iscont};

/// `utf8.len(s [, i [, j [, lax]]])`.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct LenSequence {
    #[collect(require_static)]
    bytes: Vec<u8>,
    pos: usize,
    end: usize,
    count: i64,
}

impl LenSequence {
    pub(crate) fn create<'gc>(
        ctx: Context<'gc>,
        bytes: &[u8],
        posi: i64,
        posj: i64,
    ) -> CallbackReturn<'gc> {
        let len = bytes.len();
        let start = (posi - 1) as usize;
        let end = posj as usize;
        debug_assert!(start <= end && end <= len);
        CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            LenSequence {
                bytes: bytes.to_vec(),
                pos: start,
                end,
                count: 0,
            },
        ))
    }
}

impl<'gc> Sequence<'gc> for LenSequence {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let seq = self.as_mut().get_mut();
        let fuel = exec.fuel();

        while seq.pos < seq.end {
            fuel.consume(sandbox::FUEL_PER_SCANNED_BYTE);
            match decode_utf8(&seq.bytes[seq.pos..]) {
                Some((_, char_len)) => {
                    seq.count += 1;
                    seq.pos += char_len;
                }
                None => {
                    stack.replace(ctx, (Value::Nil, (seq.pos as i64) + 1));
                    return Ok(SequencePoll::Return);
                }
            }

            if !fuel.should_continue() {
                return Ok(SequencePoll::Pending);
            }
        }

        stack.replace(ctx, seq.count);
        Ok(SequencePoll::Return)
    }
}

/// `utf8.codepoint(s [, i [, j [, lax]]])`.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct CodepointSequence {
    #[collect(require_static)]
    bytes: Vec<u8>,
    pos: usize,
    end: usize,
}

impl CodepointSequence {
    pub(crate) fn create<'gc>(
        ctx: Context<'gc>,
        bytes: &[u8],
        posi: i64,
        pose: i64,
    ) -> CallbackReturn<'gc> {
        let start = (posi - 1) as usize;
        let end = pose as usize;
        debug_assert!(start <= end && end <= bytes.len());
        CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            CodepointSequence {
                bytes: bytes.to_vec(),
                pos: start,
                end,
            },
        ))
    }
}

impl<'gc> Sequence<'gc> for CodepointSequence {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let seq = self.as_mut().get_mut();
        let fuel = exec.fuel();

        while seq.pos < seq.end {
            fuel.consume(sandbox::FUEL_PER_SCANNED_BYTE);
            let (c, char_len) = match decode_utf8(&seq.bytes[seq.pos..]) {
                Some(res) => res,
                None => return Err("invalid UTF-8 code".into_value(ctx).into()),
            };
            stack.push_back(Value::Integer(c as u32 as i64));
            seq.pos += char_len;

            if !fuel.should_continue() {
                return Ok(SequencePoll::Pending);
            }
        }

        Ok(SequencePoll::Return)
    }
}

/// `utf8.offset(s, n [, i])`.
#[derive(Collect)]
#[collect(no_drop)]
pub(crate) struct OffsetSequence {
    #[collect(require_static)]
    bytes: Vec<u8>,
    pos: usize,
    n: i64,
    // `-1` for a backward scan, `1` for a forward scan, `0` for the `n == 0`
    // code point start search.
    direction: i8,
}

impl OffsetSequence {
    pub(crate) fn create<'gc>(
        ctx: Context<'gc>,
        bytes: &[u8],
        pos: usize,
        n: i64,
        direction: i8,
    ) -> CallbackReturn<'gc> {
        CallbackReturn::Sequence(BoxSequence::new(
            &ctx,
            OffsetSequence {
                bytes: bytes.to_vec(),
                pos,
                n,
                direction,
            },
        ))
    }
}

impl<'gc> Sequence<'gc> for OffsetSequence {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let seq = self.as_mut().get_mut();
        let fuel = exec.fuel();
        let len = seq.bytes.len();

        match seq.direction {
            // `utf8.offset(s, 0, i)`: walk back to the start of the code point.
            0 => {
                while seq.pos > 0 && seq.pos < len && iscont(seq.bytes[seq.pos]) {
                    fuel.consume(sandbox::FUEL_PER_SCANNED_BYTE);
                    seq.pos -= 1;
                    if !fuel.should_continue() {
                        return Ok(SequencePoll::Pending);
                    }
                }
                stack.replace(ctx, (seq.pos as i64) + 1);
                Ok(SequencePoll::Return)
            }
            // Backward scan for `n < 0`.
            -1 => {
                while seq.n < 0 && seq.pos > 0 {
                    fuel.consume(sandbox::FUEL_PER_SCANNED_BYTE);
                    loop {
                        seq.pos -= 1;
                        if seq.pos == 0 || !iscont(seq.bytes[seq.pos]) {
                            break;
                        }
                    }
                    seq.n += 1;
                    if !fuel.should_continue() {
                        return Ok(SequencePoll::Pending);
                    }
                }
                if seq.n == 0 {
                    stack.replace(ctx, (seq.pos as i64) + 1);
                } else {
                    stack.replace(ctx, Value::Nil);
                }
                Ok(SequencePoll::Return)
            }
            // Forward scan for `n > 0`.
            _ => {
                while seq.n > 1 && seq.pos < len {
                    fuel.consume(sandbox::FUEL_PER_SCANNED_BYTE);
                    loop {
                        seq.pos += 1;
                        if seq.pos >= len || !iscont(seq.bytes[seq.pos]) {
                            break;
                        }
                    }
                    seq.n -= 1;
                    if !fuel.should_continue() {
                        return Ok(SequencePoll::Pending);
                    }
                }
                if seq.n == 1 {
                    stack.replace(ctx, (seq.pos as i64) + 1);
                } else {
                    stack.replace(ctx, Value::Nil);
                }
                Ok(SequencePoll::Return)
            }
        }
    }
}
