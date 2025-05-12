import { createRootRoute, Link, Outlet } from "@tanstack/react-router";
import { TanStackRouterDevtools } from "@tanstack/react-router-devtools";

export const rootRoute = createRootRoute({
  component: () => (
    <>
      <div style={{ display: "flex", gap: "5px" }}>
        <a href="/">back</a>
        <Link to="/">app</Link>
        <Link to="/about">about</Link>
      </div>
      <hr />
      <Outlet />
      <TanStackRouterDevtools />
    </>
  ),
});
