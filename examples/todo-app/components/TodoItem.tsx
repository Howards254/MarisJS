// @runsOn client
// Per-item component with its own internal state (editing signal).
// This state must survive when other todos are added/removed.
import "./TodoItem.css";
type TodoItemProps = { item: any };

export function TodoItem(props: TodoItemProps) {
  const editing = signal(false);

  return (
    <li class="todo-item" data-id={props.item.id}>
      <span class="todo-text">
        {editing.value ? 'Editing: ' : ''}{props.item.text}
      </span>
      <button
        class="edit-btn"
        onClick={() => editing.set(!editing.value)}
      >
        {editing.value ? 'Save' : 'Edit'}
      </button>
    </li>
  );
}
