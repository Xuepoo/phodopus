use std::cmp::Ordering;

use phodopus::table::{InvalidTableKey, TableError};
use phodopus::{Lua, Table, Value};

#[test]
fn test_table_iter() {
    let mut lua = Lua::core();

    lua.enter(|ctx| {
        let table = Table::new(&ctx);

        table.set(ctx, 1, "1").unwrap();
        table.set(ctx, 2, "2").unwrap();
        table.set(ctx, 3, "3").unwrap();
        table.set(ctx, "1", 1).unwrap();
        table.set(ctx, "2", 2).unwrap();
        table.set(ctx, "3", 3).unwrap();

        let mut pairs = table.iter().collect::<Vec<_>>();
        pairs.sort_by(|&(ak, _), &(bk, _)| match (ak, bk) {
            (phodopus::Value::Integer(a), phodopus::Value::Integer(b)) => a.cmp(&b),
            (phodopus::Value::Integer(_), phodopus::Value::String(_)) => Ordering::Less,
            (phodopus::Value::String(_), phodopus::Value::Integer(_)) => Ordering::Greater,
            (phodopus::Value::String(a), phodopus::Value::String(b)) => a.cmp(&b),
            _ => unreachable!(),
        });

        assert_eq!(pairs.len(), 6);
        assert!(matches!(pairs[0], (Value::Integer(1), Value::String(s)) if s == "1" ));
        assert!(matches!(pairs[1], (Value::Integer(2), Value::String(s)) if s == "2" ));
        assert!(matches!(pairs[2], (Value::Integer(3), Value::String(s)) if s == "3" ));
        assert!(matches!(pairs[3], (Value::String(s), Value::Integer(1)) if s == "1" ));
        assert!(matches!(pairs[4], (Value::String(s), Value::Integer(2)) if s == "2" ));
        assert!(matches!(pairs[5], (Value::String(s), Value::Integer(3)) if s == "3" ));

        for (k, _) in table.iter() {
            table.set(ctx, k, Value::Nil).unwrap();
        }

        assert!(table.get_value(ctx, 1).is_nil());
        assert!(table.get_value(ctx, 2).is_nil());
        assert!(table.get_value(ctx, 3).is_nil());
        assert!(table.get_value(ctx, "1").is_nil());
        assert!(table.get_value(ctx, "2").is_nil());
        assert!(table.get_value(ctx, "3").is_nil());
    });
}

/// Nil and NaN keys are rejected before any map slot is touched; this exercises the canonical-key
/// guard that keeps the raw table's live/dead key invariant intact.
#[test]
fn invalid_keys_are_rejected() {
    let mut lua = Lua::core();

    lua.enter(|ctx| {
        let table = Table::new(&ctx);

        assert!(matches!(
            table.set(ctx, Value::Nil, 1),
            Err(TableError::Key(InvalidTableKey::IsNil))
        ));
        assert!(matches!(
            table.set(ctx, f64::NAN, 1),
            Err(TableError::Key(InvalidTableKey::IsNaN))
        ));
        assert!(matches!(
            table.set(ctx, Value::Nil, Value::Nil),
            Err(TableError::Key(InvalidTableKey::IsNil))
        ));

        assert!(matches!(table.get_value(ctx, Value::Nil), Value::Nil));
        assert!(matches!(table.get_value(ctx, f64::NAN), Value::Nil));
    });
}

/// A key removed (set to Nil) during iteration must not corrupt iteration order or be yielded
/// again; this covers the dead-key resurrection and erase paths in `RawTable::set`/`RawTable::next`.
#[test]
fn remove_during_iteration_then_refill() {
    let mut lua = Lua::core();

    lua.enter(|ctx| {
        let table = Table::new(&ctx);
        for i in 1..=8 {
            table.set(ctx, i, i).unwrap();
        }
        table.set(ctx, "key", "value").unwrap();

        for (key, _) in table.iter() {
            table.set(ctx, key, Value::Nil).unwrap();
        }
        assert_eq!(table.iter().count(), 0);

        table.set(ctx, "key", "refilled").unwrap();
        assert!(
            matches!(table.get_value(ctx, "key"), Value::String(s) if s == "refilled"),
            "expected the re-inserted string value"
        );
        assert_eq!(table.iter().count(), 1);
    });
}
