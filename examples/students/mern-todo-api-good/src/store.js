/** In-memory todo storage. One store per app instance. */
export class TodoStore {
  #todos = new Map();
  #nextId = 1;

  list() {
    return [...this.#todos.values()];
  }

  create(title) {
    const todo = { id: String(this.#nextId++), title, done: false };
    this.#todos.set(todo.id, todo);
    return todo;
  }

  get(id) {
    return this.#todos.get(id) ?? null;
  }

  update(id, patch) {
    const existing = this.#todos.get(id);
    if (!existing) return null;
    const updated = { ...existing, ...patch };
    this.#todos.set(id, updated);
    return updated;
  }

  remove(id) {
    return this.#todos.delete(id);
  }
}
