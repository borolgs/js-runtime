# JS Runtime

A tiny wrapper around [quickjs-rusty](https://github.com/Icemic/quickjs-rusty).

```rust
let runtime = js::Runtime::new(js::RuntimeConfig {
    workers: 10,
    ..Default::default()
});

let res = runtime
    .execute_script(Script::Function {
        code: "console.log('hello!'); 2 + 2".into(),
        args: None,
    })
    .await
    .unwrap();

println!("{}", res.output); // 4
println!("{}", (res.console_output); // hello!
```
