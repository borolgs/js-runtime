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
