#![cfg(not(feature = "no_function"))]
use rhai::{
    BorrowedScopeEntry, CallFnOptions, Dynamic, Engine, EvalAltResult, FnPtr, Func, FuncArgs,
    Scope, AST, INT,
};
use std::any::TypeId;

#[test]
fn test_call_fn() {
    let mut engine = Engine::new();
    let mut scope = Scope::new();

    engine.register_fn("test", |x: INT, y: bool| if y { x + 1 } else { x - 1 });
    scope.push("foo", 42 as INT);

    let ast = engine
        .compile(
            "
                fn hello(x, y) {
                    x + y
                }
                fn hello(x) {
                    x *= foo;
                    foo = 1;
                    x
                }
                fn hello() {
                    41 + foo
                }
                fn define_var(scale) {
                    let bar = 21;
                    bar * scale
                }
            ",
        )
        .unwrap();

    let r = engine.call_fn::<INT>(&mut scope, &ast, "hello", (42 as INT, 123 as INT)).unwrap();
    assert_eq!(r, 165);

    let r = engine.call_fn::<INT>(&mut scope, &ast, "hello", (123 as INT,)).unwrap();
    assert_eq!(r, 5166);

    let r = engine.call_fn::<INT>(&mut scope, &ast, "hello", ()).unwrap();
    assert_eq!(r, 42);

    assert_eq!(scope.get_value::<INT>("foo").expect("variable foo should exist"), 1);

    let r = engine.call_fn::<INT>(&mut scope, &ast, "define_var", (2 as INT,)).unwrap();
    assert_eq!(r, 42);

    assert!(!scope.contains("bar"));

    let options = CallFnOptions::new().eval_ast(false).rewind_scope(false);

    let r = engine.call_fn_with_options::<INT>(options, &mut scope, &ast, "define_var", (2 as INT,)).unwrap();
    assert_eq!(r, 42);

    assert_eq!(scope.get_value::<INT>("bar").expect("variable bar should exist"), 21);

    assert!(!scope.contains("scale"));

    let options = CallFnOptions::new().in_all_namespaces(true);

    let _ = engine.call_fn::<INT>(&mut scope, &ast, "test", (41 as INT, true)).unwrap_err();

    let r = engine.call_fn_with_options::<INT>(options, &mut scope, &ast, "test", (41 as INT, true)).unwrap();
    assert_eq!(r, 42);
}

#[test]
fn test_call_fn_scope() {
    let engine = Engine::new();
    let mut scope = Scope::new();

    let ast = engine
        .compile(
            "
                fn foo(x) {
                    let hello = 42;
                    bar + hello + x
                }

                let bar = 123;
            ",
        )
        .unwrap();

    for _ in 0..50 {
        assert_eq!(
            engine
                .call_fn_with_options::<INT>(CallFnOptions::new().rewind_scope(false), &mut scope, &ast, "foo", [Dynamic::THREE],)
                .unwrap(),
            168
        );
    }

    assert_eq!(scope.len(), 100);
}

struct Options {
    pub foo: bool,
    pub bar: String,
    pub baz: INT,
}

impl FuncArgs for Options {
    fn parse<C: Extend<Dynamic>>(self, container: &mut C) {
        container.extend(Some(self.foo.into()));
        container.extend(Some(self.bar.into()));
        container.extend(Some(self.baz.into()));
    }
}

#[test]
fn test_call_fn_args() {
    let options = Options { foo: false, bar: "world".to_string(), baz: 42 };

    let engine = Engine::new();
    let mut scope = Scope::new();

    let ast = engine
        .compile(
            "
                fn hello(x, y, z) {
                    if x { `hello ${y}` } else { y + z }
                }
            ",
        )
        .unwrap();

    let result = engine.call_fn::<String>(&mut scope, &ast, "hello", options).unwrap();

    assert_eq!(result, "world42");
}

#[test]
fn test_call_fn_private() {
    let engine = Engine::new();
    let mut scope = Scope::new();

    let ast = engine.compile("fn add(x, n) { x + n }").unwrap();

    let r = engine.call_fn::<INT>(&mut scope, &ast, "add", (40 as INT, 2 as INT)).unwrap();
    assert_eq!(r, 42);

    let ast = engine.compile("private fn add(x, n, ) { x + n }").unwrap();

    let r = engine.call_fn::<INT>(&mut scope, &ast, "add", (40 as INT, 2 as INT)).unwrap();
    assert_eq!(r, 42);
}

#[test]
#[cfg(not(feature = "no_object"))]
fn test_fn_ptr_raw() {
    let mut engine = Engine::new();

    engine
        .register_fn("mul", |x: &mut INT, y: INT| *x *= y)
        .register_raw_fn("bar", [TypeId::of::<INT>(), TypeId::of::<FnPtr>(), TypeId::of::<INT>()], move |context, args| {
            let fp = args[1].take().cast::<FnPtr>();
            let value = args[2].clone();
            let this_ptr = args.get_mut(0).unwrap();

            fp.call_raw(&context, Some(this_ptr), [value])
        });

    assert_eq!(
        engine
            .eval::<INT>(
                r#"
                    fn foo(x) { this += x; }

                    let x = 41;
                    x.bar(foo, 1);
                    x
                "#
            )
            .unwrap(),
        42
    );

    assert_eq!(
        engine
            .eval::<INT>(
                r#"
                    fn foo(x, y) { this += x + y; }

                    let x = 40;
                    let v = 1;
                    x.bar(Fn("foo").curry(v), 1);
                    x
                "#
            )
            .unwrap(),
        42
    );

    assert_eq!(
        engine
            .eval::<INT>(
                r#"
                    private fn foo(x) { this += x; }

                    let x = 41;
                    x.bar(Fn("foo"), 1);
                    x
                "#
            )
            .unwrap(),
        42
    );

    assert_eq!(
        engine
            .eval::<INT>(
                r#"
                    let x = 21;
                    x.bar(Fn("mul"), 2);
                    x
                "#
            )
            .unwrap(),
        42
    );
}

#[test]
fn test_anonymous_fn() {
    let calc_func = Func::<(INT, INT, INT), INT>::create_from_script(Engine::new(), "fn calc(x, y, z,) { (x + y) * z }", "calc").unwrap();
    assert_eq!(calc_func(42, 123, 9).unwrap(), 1485);

    let calc_func = Func::<(INT, String, INT), INT>::create_from_script(Engine::new(), "fn calc(x, y, z) { (x + len(y)) * z }", "calc").unwrap();
    assert_eq!(calc_func(42, "hello".to_string(), 9).unwrap(), 423);

    let calc_func = Func::<(INT, String, INT), INT>::create_from_script(Engine::new(), "private fn calc(x, y, z) { (x + len(y)) * z }", "calc").unwrap();
    assert_eq!(calc_func(42, "hello".to_string(), 9).unwrap(), 423);

    let calc_func = Func::<(INT, &str, INT), INT>::create_from_script(Engine::new(), "fn calc(x, y, z) { (x + len(y)) * z }", "calc").unwrap();
    assert_eq!(calc_func(42, "hello", 9).unwrap(), 423);
}

#[test]
fn test_call_fn_events() {
    // Event handler
    struct Handler {
        // Scripting engine
        pub engine: Engine,
        // Use a custom 'Scope' to keep stored state
        pub scope: Scope<'static>,
        // Program script
        pub ast: AST,
    }

    const SCRIPT: &str = r#"
        fn start(data) { 42 + data }
        fn end(data) { 0 }
    "#;

    impl Handler {
        pub fn new() -> Self {
            let engine = Engine::new();

            // Create a custom 'Scope' to hold state
            let mut scope = Scope::new();

            // Add initialized state into the custom 'Scope'
            scope.push("state", false);

            // Compile the handler script.
            let ast = engine.compile(SCRIPT).unwrap();

            // Evaluate the script to initialize it and other state variables.
            // In a real application you'd again be handling errors...
            engine.run_ast_with_scope(&mut scope, &ast).unwrap();

            // The event handler is essentially these three items:
            Handler { engine, scope, ast }
        }

        // Say there are three events: 'start', 'end', 'update'.
        // In a real application you'd be handling errors...
        pub fn on_event(&mut self, event_name: &str, event_data: INT) -> Dynamic {
            let engine = &self.engine;
            let scope = &mut self.scope;
            let ast = &self.ast;

            match event_name {
                // The 'start' event maps to function 'start'.
                // In a real application you'd be handling errors...
                "start" => engine.call_fn(scope, ast, "start", (event_data,)).unwrap(),

                // The 'end' event maps to function 'end'.
                // In a real application you'd be handling errors...
                "end" => engine.call_fn(scope, ast, "end", (event_data,)).unwrap(),

                // The 'update' event maps to function 'update'.
                // This event provides a default implementation when the script-defined function is not found.
                "update" => engine
                    .call_fn(scope, ast, "update", (event_data,))
                    .or_else(|err| match *err {
                        EvalAltResult::ErrorFunctionNotFound(fn_name, ..) if fn_name.starts_with("update") => {
                            // Default implementation of 'update' event handler
                            self.scope.set_value("state", true);
                            // Turn function-not-found into a success
                            Ok(Dynamic::UNIT)
                        }
                        _ => Err(err),
                    })
                    .unwrap(),
                // In a real application you'd be handling unknown events...
                _ => panic!("unknown event: {}", event_name),
            }
        }
    }

    let mut handler = Handler::new();
    assert!(!handler.scope.get_value::<bool>("state").unwrap());
    let _ = handler.on_event("update", 999);
    assert!(handler.scope.get_value::<bool>("state").unwrap());
    assert_eq!(handler.on_event("start", 999).as_int().unwrap(), 1041);
}

#[test]
fn test_call_fn_with_borrowed_scope_bindings() {
    let engine = Engine::new();
    let mut scope = Scope::new();

    let ast = engine
        .compile(
            r#"
                fn update(flag) {
                    host_counter += 1;
                    if flag {
                        host_text += "!";
                    }
                }
            "#,
        )
        .unwrap();

    let mut host_counter = Dynamic::from(41 as INT);
    let mut host_text = Dynamic::from("abc");

    engine
        .call_fn_with_borrowed_scope::<()>(
            &mut scope,
            &ast,
            "update",
            (true,),
            [
                BorrowedScopeEntry::dynamic("host_counter", &mut host_counter),
                BorrowedScopeEntry::dynamic("host_text", &mut host_text),
            ],
        )
        .unwrap();

    assert_eq!(host_counter.clone_cast::<INT>(), 42);
    assert_eq!(host_text.clone_cast::<String>(), "abc!");
}

#[test]
fn test_call_fn_with_borrowed_scope_bindings_optional_usage() {
    let engine = Engine::new();
    let mut scope = Scope::new();

    let ast = engine.compile("fn maybe_use(flag) { if flag { host_data += 1; } }").unwrap();

    let mut host_data = Dynamic::from(40 as INT);

    engine
        .call_fn_with_borrowed_scope::<()>(
            &mut scope,
            &ast,
            "maybe_use",
            (false,),
            [BorrowedScopeEntry::dynamic("host_data", &mut host_data)],
        )
        .unwrap();

    assert_eq!(host_data.clone_cast::<INT>(), 40);
}

#[test]
fn test_call_fn_with_borrowed_scope_binding_conflict() {
    let engine = Engine::new();
    let mut scope = Scope::new();
    scope.push("existing", 1 as INT);

    let ast = engine.compile("fn foo() { let _x = 1; }").unwrap();

    let mut host_value = Dynamic::from(42 as INT);

    let err = engine
        .call_fn_with_borrowed_scope::<()>(
            &mut scope,
            &ast,
            "foo",
            (),
            [BorrowedScopeEntry::dynamic("existing", &mut host_value)],
        )
        .unwrap_err();

    assert!(err.to_string().contains("conflicts with an existing variable"));
}

#[test]
fn test_call_fn_with_borrowed_scope_binding_expired_access() {
    let engine = Engine::new();
    let mut scope = Scope::new();
    scope.push("saved", Dynamic::UNIT);

    let ast = engine
        .compile(
            r#"
                fn capture() {
                    saved = || borrowed + 1;
                }

                fn invoke() {
                    saved.call()
                }
            "#,
        )
        .unwrap();

    let mut host_value = Dynamic::from(41 as INT);

    engine
        .call_fn_with_options_and_borrowed_scope::<()>(
            CallFnOptions::new().rewind_scope(false),
            &mut scope,
            &ast,
            "capture",
            (),
            [BorrowedScopeEntry::dynamic("borrowed", &mut host_value)],
        )
        .unwrap();

    let err = engine.call_fn::<Dynamic>(&mut scope, &ast, "invoke", ()).unwrap_err();

    assert!(err.to_string().contains("no longer valid"));
}

#[test]
fn test_call_fn_with_borrowed_scope_bindings_with_registered_rust_fn() {
    struct NonCloneState {
        value: INT,
    }

    let mut engine = Engine::new();
    let mut scope = Scope::new();

    engine.register_borrow_fn(
        "bump_state",
        [TypeId::of::<Dynamic>()],
        |ctx, args| {
            ctx.with_borrowed_arg_mut::<NonCloneState, _>(args, 0, |state| state.value += 1)?;
            Ok(())
        },
    );
    engine.register_borrow_fn(
        "scale_add_state",
        [TypeId::of::<Dynamic>(), TypeId::of::<INT>()],
        |ctx, args| {
            let factor = ctx.arg_value::<INT>(args, 1)?;
            ctx.with_borrowed_arg_mut::<NonCloneState, _>(args, 0, |state| {
                state.value = state.value * factor + 1;
            })?;
            Ok(())
        },
    );

    let ast = engine
        .compile(
            r#"
                fn run() {
                    bump_state(host_state);
                    scale_add_state(host_state, 2);
                }
            "#,
        )
        .unwrap();

    let mut host_state = NonCloneState { value: 20 };

    engine
        .call_fn_with_borrowed_scope::<()>(
            &mut scope,
            &ast,
            "run",
            (),
            [BorrowedScopeEntry::opaque("host_state", &mut host_state)],
        )
        .unwrap();

    // ((20 + 1) * 2) + 1 = 43
    assert_eq!(host_state.value, 43);
}

#[test]
fn test_register_borrow_fn_missing_or_inactive_binding_errors() {
    struct NonCloneState {
        value: INT,
    }

    let mut engine = Engine::new();
    let mut scope = Scope::new();

    engine.register_borrow_fn("set_state", [TypeId::of::<rhai::ImmutableString>()], |ctx, args| {
        let name = args[0]
            .clone()
            .into_immutable_string()
            .map_err(|typ| {
                rhai::EvalAltResult::ErrorMismatchDataType(
                    "string".into(),
                    typ.into(),
                    rhai::Position::NONE,
                )
            })?;

        ctx.with_borrowed_mut::<NonCloneState, _>(name.as_str(), |state| state.value = 99)?;
        Ok(())
    });

    let ast = engine
        .compile(
            r#"
                fn run(name) {
                    set_state(name);
                }
            "#,
        )
        .unwrap();

    // Inactive borrowed scope.
    let err = engine
        .call_fn::<()>(&mut scope, &ast, "run", ("host_state",))
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("borrowed bindings are unavailable outside call_fn_with_borrowed_scope"));

    // Active borrowed scope but missing binding.
    let mut host_state = NonCloneState { value: 20 };
    let err = engine
        .call_fn_with_borrowed_scope::<()>(
            &mut scope,
            &ast,
            "run",
            ("unknown",),
            [BorrowedScopeEntry::opaque("host_state", &mut host_state)],
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("borrowed binding 'unknown' is not found"));
}
