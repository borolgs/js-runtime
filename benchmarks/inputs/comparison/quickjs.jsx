import { Footer } from "./quickjs_footer.jsx";

export const Page = (args) => {
  return (
    <body>
      <header>
        <h1>Hello</h1>
        <nav>
          <ul class="nav">
            {args.site.nav.map((item) => (
              <li>
                <a href={item.url} className={item.is_active ? "active" : ""}>
                  {item.title.toUpperCase()}
                </a>
              </li>
            ))}
          </ul>
        </nav>
      </header>
      <main>
        <ul>
          {args.items.map((item, i) => (
            <li>
              {i}: {item.toUpperCase()}
            </li>
          ))}
        </ul>
      </main>
      <footer>
        <Footer copyright={args.site.copyright} />
      </footer>
    </body>
  );
};
