// A deliberately broken solution, used to exercise the failure-reporting path.
//
// Defects: POST answers 200 instead of 201, no input validation (so a missing or empty title is
// accepted rather than rejected with 400), and DELETE answers 200 instead of 204.
import { TodoStore } from "./store.js";

const json = (body, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });

const notFound = () => json({ error: "not found" }, 404);

export function createApp() {
  const store = new TodoStore();

  return {
    async fetch(request) {
      const { pathname } = new URL(request.url);
      const segments = pathname.split("/").filter(Boolean);

      if (segments[0] !== "todos") return notFound();
      const id = segments[1];

      if (!id) {
        if (request.method === "GET") return json(store.list());
        if (request.method === "POST") {
          const body = await request.json().catch(() => ({}));
          return json(store.create(body.title));
        }
        return notFound();
      }

      if (request.method === "GET") {
        const todo = store.get(id);
        return todo ? json(todo) : notFound();
      }

      if (request.method === "PATCH") {
        const body = await request.json().catch(() => ({}));
        const updated = store.update(id, body);
        return updated ? json(updated) : notFound();
      }

      if (request.method === "DELETE") {
        return store.remove(id) ? json({ deleted: id }) : notFound();
      }

      return notFound();
    },
  };
}
