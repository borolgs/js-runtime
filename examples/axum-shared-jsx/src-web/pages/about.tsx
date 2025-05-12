import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./root";

export const aboutRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/about",
  component: function About() {
    return (
      <>
        <h1>SPA Application</h1>
        <div>Hello from About!</div>
      </>
    );
  },
});
