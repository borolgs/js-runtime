import type { PropsWithChildren } from "react";

export function ServerLayout(
  props: PropsWithChildren<{ head?: string; title: string }>,
) {
  return (
    <>
      {"<!doctype html>"}
      <html lang="en">
        <head>
          <meta charSet="UTF-8" />
          <meta
            name="viewport"
            content="width=device-width, initial-scale=1.0"
          />
          <title>{props.title}</title>
          {props.head}
        </head>
        <body>{props.children}</body>
      </html>
    </>
  );
}
