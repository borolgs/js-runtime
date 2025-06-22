import { ServerLayout } from "../../components/server-layout.tsx";

export default () => (
  <ServerLayout title="My App">
    <div>
      <h1>My App</h1>
      <a href="/items">My Items</a>
      <br />
      <a href="/app">My App</a>
    </div>
  </ServerLayout>
);
