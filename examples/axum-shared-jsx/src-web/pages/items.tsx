import { Item } from "../components/item.tsx";
import { ServerLayout } from "../components/server-layout.tsx";

export default ({ items }: { items: any[] }) => {
  return (
    <ServerLayout title="My Items">
      <h1>My Items</h1>
      <a href="/">back</a>
      <ul style={{ listStyleType: "none", padding: 0 }}>
        {items.map((item) => (
          <Item {...item} />
        ))}
      </ul>
    </ServerLayout>
  );
};
