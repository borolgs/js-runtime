({ items }) => {
  return (
    <div>
      <h1>My Items</h1>
      <a href="/">back</a>
      <ul style={{ listStyleType: "none", padding: 0 }}>
        {items.map((item) => (
          <li
            key={item.id}
            style={{
              border: "1px solid #ccc",
              padding: "12px",
              borderRadius: "6px",
              marginBottom: "8px",
            }}
          >
            <div style={{ fontWeight: "600" }}>{item.name}</div>
            <div style={{ fontSize: "14px", color: "#555" }}>
              {item.description}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
};
