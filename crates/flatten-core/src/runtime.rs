// crates/flatten-core/src/runtime.rs

#[cfg(test)]
mod spike {
    use rquickjs::{Context, Module, Runtime};
    use std::time::{Duration, Instant};

    /// Proves the embedded JS runtime can evaluate a module from a source string and return values to Rust.
    #[test]
    fn module_eval() {
        let runtime: Runtime = Runtime::new().unwrap();

        let ctx: Context = Context::full(&runtime).unwrap();
        ctx.with(|ctx| {
            let source: &str = "export const value = 1 + 2;";
            let module: Module = Module::declare(ctx.clone(), "test.js", source).unwrap();
            let (module, _promise) = module.eval().unwrap();
            let ns = module.namespace().unwrap();
            let value: i32 = ns.get("value").unwrap();

            assert_eq!(value, 3)
        });
    }

    /// Proves the interrupt handler can terminate a runaway script within a bounded time.
    #[test]
    fn interrupt_timeout_script() {
        let runtime: Runtime = Runtime::new().unwrap();

        let delta: Instant = Instant::now();
        let limit: Duration = Duration::from_millis(100);
        runtime.set_interrupt_handler(Some(Box::new(move || delta.elapsed() > limit)));

        let ctx: Context = Context::full(&runtime).unwrap();
        ctx.with(|ctx| {
            let result = ctx.eval::<rquickjs::Value, _>("for(;;) {}");
            assert!(result.is_err());
        });
        assert!(delta.elapsed() < Duration::from_millis(500));
    }

    /// Proves the interrupt handler can terminate a runaway module within a bounded time.
    #[test]
    fn interrupt_timeout_module() {
        let runtime: Runtime = Runtime::new().unwrap();

        let delta: Instant = Instant::now();
        let limit: Duration = Duration::from_millis(100);
        runtime.set_interrupt_handler(Some(Box::new(move || delta.elapsed() > limit)));

        let ctx = Context::full(&runtime).unwrap();
        ctx.with(|ctx| {
            let source = "for(;;) {}";
            let module = Module::declare(ctx.clone(), "test.js", source).unwrap();
            let result = module.eval();
            match result {
                Ok((_module, promise)) => {
                    let resolved: Result<rquickjs::Value, _> = promise.finish();
                    assert!(resolved.is_err());
                    assert!(delta.elapsed() < Duration::from_millis(500));
                }
                Err(_) => {
                    panic!("Interrupt surfaced as a direct Err, not via promise");
                }
            }
        });
    }

    /// Proves the memory limit prevents a script from allocating unbounded memory.
    #[test]
    fn memory_limit_module() {
        let runtime: Runtime = Runtime::new().unwrap();

        runtime.set_memory_limit(1024 * 1024);

        let ctx: Context = Context::full(&runtime).unwrap();
        ctx.with(|ctx| {
            let source =
                "let a = []; for(let i = 0; i < 100000; i++) { a.push('x'.repeat(1000)); }";
            let module = Module::declare(ctx.clone(), "test.js", source).unwrap();
            let result = module.eval();
            match result {
                Ok((_module, promise)) => {
                    let resolved: Result<rquickjs::Value, _> = promise.finish();
                    assert!(resolved.is_err());
                }
                Err(_) => {
                    panic!("Memory limit surfaced as a direct Err, not via promise")
                }
            }
        })
    }
}
