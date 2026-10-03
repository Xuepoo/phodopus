use allocator_api2::vec;
use gc_arena::allocator_api::MetricsAlloc;

use crate::{
    Closure, Constant, Context, Function, String, Table, Value,
    meta_ops::{self, ConcatMetaResult, MetaResult},
    opcode::{Operation, RCIndex},
    thread::thread::MetaReturn,
    types::{RegisterIndex, UpValueDescriptor, VarCount},
};

use super::{VMError, thread::LuaFrame};

// Runs the VM for the given number of instructions or until the current LuaFrame may have been
// changed.
//
// Returns the number of instructions that were run.
pub(super) fn run_vm<'gc>(
    ctx: Context<'gc>,
    mut lua_frame: LuaFrame<'gc, '_>,
    max_instructions: u32,
) -> Result<u32, VMError> {
    if max_instructions == 0 {
        return Ok(0);
    }

    let current_function = lua_frame.closure();
    let current_prototype = current_function.prototype();
    let current_upvalues = current_function.upvalues();
    let mut registers = lua_frame.registers();
    let mut instructions_run = 0;

    fn get_rc<'gc>(
        stack_frame: &[Value<'gc>],
        constants: &[Constant<String<'gc>>],
        rc: RCIndex,
    ) -> Value<'gc> {
        match rc {
            RCIndex::Register(r) => stack_frame[r.0 as usize],
            RCIndex::Constant(c) => constants[c.0 as usize].into(),
        }
    }

    loop {
        let op = current_prototype.opcodes[*registers.pc].decode();
        *registers.pc += 1;

        match op {
            Operation::Move { dest, source } => {
                registers.stack_frame[dest.0 as usize] = registers.stack_frame[source.0 as usize];
            }

            Operation::LoadConstant { dest, constant } => {
                registers.stack_frame[dest.0 as usize] =
                    current_prototype.constants[constant.0 as usize].into();
            }

            Operation::LoadBool {
                dest,
                value,
                skip_next,
            } => {
                registers.stack_frame[dest.0 as usize] = Value::Boolean(value);
                if skip_next {
                    *registers.pc += 1;
                }
            }

            Operation::LoadNil { dest, count } => {
                for i in dest.0..dest.0 + count {
                    registers.stack_frame[i as usize] = Value::Nil;
                }
            }

            Operation::NewTable {
                dest,
                array_size,
                map_size,
            } => {
                // Hard-quota check *before* the table parts are allocated, so a constructor chain
                // such as `local t = {}; while true do t = {t} end` is refused cleanly rather than
                // being built and only measured afterwards.
                let table = Table::try_new(ctx, array_size as usize, map_size as usize)?;
                registers.stack_frame[dest.0 as usize] = Value::Table(table);
            }

            Operation::GetTable { dest, table, key } => {
                let table = registers.stack_frame[table.0 as usize];
                let key = get_rc(&registers.stack_frame, &current_prototype.constants, key);
                match table {
                    Value::Table(t) if t.metatable().is_none() => {
                        registers.stack_frame[dest.0 as usize] = t.get_value(ctx, key);
                    }
                    _ => match meta_ops::index(ctx, table, key)? {
                        MetaResult::Value(v) => {
                            registers.stack_frame[dest.0 as usize] = v;
                        }
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::SetTable { table, key, value } => {
                let table = registers.stack_frame[table.0 as usize];
                let key = get_rc(&registers.stack_frame, &current_prototype.constants, key);
                let value = get_rc(&registers.stack_frame, &current_prototype.constants, value);
                match table {
                    Value::Table(t) if t.metatable().is_none() => {
                        t.set_raw(ctx, key, value)?;
                    }
                    _ => {
                        if let Some(call) = meta_ops::new_index(ctx, table, key, value)? {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::None,
                            )?;
                            break;
                        }
                    }
                }
            }

            Operation::GetUpTable { dest, table, key } => {
                let table = registers.get_upvalue(&ctx, current_upvalues[table.0 as usize]);
                let key = get_rc(&registers.stack_frame, &current_prototype.constants, key);
                match table {
                    Value::Table(t) if t.metatable().is_none() => {
                        registers.stack_frame[dest.0 as usize] = t.get_value(ctx, key);
                    }
                    _ => match meta_ops::index(ctx, table, key)? {
                        MetaResult::Value(v) => {
                            registers.stack_frame[dest.0 as usize] = v;
                        }
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::SetUpTable { table, key, value } => {
                let table = registers.get_upvalue(&ctx, current_upvalues[table.0 as usize]);
                let key = get_rc(&registers.stack_frame, &current_prototype.constants, key);
                let value = get_rc(&registers.stack_frame, &current_prototype.constants, value);
                match table {
                    Value::Table(t) if t.metatable().is_none() => {
                        t.set_raw(ctx, key, value)?;
                    }
                    _ => {
                        if let Some(call) = meta_ops::new_index(ctx, table, key, value)? {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::None,
                            )?;
                            break;
                        }
                    }
                }
            }

            Operation::SetList { base, count } => {
                lua_frame.set_table_list(ctx, base, count)?;
                registers = lua_frame.registers();
            }

            Operation::Call {
                func,
                args,
                returns,
            } => {
                lua_frame.call_function(ctx, func, args, returns)?;
                break;
            }

            Operation::TailCall { func, args } => {
                lua_frame.tail_call_function(ctx, func, args)?;
                break;
            }

            Operation::Return { start, count } => {
                lua_frame.return_upper(&ctx, start, count)?;
                break;
            }

            Operation::VarArgs { dest, count } => {
                lua_frame.varargs(dest, count)?;
                registers = lua_frame.registers();
            }

            Operation::Jump {
                offset,
                close_upvalues,
            } => {
                *registers.pc = add_offset(*registers.pc, offset);
                if let Some(r) = close_upvalues.to_u8() {
                    registers.close_upvalues(&ctx, RegisterIndex(r));
                }
            }

            Operation::Test { value, is_true } => {
                let value = registers.stack_frame[value.0 as usize];
                if value.to_bool() == is_true {
                    *registers.pc += 1;
                }
            }

            Operation::TestSet {
                dest,
                value,
                is_true,
            } => {
                let value = registers.stack_frame[value.0 as usize];
                if value.to_bool() == is_true {
                    *registers.pc += 1;
                } else {
                    registers.stack_frame[dest.0 as usize] = value;
                }
            }

            Operation::Closure { proto, dest } => {
                let proto = current_prototype.prototypes[proto.0 as usize];
                let mut upvalues = vec::Vec::new_in(MetricsAlloc::new(&ctx));
                for &desc in proto.upvalues.iter() {
                    match desc {
                        UpValueDescriptor::Environment => {
                            return Err(VMError::BadEnvUpValue.into());
                        }
                        UpValueDescriptor::ParentLocal(reg) => {
                            upvalues.push(registers.open_upvalue(&ctx, reg));
                        }
                        UpValueDescriptor::Outer(uvindex) => {
                            upvalues.push(current_upvalues[uvindex.0 as usize]);
                        }
                    }
                }

                let closure = Closure::try_from_parts(ctx, proto, upvalues)?;
                registers.stack_frame[dest.0 as usize] =
                    Value::Function(Function::Closure(closure));
            }

            Operation::NumericForPrep { base, jump } => {
                let init = registers.stack_frame[base.0 as usize];
                let step = registers.stack_frame[base.0 as usize + 2];
                let res = match (init, step) {
                    (Value::Integer(i), Value::Integer(s)) => {
                        Some(Value::Integer(i.wrapping_sub(s)))
                    }
                    (Value::Number(i), Value::Number(s)) => Some(Value::Number(i - s)),
                    (Value::Integer(i), Value::Number(s)) => Some(Value::Number((i as f64) - s)),
                    (Value::Number(i), Value::Integer(s)) => Some(Value::Number(i - (s as f64))),
                    _ => raw_subtract(init, step),
                };
                registers.stack_frame[base.0 as usize] =
                    res.ok_or_else(|| VMError::BadForLoopPrep(init.type_name(), step.type_name()))?;
                *registers.pc = add_offset(*registers.pc, jump);
            }

            Operation::NumericForLoop { base, jump } => {
                match (
                    registers.stack_frame[base.0 as usize],
                    registers.stack_frame[base.0 as usize + 1],
                    registers.stack_frame[base.0 as usize + 2],
                ) {
                    (Value::Integer(index), Value::Integer(limit), Value::Integer(step)) => {
                        let (index, overflow) = index.overflowing_add(step);
                        registers.stack_frame[base.0 as usize] = Value::Integer(index);

                        let past_end = overflow
                            || if step < 0 {
                                index < limit
                            } else {
                                index > limit
                            };
                        if !past_end {
                            *registers.pc = add_offset(*registers.pc, jump);
                            registers.stack_frame[base.0 as usize + 3] = Value::Integer(index);
                        }
                    }
                    (Value::Integer(index), limit, Value::Integer(step)) => {
                        if let Some(limit) = limit.to_number() {
                            let (index, overflow) = index.overflowing_add(step);
                            registers.stack_frame[base.0 as usize] = Value::Integer(index);

                            let past_end = overflow
                                || if step < 0 {
                                    !(index as f64 >= limit)
                                } else {
                                    !(index as f64 <= limit)
                                };
                            if !past_end {
                                *registers.pc = add_offset(*registers.pc, jump);
                                registers.stack_frame[base.0 as usize + 3] = Value::Integer(index);
                            }
                        } else {
                            return Err(VMError::BadForLoop(
                                "integer",
                                limit.type_name(),
                                "integer",
                            ));
                        }
                    }
                    (index, limit, step) => {
                        if let (Some(index), Some(limit), Some(step)) =
                            (index.to_number(), limit.to_number(), step.to_number())
                        {
                            let index = index + step;
                            registers.stack_frame[base.0 as usize] = Value::Number(index);

                            let past_end = if step < 0.0 {
                                !(index >= limit)
                            } else {
                                !(index <= limit)
                            };
                            if !past_end {
                                *registers.pc = add_offset(*registers.pc, jump);
                                registers.stack_frame[base.0 as usize + 3] = Value::Number(index);
                            }
                        } else {
                            return Err(VMError::BadForLoop(
                                index.type_name(),
                                limit.type_name(),
                                step.type_name(),
                            ));
                        }
                    }
                }
            }

            Operation::GenericForCall { base, var_count } => {
                lua_frame.call_function_keep(ctx, base, 2, VarCount::constant(var_count))?;
                break;
            }

            Operation::GenericForLoop { base, jump } => {
                if registers.stack_frame[base.0 as usize + 1].to_bool() {
                    registers.stack_frame[base.0 as usize] =
                        registers.stack_frame[base.0 as usize + 1];
                    *registers.pc = add_offset(*registers.pc, jump);
                }
            }

            Operation::Method { base, table, key } => {
                let table = registers.stack_frame[table.0 as usize];
                let key = get_rc(&registers.stack_frame, &current_prototype.constants, key);
                registers.stack_frame[base.0 as usize + 1] = table;
                match meta_ops::index(ctx, table, key)? {
                    MetaResult::Value(v) => {
                        registers.stack_frame[base.0 as usize] = v;
                    }
                    MetaResult::Call(call) => {
                        lua_frame.call_meta_function(
                            ctx,
                            call.function,
                            &call.args,
                            MetaReturn::Register(base),
                        )?;
                        break;
                    }
                }
            }

            Operation::Concat {
                dest,
                source,
                count,
            } => {
                let base = source.0 as usize;
                let values = &registers.stack_frame[base..base + count as usize];
                match meta_ops::concat_many(ctx, values)? {
                    ConcatMetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                    ConcatMetaResult::Call(func) => {
                        // This would be nice to do in-place (without copying params)
                        // but that turns out to be difficult to do without corrupting
                        // the stack, and likely isn't worth the complexity for the
                        // fallback case.
                        let args = values.to_owned();
                        lua_frame.call_meta_function(
                            ctx,
                            func,
                            &args,
                            MetaReturn::Register(dest),
                        )?;
                        break;
                    }
                }
            }

            Operation::GetUpValue { source, dest } => {
                registers.stack_frame[dest.0 as usize] =
                    registers.get_upvalue(&ctx, current_upvalues[source.0 as usize]);
            }

            Operation::SetUpValue { source, dest } => {
                registers.set_upvalue(
                    &ctx,
                    current_upvalues[dest.0 as usize],
                    registers.stack_frame[source.0 as usize],
                );
            }

            Operation::Length { dest, source } => {
                let source = registers.stack_frame[source.0 as usize];
                match source {
                    Value::String(s) => {
                        registers.stack_frame[dest.0 as usize] = (s.len() as i64).into();
                    }
                    Value::Table(t) if t.metatable().is_none() => {
                        registers.stack_frame[dest.0 as usize] = (t.length() as i64).into();
                    }
                    _ => match meta_ops::len(ctx, source)? {
                        MetaResult::Value(v) => {
                            registers.stack_frame[dest.0 as usize] = v;
                        }
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Eq {
                skip_if,
                left,
                right,
            } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        if (a == b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        if (a == b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        if (a as f64 == b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        if (b as f64 == a) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Boolean(a), Value::Boolean(b)) => {
                        if (a == b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::String(a), Value::String(b)) => {
                        if (a == b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Nil, Value::Nil) => {
                        if skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Nil, _) | (_, Value::Nil) => {
                        if !skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Table(a), Value::Table(b)) if a == b => {
                        if skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::UserData(a), Value::UserData(b)) if a == b => {
                        if skip_if {
                            *registers.pc += 1;
                        }
                    }
                    _ => match meta_ops::equal(ctx, left, right)? {
                        MetaResult::Value(v) => {
                            if v.to_bool() == skip_if {
                                *registers.pc += 1;
                            }
                        }
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::SkipIf(skip_if),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Less {
                skip_if,
                left,
                right,
            } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        if (a < b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        if (a < b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        if ((a as f64) < b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        if (a < (b as f64)) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::String(a), Value::String(b)) => {
                        if (a.as_bytes() < b.as_bytes()) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    _ => match meta_ops::less_than(ctx, left, right)? {
                        MetaResult::Value(v) => {
                            if v.to_bool() == skip_if {
                                *registers.pc += 1;
                            }
                        }
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::SkipIf(skip_if),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::LessEq {
                skip_if,
                left,
                right,
            } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        if (a <= b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        if (a <= b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        if ((a as f64) <= b) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        if (a <= (b as f64)) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    (Value::String(a), Value::String(b)) => {
                        if (a.as_bytes() <= b.as_bytes()) == skip_if {
                            *registers.pc += 1;
                        }
                    }
                    _ => match meta_ops::less_equal(ctx, left, right)? {
                        MetaResult::Value(v) => {
                            if v.to_bool() == skip_if {
                                *registers.pc += 1;
                            }
                        }
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::SkipIf(skip_if),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Not { dest, source } => {
                let source = registers.stack_frame[source.0 as usize];
                registers.stack_frame[dest.0 as usize] = (!source.to_bool()).into();
            }

            Operation::Minus { dest, source } => {
                let value = registers.stack_frame[source.0 as usize];
                match value {
                    Value::Integer(a) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(a.wrapping_neg());
                    }
                    Value::Number(a) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(-a);
                    }
                    _ => match meta_ops::negate(ctx, value)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::BitNot { dest, source } => {
                let value = registers.stack_frame[source.0 as usize];
                match value {
                    Value::Integer(a) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(!a);
                    }
                    _ => match meta_ops::bitwise_not(ctx, value)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Add { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(a.wrapping_add(b));
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a + b);
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number((a as f64) + b);
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a + (b as f64));
                    }
                    _ => match meta_ops::add(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Sub { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(a.wrapping_sub(b));
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a - b);
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number((a as f64) - b);
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a - (b as f64));
                    }
                    _ => match meta_ops::subtract(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Mul { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(a.wrapping_mul(b));
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a * b);
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number((a as f64) * b);
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a * (b as f64));
                    }
                    _ => match meta_ops::multiply(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Div { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Number(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a / b);
                    }
                    (Value::Integer(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] =
                            Value::Number((a as f64) / (b as f64));
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number((a as f64) / b);
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(a / (b as f64));
                    }
                    _ => match meta_ops::float_divide(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::IDiv { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) if b != 0 => {
                        let d = a.wrapping_div(b);
                        let r = a.wrapping_rem(b);
                        let d = if (r > 0 && b < 0) || (r < 0 && b > 0) {
                            d.wrapping_sub(1)
                        } else {
                            d
                        };
                        registers.stack_frame[dest.0 as usize] = Value::Integer(d);
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number((a / b).floor());
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] =
                            Value::Number(((a as f64) / b).floor());
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] =
                            Value::Number((a / (b as f64)).floor());
                    }
                    _ => match meta_ops::floor_divide(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Mod { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) if b != 0 => {
                        if b == -1 {
                            registers.stack_frame[dest.0 as usize] = Value::Integer(0);
                        } else {
                            let r = a % b;
                            let res = if (r > 0 && b < 0) || (r < 0 && b > 0) {
                                r.wrapping_add(b)
                            } else {
                                r
                            };
                            registers.stack_frame[dest.0 as usize] = Value::Integer(res);
                        }
                    }
                    (Value::Number(a), Value::Number(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Number(((a % b) + b) % b);
                    }
                    (Value::Integer(a), Value::Number(b)) => {
                        let a = a as f64;
                        registers.stack_frame[dest.0 as usize] = Value::Number(((a % b) + b) % b);
                    }
                    (Value::Number(a), Value::Integer(b)) => {
                        let b = b as f64;
                        registers.stack_frame[dest.0 as usize] = Value::Number(((a % b) + b) % b);
                    }
                    _ => match meta_ops::modulo(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::Pow { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match meta_ops::exponentiate(ctx, left, right)? {
                    MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                    MetaResult::Call(call) => {
                        lua_frame.call_meta_function(
                            ctx,
                            call.function,
                            &call.args,
                            MetaReturn::Register(dest),
                        )?;
                        break;
                    }
                }
            }

            Operation::BitAnd { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(a & b);
                    }
                    _ => match meta_ops::bitwise_and(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::BitOr { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(a | b);
                    }
                    _ => match meta_ops::bitwise_or(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::BitXor { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        registers.stack_frame[dest.0 as usize] = Value::Integer(a ^ b);
                    }
                    _ => match meta_ops::bitwise_xor(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::ShiftLeft { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) if b >= 0 => {
                        let shift = (b as u64).try_into().unwrap_or(u32::MAX);
                        registers.stack_frame[dest.0 as usize] =
                            Value::Integer(a.checked_shl(shift).unwrap_or(0));
                    }
                    _ => match meta_ops::shift_left(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }

            Operation::ShiftRight { dest, left, right } => {
                let left = get_rc(&registers.stack_frame, &current_prototype.constants, left);
                let right = get_rc(&registers.stack_frame, &current_prototype.constants, right);
                match (left, right) {
                    (Value::Integer(a), Value::Integer(b)) if b >= 0 => {
                        let shift = (b as u64).try_into().unwrap_or(u32::MAX);
                        registers.stack_frame[dest.0 as usize] =
                            Value::Integer(((a as u64).checked_shr(shift).unwrap_or(0)) as i64);
                    }
                    _ => match meta_ops::shift_right(ctx, left, right)? {
                        MetaResult::Value(v) => registers.stack_frame[dest.0 as usize] = v,
                        MetaResult::Call(call) => {
                            lua_frame.call_meta_function(
                                ctx,
                                call.function,
                                &call.args,
                                MetaReturn::Register(dest),
                            )?;
                            break;
                        }
                    },
                }
            }
        }

        instructions_run += 1;
        if instructions_run >= max_instructions {
            break;
        }
    }

    Ok(instructions_run)
}

fn add_offset(pc: usize, offset: i16) -> usize {
    if offset > 0 {
        pc.checked_add(offset as usize).unwrap()
    } else if offset < 0 {
        pc.checked_sub(-offset as usize).unwrap()
    } else {
        pc
    }
}

fn raw_subtract<'gc>(lhs: Value<'gc>, rhs: Value<'gc>) -> Option<Value<'gc>> {
    Some(lhs.to_constant()?.subtract(&rhs.to_constant()?)?.into())
}
