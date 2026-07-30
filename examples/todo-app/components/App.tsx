// @runsOn client
// Shared signal to coordinate sibling communication
import { TodoList } from './TodoList';
import { AddTodoForm } from './AddTodoForm';

type AppProps = {};

export function App(props: AppProps) {
  const todos = signal([]);

  return (
    <div class="app">
      <h1>Todo App</h1>
      <AddTodoForm todos={todos} />
      <TodoList todos={todos} />
    </div>
  );
}
