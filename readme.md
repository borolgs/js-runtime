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

[Render jsx](./examples/axum-jsx/src/main.rs):

```rust
let runtime = js::Runtime::new(js::RuntimeConfig {
    js_src: Some(include_dir::include_dir!("$CARGO_MANIFEST_DIR/src-js")),
    ..Default::default()
});

async fn items(runtime: js::Runtime) -> impl IntoResponse {
    let items = json!({
        "items": [
            { "id": 1, "name": "Item A", "description": "This is the first item." },
            { "id": 2, "name": "Item B", "description": "Another useful item." },
            { "id": 3, "name": "Item C", "description": "Yet another item here." }
        ]
    });
    runtime
        // src-js/pages/page_name.tsx
        .render(Some(items), "page_name")
        .await
        .into_response()
}
```
