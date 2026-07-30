// @runsOn client
import "./AddTodoForm.css";
type AddTodoFormProps = { todos: any };

export function AddTodoForm(props: AddTodoFormProps) {
  const inputText = signal('');

  function handleSubmit(e) {
    e.preventDefault();
    const text = inputText.value.trim();
    if (text) {
      const newItem = { id: Date.now(), text };
      props.todos.set([...props.todos.value, newItem]);
      inputText.set('');
    }
  }

  return (
    <form class="add-form" onSubmit={handleSubmit}>
      <input class="todo-input" type="text"
        value={inputText.value}
        onInput={(e) => inputText.set(e.target.value)}
      />
      <button type="submit">Add</button>
    </form>
  );
}
