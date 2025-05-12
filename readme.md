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

## Render JSX

- [Simple](examples/axum-simple-jsx) – a basic example of server-side rendering
- [With React](examples/axum-shared-jsx) – a hybrid app: a SPA on one route and traditional server-rendered pages on the others, with component sharing

```rust
let runtime = js::Runtime::new(js::RuntimeConfig {
    js_src: Some(include_dir::include_dir!("$CARGO_MANIFEST_DIR/src-web")),
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
        // src-web/pages/items.tsx
        .render(Some(items), "items")
        .await
        .into_response()
}
```

For server pages, use default exports.

These pages are rendered using a vendored version of [@kitajs/html](https://github.com/kitajs/html),
so the React Hook API is not available in this context.

```tsx
// src-web/pages/items.tsx

import { Item } from "../components/item.tsx";

export default ({ items }: { items: any[] }) => {
  return (
    <div>
      <h1>My Items</h1>
      <a href="/">back</a>
      <ul style={{ listStyleType: "none", padding: 0 }}>
        {items.map((item) => (
          <Item {...item} />
        ))}
      </ul>
    </div>
  );
};
```
