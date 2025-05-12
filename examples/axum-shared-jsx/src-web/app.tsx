import { createRouter, RouterProvider } from "@tanstack/react-router";
import { rootRoute } from "./pages/root";
import { indexRoute } from "./pages/home";
import { aboutRoute } from "./pages/about";

const routeTree = rootRoute.addChildren([indexRoute, aboutRoute]);

const router = createRouter({ routeTree, basepath: "/app" });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

export function App() {
  return <RouterProvider router={router} />;
}
