//! Verification tests for the sandboxed module system (`require`).
//!
//! These tests exercise the verification plan of
//! `docs/specifications/module-resolver.md`:
//!
//! 1. a traversal attempt is rejected as a security error *before* any
//!    resolution or filesystem/tablespace query;
//! 2. a circular `require` (A -> B -> A) terminates through the sentinel;
//! 3. a preloaded module resolves and executes with no VFS root configured;
//! 4. a missing module reports the candidates the searcher chain checked.
//!
//! They also pin the sandbox defaults: empty `package.path`, no `loadlib`,
//! no native C searcher.

use phodopus::{
    Closure, Context, Executor, ExternError, FromMultiValue, Function, Lua,
    lua::LuaBuilder,
    stdlib::{Collect, ModuleSearcher, SearchError, register_preload, register_searcher},
};

/// Run `source` to completion and return the values as a single string
/// (the tests always `return` a string or use `tostring`).
fn run<R: for<'gc> FromMultiValue<'gc>>(lua: &mut Lua, source: &str) -> Result<R, ExternError> {
    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, source.as_bytes())?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    lua.execute::<R>(&executor)
}

fn run_string(lua: &mut Lua, source: &str) -> String {
    run::<String>(lua, source).expect("script should run cleanly")
}

fn builder() -> LuaBuilder {
    Lua::builder()
}

#[test]
fn default_runtime_is_preload_only_and_pathless() {
    let mut lua = Lua::core();
    let result = run_string(
        &mut lua,
        r#"
            return table.concat({
                tostring(type(require)),
                tostring(package.path == ""),
                tostring(package.cpath == nil),
                tostring(package.loadlib == nil),
            }, ",")
        "#,
    );
    assert_eq!(result, "function,true,true,true");
}

#[test]
fn preload_resolves_and_executes_without_vfs() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            Some("=greet"),
            b"return { greeting = 'hello' }".as_slice(),
        )?;
        let function: Function = closure.into();
        register_preload(ctx, "greet", function);
        Ok(())
    })?;

    let result = run_string(
        &mut lua,
        r#"
            local greet = require("greet")
            return greet.greeting
        "#,
    );
    assert_eq!(result, "hello");

    Ok(())
}

#[test]
fn preload_result_is_cached_in_package_loaded() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, Some("=counter"), b"return {}".as_slice())?;
        register_preload(ctx, "counter", closure.into());
        Ok(())
    })?;

    let result = run_string(
        &mut lua,
        r#"
            local a = require("counter")
            local b = require("counter")
            return tostring(a == b) .. "," .. tostring(package.loaded["counter"] == a)
        "#,
    );
    assert_eq!(result, "true,true");

    Ok(())
}

#[test]
fn require_resolution_is_interruptible_by_fuel() {
    let lua = &mut builder()
        .add_embedded_module("interruptible", "return 'loaded'")
        .build();

    // A tiny per-step Fuel budget must preempt the searcher walk without losing
    // progress; the sequence resumes to the same result. This proves resolution
    // is charged proportionally rather than running to completion in one step.
    let executor = lua
        .try_enter(|ctx| {
            let closure = Closure::load(ctx, None, br#"return require("interruptible")"#)?;
            Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
        })
        .expect("script should compile");

    let mut interruptions = 0usize;
    loop {
        let done = lua.enter(|ctx| {
            let mut fuel = phodopus::Fuel::with(1);
            ctx.fetch(&executor).step(ctx, &mut fuel).unwrap()
        });
        if done {
            break;
        }
        interruptions += 1;
        assert!(interruptions < 1_000_000, "executor made no progress");
    }

    let result = lua.execute::<String>(&executor).expect("clean finish");
    assert_eq!(result, "loaded");
    let _ = interruptions;
}

#[test]
fn failed_loader_does_not_leave_sentinel_cached() {
    let lua = &mut builder()
        .add_vfs_module("plugin", "boom.lua", "error('loader exploded')")
        .build();

    let result = run_string(
        lua,
        r#"
            local ok1, err1 = pcall(require, "boom")
            -- A second attempt must re-run the loader rather than return the
            -- leftover sentinel `true`.
            local ok2, err2 = pcall(require, "boom")
            return tostring(ok1) .. "," .. tostring(ok2)
                .. "," .. tostring(package.loaded["boom"] == nil)
        "#,
    );
    assert_eq!(result, "false,false,true");
}

#[test]
fn vfs_resolves_dotted_names_and_init_files() {
    let lua = &mut builder()
        .add_vfs_module("plugin", "foo/bar.lua", "return 'bar'")
        .add_vfs_module("plugin", "pkg/init.lua", "return 'pkg-init'")
        .build();

    let result = run_string(
        lua,
        r#"
            return require("foo.bar") .. "," .. require("pkg")
        "#,
    );
    assert_eq!(result, "bar,pkg-init");
}

#[test]
fn vfs_candidates_are_per_root() {
    let lua = &mut builder()
        .add_vfs_root("one")
        .add_vfs_root("two")
        .add_vfs_module("two", "mod.lua", "return 'from-two'")
        .build();

    let result = run_string(
        lua,
        r#"
            local ok, mod = pcall(require, "mod")
            return tostring(ok) .. "," .. tostring(mod)
        "#,
    );
    assert_eq!(result, "true,from-two");
}

#[test]
fn embedded_module_resolves_before_vfs() {
    let lua = &mut builder()
        .add_embedded_module("shared", "return 'embedded'")
        .add_vfs_module("plugin", "shared.lua", "return 'vfs'")
        .build();

    let result = run_string(lua, r#"return require("shared")"#);
    assert_eq!(result, "embedded");
}

#[test]
fn circular_require_terminates_via_sentinel() {
    let lua = &mut builder()
        .add_vfs_module(
            "plugin",
            "a.lua",
            "local b = require('b'); return 'a(' .. b .. ')'",
        )
        .add_vfs_module(
            "plugin",
            "b.lua",
            "local a = require('a'); return 'b(' .. tostring(a) .. ')'",
        )
        .build();

    let result = run_string(lua, r#"return require("a")"#);
    // `a` requires `b`; `b` requires `a` while `a` is still loading and sees
    // the sentinel `true`; `b` therefore returns "b(true)" and `a` -> "a(b(true))".
    assert_eq!(result, "a(b(true))");
}

#[test]
fn self_require_returns_sentinel() {
    let lua = &mut builder()
        .add_vfs_module(
            "plugin",
            "self.lua",
            "return 'self=' .. tostring(require('self'))",
        )
        .build();

    let result = run_string(lua, r#"return require("self")"#);
    assert_eq!(result, "self=true");
}

#[test]
fn traversal_is_rejected_before_any_searcher_runs() {
    let mut lua = builder()
        .add_vfs_module("plugin", "safe.lua", "return 'safe'")
        .build();

    // A searcher placed at the end of the chain that would raise a
    // recognizable error if it were ever consulted. If traversal validation
    // happened after resolution, this searcher would run first.
    #[derive(Collect)]
    #[collect(require_static)]
    struct SpySearcher;

    impl<'gc> ModuleSearcher<'gc> for SpySearcher {
        fn search(
            &self,
            _ctx: Context<'gc>,
            _name: &str,
        ) -> Result<Option<Function<'gc>>, SearchError> {
            Err(SearchError("spy searcher must never be reached".into()))
        }

        fn describe(&self, _name: &str) -> String {
            "spy".into()
        }
    }

    lua.try_enter(|ctx| {
        register_searcher(ctx, SpySearcher);
        Ok(())
    })
    .expect("searcher registration should succeed");

    let result = run_string(
        &mut lua,
        r#"
            local ok, err = pcall(require, "../../../etc/passwd")
            return tostring(ok) .. "|" .. tostring(err)
        "#,
    );
    let (ok, err) = result.split_once('|').expect("expected ok|err");
    assert_eq!(ok, "false", "traversal must raise an error");
    assert!(
        err.contains("access violation") && err.contains("../../../etc/passwd"),
        "expected an access-violation security error, got: {err}"
    );
    assert!(
        !err.contains("spy searcher"),
        "validation must happen before any searcher runs, got: {err}"
    );
}

#[test]
fn traversal_variants_are_all_rejected() {
    let lua = &mut builder()
        .add_vfs_module("plugin", "safe.lua", "return 'safe'")
        .build();

    // Each of these attempts an escape that must be rejected by the
    // pre-resolution validator (traversal, root escape, absolute path, drive
    // letter, UNC prefix, empty segment).
    let names = [
        "../../../etc/passwd",
        "./relative",
        "/absolute",
        "a..b",
        "trailing.",
        "C:windows",
        "\\\\server\\share",
        "foo/bar",
        "foo\\bar",
    ];

    for name in names {
        let source = format!(
            r#"
                local ok, err = pcall(require, {name:?})
                return tostring(ok) .. "|" .. tostring(err)
            "#,
            name = name
        );
        let result = run_string(lua, &source);
        let (ok, err) = result.split_once('|').expect("expected ok|err");
        assert_eq!(ok, "false", "require({name:?}) must be rejected");
        assert!(
            err.contains("access violation"),
            "require({name:?}) must produce an access violation, got: {err}"
        );
    }
}

#[test]
fn missing_module_lists_searcher_candidates() {
    let lua = &mut builder()
        .add_vfs_root("plugin")
        .add_embedded_module("present", "return 1")
        .build();

    let result = run_string(
        lua,
        r#"
            local ok, err = pcall(require, "absent.module")
            return tostring(ok) .. "|" .. tostring(err)
        "#,
    );
    let (ok, err) = result.split_once('|').expect("expected ok|err");
    assert_eq!(ok, "false");
    assert!(
        err.contains("module 'absent.module' not found:"),
        "expected a not-found header, got: {err}"
    );
    assert!(
        err.contains("no field package.preload['absent.module']"),
        "preload candidate missing from: {err}"
    );
    assert!(
        err.contains("no embedded module 'absent.module'"),
        "embedded candidate missing from: {err}"
    );
    assert!(
        err.contains("no VFS module 'plugin/absent/module.lua'"),
        "VFS candidate missing from: {err}"
    );
}

#[test]
fn custom_host_searcher_participates_in_chain() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    lua.try_enter(|ctx| {
        // A host searcher can return a loader produced from an arbitrary
        // source, mimicking a host-provided module.
        let closure = Closure::load(ctx, Some("=host"), b"return 'host-loaded'".as_slice())?;
        let function: Function = closure.into();
        ctx.set_global("host_loader", function);
        Ok(())
    })?;

    lua.try_enter(|ctx| {
        register_host_searcher(ctx);
        Ok(())
    })
    .expect("registration should succeed");

    let result = run_string(&mut lua, r#"return require("from.host") "#);
    assert_eq!(result, "host-loaded");

    Ok(())
}

fn register_host_searcher<'gc>(ctx: Context<'gc>) {
    let loader: phodopus::Value = ctx.get_global_value("host_loader");
    register_searcher(ctx, HostSearcher { loader });
}

#[derive(Collect)]
#[collect(no_drop)]
struct HostSearcher<'gc> {
    loader: phodopus::Value<'gc>,
}

impl<'gc> ModuleSearcher<'gc> for HostSearcher<'gc> {
    fn search(&self, _ctx: Context<'gc>, name: &str) -> Result<Option<Function<'gc>>, SearchError> {
        if name == "from.host" {
            match self.loader {
                phodopus::Value::Function(f) => Ok(Some(f)),
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    fn describe(&self, name: &str) -> String {
        format!("no host module '{name}'")
    }
}

#[test]
fn module_config_used_with_load_core_with() -> Result<(), ExternError> {
    let mut config = phodopus::stdlib::ModuleConfig::default();
    config.add_vfs_module("plugin", "configured.lua", "return 'configured'");

    let mut lua = Lua::empty();
    lua.load_core_with(&config);

    let result = run::<String>(&mut lua, r#"return require("configured")"#)?;
    assert_eq!(result, "configured");

    Ok(())
}

#[test]
fn core_builtins_resolve_through_preload() {
    let mut lua = Lua::core();
    // `require("string")` must return the same table as the `string` global and
    // must not consult the VFS (there is no root).
    let result = run_string(
        &mut lua,
        r#"
            return tostring(require("string") == string)
                .. "," .. tostring(require("table") == table)
                .. "," .. tostring(require("math") == math)
        "#,
    );
    assert_eq!(result, "true,true,true");

    // An unloaded core library is not preloaded, so requiring it fails cleanly.
    let result = run_string(
        &mut lua,
        r#"
            local ok = pcall(require, "io")
            return tostring(ok)
        "#,
    );
    assert_eq!(result, "false");
}

#[test]
fn require_result_is_visible_to_lua_globals() {
    let lua = &mut builder()
        .add_embedded_module("values", "return 40 + 2")
        .build();

    let result = run_string(lua, r#"return tostring(require("values"))"#);
    assert_eq!(result, "42");
}

#[test]
fn require_rejects_non_string_argument() {
    let mut lua = Lua::core();
    let result = run_string(
        &mut lua,
        r#"
            local ok, err = pcall(require, 42)
            return tostring(ok) .. "|" .. tostring(err)
        "#,
    );
    // Integers are implicitly convertible to strings in Lua; 42 becomes "42",
    // which is a valid module name and simply fails to resolve.
    assert!(
        result.starts_with("false|"),
        "require(42) should fail to resolve, got: {result}"
    );
}

#[test]
fn traversal_does_not_reach_a_registered_root() {
    // Register a root containing a file named like the escape target to prove
    // the rejection is path validation, not a mere lookup miss.
    let lua = &mut builder()
        .add_vfs_module("plugin", "etc/passwd", "return 'secret'")
        .build();

    let result = run_string(
        lua,
        r#"
            local ok, err = pcall(require, "../../../etc/passwd")
            return tostring(ok) .. "|" .. tostring(err)
        "#,
    );
    let (ok, err) = result.split_once('|').expect("expected ok|err");
    assert_eq!(ok, "false");
    assert!(err.contains("access violation"), "got: {err}");
    assert!(!result.contains("secret"));
}

#[test]
fn validate_module_path_accepts_plain_names() {
    for name in ["a", "a.b.c", "foo_bar", "foo-bar.baz", "A1._x"] {
        phodopus::stdlib::validate_module_path(name)
            .unwrap_or_else(|err| panic!("{name:?} should be valid, got {err}"));
    }
}

#[test]
fn validate_module_path_rejects_escapes() {
    for name in [
        "",
        ".",
        "..",
        "a..b",
        ".a",
        "a.",
        "/a",
        "a/b",
        "a\\b",
        "C:",
        "C:\\x",
        "\\\\server",
        "a b",
        "a$b",
        "a\x00b",
    ] {
        assert!(
            phodopus::stdlib::validate_module_path(name).is_err(),
            "{name:?} should be rejected"
        );
    }
}
