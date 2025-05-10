import { Item } from "./components/item";

const items = [
  { id: "1", name: "Item A", description: "This is the first item." },
  { id: "2", name: "Item B", description: "Another useful item." },
  { id: "3", name: "Item C", description: "Yet another item here." },
] as const;

function App() {
  return (
    <>
      <h1>SPA Application</h1>
      <h2>My Items</h2>
      <ul style={{ listStyleType: "none", padding: 0 }}>
        {items.map((item) => (
          <Item {...item} />
        ))}
      </ul>
    </>
  );
}

export default App;
