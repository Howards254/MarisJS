// @runsOn client
// Receives the shared todos signal and renders a <For> list
import { TodoItem } from './TodoItem';

type TodoListProps = { todos: any };

export function TodoList(props: TodoListProps) {
  return (
    <ul class="todo-list">
      <For each={props.todos.value} key={(t) => t.id}>
        {(todo) => <TodoItem item={todo} />}
      </For>
    </ul>
  );
}
