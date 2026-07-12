import { TodoStore } from "./store.js";

/**
 * Build a todo API.
 *
 * Returns an object with a `fetch(request)` method — the same shape Bun.serve expects — so the
 * whole API can be exercised without opening a socket.
 */
export function createApp() {
  const store = new TodoStore();

  return {
    async fetch(request) {
      throw new Error("not implemented");
    },
  };
}
