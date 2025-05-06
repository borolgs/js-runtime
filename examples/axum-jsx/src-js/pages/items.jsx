import { Item } from "../components/item.jsx";

export const ItemsPage = ({ items }) => {
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
