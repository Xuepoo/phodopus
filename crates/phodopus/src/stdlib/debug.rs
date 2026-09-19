use anyhow::anyhow;

use crate::{
    conversion::IntoValue,
    error::{pretty_print_error_with_backtrace, BacktraceFrame},
    Callback, CallbackReturn, Context, Table, Value,
};

pub fn load_debug<'gc>(ctx: Context<'gc>) {
    let debug = Table::new(&ctx);

    debug.set_field(
        ctx,
        "traceback",
        Callback::from_fn(&ctx, |ctx, exec, mut stack| {
            let (thread, message, level) = match stack.get(0) {
                Value::Thread(t) => {
                    let message = if stack.get(1).is_nil() {
                        None
                    } else {
                        Some(stack.get(1))
                    };
                    let level = if stack.get(2).is_nil() {
                        None
                    } else {
                        Some(
                            stack
                                .get(2)
                                .to_integer()
                                .ok_or_else(|| anyhow!("level must be an integer"))?,
                        )
                    };
                    (Some(t), message, level)
                }
                Value::Nil
                    if stack.len() > 2
                        || (stack.len() == 2
                            && !stack.get(1).is_nil()
                            && stack.get(1).to_integer().is_none()) =>
                {
                    let message = if stack.get(1).is_nil() {
                        None
                    } else {
                        Some(stack.get(1))
                    };
                    let level = if stack.get(2).is_nil() {
                        None
                    } else {
                        Some(
                            stack
                                .get(2)
                                .to_integer()
                                .ok_or_else(|| anyhow!("level must be an integer"))?,
                        )
                    };
                    (None, message, level)
                }
                _ => {
                    let message = if stack.get(0).is_nil() {
                        None
                    } else {
                        Some(stack.get(0))
                    };
                    let level = if stack.get(1).is_nil() {
                        None
                    } else {
                        Some(
                            stack
                                .get(1)
                                .to_integer()
                                .ok_or_else(|| anyhow!("level must be an integer"))?,
                        )
                    };
                    (None, message, level)
                }
            };

            // Level defaults to 1 for user calls, skipping the `debug.traceback` frame itself.
            let level = level.unwrap_or(1);
            if level < 0 {
                return Err(anyhow!("level must be non-negative").into());
            }

            let level0 = BacktraceFrame::Callback {
                name: "debug.traceback",
            };
            let backtrace_frames =
                if let Some(t) = thread.filter(|t| t != &exec.current_thread().thread) {
                    // Try to borrow the target thread's state
                    // (Note: we filter out the current thread because we know it's already borrowed while running)
                    let thread_state_borrow = t.into_inner().try_borrow();
                    let thread_state = thread_state_borrow
                        .as_ref()
                        .map_err(|_| anyhow!("cannot get traceback for running thread"))?;
                    crate::thread::backtrace(
                        thread_state.frames(),
                        thread_state.stack(),
                        &[],
                        Some(level0),
                    )
                } else {
                    // This is the current running thread, use its upper frames and the full stack
                    crate::thread::backtrace(
                        exec.upper_frames(),
                        stack.borrow_thread_view(),
                        &[],
                        Some(level0),
                    )
                };

            let mut formatted_error_str = String::new();

            if let Some(msg_value) = message {
                formatted_error_str = msg_value
                    .into_string(ctx)
                    .map(|s| s.display_lossy().to_string())
                    .unwrap_or_else(|| msg_value.display().to_string());
            }

            // Skip frames based on the level.
            let skipped_frames = (level as usize).min(backtrace_frames.len());
            let remaining_frames = &backtrace_frames[skipped_frames..];

            let dummy_error = formatted_error_str.into_value(ctx);

            let mut traceback_string = String::new();
            pretty_print_error_with_backtrace(
                &mut traceback_string,
                &dummy_error.display(),
                &Some(remaining_frames),
                None, // No source map
            )?;

            stack.replace(ctx, traceback_string);
            Ok(CallbackReturn::Return)
        }),
    );

    ctx.set_global("debug", debug);
}
