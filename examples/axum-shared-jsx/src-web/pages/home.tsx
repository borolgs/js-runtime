import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./root";

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: function Index() {
    return (
      <div>
        <h1>SPA Application</h1>
        <div>This is SPA!</div>
      </div>
    );
  },
});
