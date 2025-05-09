export const Item = (props: {
  id: string;
  name: string;
  description: string;
}) => {
  return (
    <li
      style={{
        border: "1px solid #ccc",
        padding: "12px",
        borderRadius: "6px",
        marginBottom: "8px",
      }}
    >
      <div style={{ fontWeight: "600" }}>{props.name}</div>
      <div style={{ fontSize: "14px", color: "#555" }}>{props.description}</div>
    </li>
  );
};
