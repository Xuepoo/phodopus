use std::pin::Pin;

use gc_arena::{Collect, Gc};

use crate::fuel::count_fuel;
use crate::{
    BoxSequence, Callback, CallbackReturn, Closure, Context, Error, Execution, Function, IntoValue,
    Sequence, SequencePoll, Stack, String, Table, TypeError, Value,
};

const LOAD_BYTES_PER_FUEL: i32 = 32;
const MAX_CHUNK_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

#[derive(Collect, Copy, Clone, PartialEq, Eq)]
#[collect(require_static)]
enum LoadMode {
    Text,
    Binary,
    BinaryOrText,
}

struct LoadInfo<'gc> {
    chunk: String<'gc>,
    name: Option<String<'gc>>,
    mode: Option<LoadMode>,
    env: Option<Table<'gc>>,
}

/// Load the parts of the stdlib that allow loading new code at runtime
/// from text source code (not bytecode).
pub fn load_load_text<'gc>(ctx: Context<'gc>) {
    ctx.set_global(
        "load",
        load_wrapper(ctx, |ctx, info, mut exec| {
            let mode = info.mode.unwrap_or(LoadMode::BinaryOrText);
            let env = info.env.unwrap_or_else(|| ctx.globals());
            let name = match info.name {
                Some(name) => format!("{}", name.display_lossy()),
                None => "=(load)".into(),
            };

            if matches!(mode, LoadMode::Binary) {
                return Err("attempt to load a binary chunk (mode is 't')"
                    .into_value(ctx)
                    .into());
            }

            let source = info.chunk.as_bytes();
            if source.starts_with(b"\x1b") {
                return Err("attempt to load a binary chunk (mode is 't')"
                    .into_value(ctx)
                    .into());
            }

            if source.len() > MAX_CHUNK_SIZE {
                return Err("chunk too large".into_value(ctx).into());
            }

            exec.fuel()
                .consume(count_fuel(LOAD_BYTES_PER_FUEL, source.len()));

            let closure = Closure::load_with_env(ctx, Some(&*name), source, env)?;
            Ok(closure.into())
        }),
    );
}

/// An implementation of the argument handling logic for `load` to simplify
/// custom load variants.
///
/// This implements the argument handling required for a spec-compliant load
/// implementation, and then calls the provided function with the processed
/// arguments (`LoadInfo`). The callback should return either a `Function` or
/// an error, which this will convert to the format expected by `load`.
fn load_wrapper<'gc, F>(ctx: Context<'gc>, load_callback: F) -> Callback<'gc>
where
    F: Fn(Context<'gc>, LoadInfo<'gc>, Execution<'gc, '_>) -> Result<Function<'gc>, Error<'gc>>
        + 'static,
{
    let load_callback = Gc::new_static(&ctx, load_callback);

    Callback::from_fn_with(&ctx, load_callback, |&load_callback, ctx, _, mut stack| {
        let (chunk, name, mode, env): (Value, Option<String>, Option<String>, Option<Table>) =
            stack.consume(ctx)?;

        let mode = match mode.as_deref() {
            Some(b"t") => Some(LoadMode::Text),
            Some(b"b") => Some(LoadMode::Binary),
            Some(b"bt") => Some(LoadMode::BinaryOrText),
            Some(_) => {
                stack.replace(ctx, (Value::Nil, "invalid mode"));
                return Ok(CallbackReturn::Return);
            }
            None => None,
        };

        if mode == Some(LoadMode::Binary) {
            stack.replace(
                ctx,
                (Value::Nil, "attempt to load a binary chunk (mode is 't')"),
            );
            return Ok(CallbackReturn::Return);
        }

        let root = (name, mode, env, load_callback);
        let inner = Callback::from_fn_with(&ctx, root, |&root, ctx, exec, mut stack| {
            let (name, mode, env, load_callback) = root;
            let chunk: String = stack.consume(ctx)?;
            let info = LoadInfo {
                chunk,
                name,
                mode,
                env,
            };
            match load_callback(ctx, info, exec) {
                Ok(func) => stack.push_back(Value::Function(func)),
                Err(Error::Lua(err)) => stack.replace(ctx, (Value::Nil, err.value)),
                Err(Error::Runtime(err)) => {
                    stack.replace(ctx, (Value::Nil, format!("{:#}", err.error)))
                }
            }
            Ok(CallbackReturn::Return)
        });
        let inner: Function = inner.into();

        match chunk {
            Value::String(_) => {
                stack.push_back(chunk);
                Ok(CallbackReturn::Call {
                    function: inner,
                    then: None,
                })
            }
            Value::Function(func) => Ok(CallbackReturn::Sequence(BoxSequence::new(
                &ctx,
                BuildLoadString {
                    step: 0,
                    total_len: 0,
                    func,
                    then: inner,
                },
            ))),
            _ => Err(TypeError {
                expected: "string or function",
                found: chunk.type_name(),
            }
            .into()),
        }
    })
}

#[derive(Collect)]
#[collect(no_drop)]
struct BuildLoadString<'gc> {
    step: usize,
    total_len: usize,
    func: Function<'gc>,
    then: Function<'gc>,
}

impl BuildLoadString<'_> {
    fn finalize<'gc>(&self, ctx: Context<'gc>, stack: &mut Stack<'gc, '_>) -> String<'gc> {
        let mut bytes = Vec::with_capacity(self.total_len);
        for value in stack.drain(..) {
            let Value::String(s) = value else {
                unreachable!() // guaranteed by the BuildLoadString sequence
            };
            bytes.extend(s.as_bytes());
        }
        String::from_slice(&ctx, &bytes)
    }
}

impl<'gc> Sequence<'gc> for BuildLoadString<'gc> {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        stack.resize(self.step);

        if self.step != 0 {
            let done = match stack.get_mut(self.step - 1) {
                None | Some(Value::Nil) => true,
                Some(v) => {
                    // PRLua implicitly converts integer/number values to strings in load
                    let Some(s) = v.into_string(ctx) else {
                        let error = format!(
                            "error loading string: expected string, found {}",
                            v.type_name()
                        );
                        stack.replace(ctx, (Value::Nil, error));
                        return Ok(SequencePoll::Return);
                    };
                    *v = Value::String(s);
                    // Assembling one piece copies its bytes; charge the copy
                    // proportionally. Total length is already capped by
                    // `MAX_CHUNK_SIZE`.
                    exec.fuel().consume(count_fuel(1, s.len() as usize));
                    self.total_len += s.len() as usize;
                    if self.total_len > MAX_CHUNK_SIZE {
                        stack.replace(ctx, (Value::Nil, "chunk too large"));
                        return Ok(SequencePoll::Return);
                    }
                    s.is_empty()
                }
            };
            if done {
                // The last arg was nil or an empty string, so the load
                // function is done.
                stack.pop_back();
                let str = self.finalize(ctx, &mut stack);
                exec.fuel().consume(count_fuel(1, str.len() as usize));
                stack.push_back(Value::String(str));
                return Ok(SequencePoll::TailCall(self.then));
            }
        }

        let bottom = self.step;
        self.step += 1;
        Ok(SequencePoll::Call {
            function: self.func,
            bottom,
        })
    }

    fn error(
        self: Pin<&mut Self>,
        ctx: Context<'gc>,
        _exec: Execution<'gc, '_>,
        error: Error<'gc>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        // This catches errors thrown by the inner function;
        // PUC-Rio's tests require it, but it's not documented.
        let error = error.to_value(ctx);
        stack.replace(ctx, (Value::Nil, error));
        Ok(SequencePoll::Return)
    }
}
