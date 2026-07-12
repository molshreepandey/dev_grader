import { z } from "zod";

import { TodoStore } from "./store.js";

const createTodo = z.object({ title: z.string().min(1) });
const patchTodo = z
  .object({ title: z.string().min(1).optional(), done: z.boolean().optional() })
  .refine((patch) => Object.keys(patch).length > 0, { message: "empty patch" });

const json = (body, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });

const notFound = () => json({ error: "not found" }, 404);
const badRequest = (message) => json({ error: message }, 400);

/** Parse a JSON body, returning `undefined` when it is absent or malformed. */
async function readJson(request) {
  try {
    return await request.json();
  } catch {
    return undefined;
  }
}

export function createApp() {
  const store = new TodoStore();

  return {
    async fetch(request) {
      const { pathname } = new URL(request.url);
      const segments = pathname.split("/").filter(Boolean);

      if (segments[0] !== "todos" || segments.length > 2) return notFound();
      const id = segments[1];

      if (!id) {
        if (request.method === "GET") return json(store.list());

        if (request.method === "POST") {
          const parsed = createTodo.safeParse(await readJson(request));
          if (!parsed.success) return badRequest("title is required");
          return json(store.create(parsed.data.title), 201);
        }

        return notFound();
      }

      if (request.method === "GET") {
        const todo = store.get(id);
        return todo ? json(todo) : notFound();
      }

      if (request.method === "PATCH") {
        const parsed = patchTodo.safeParse(await readJson(request));
        if (!parsed.success) return badRequest("invalid patch");
        const updated = store.update(id, parsed.data);
        return updated ? json(updated) : notFound();
      }

      if (request.method === "DELETE") {
        return store.remove(id) ? new Response(null, { status: 204 }) : notFound();
      }

      return notFound();
    },
  };
}
