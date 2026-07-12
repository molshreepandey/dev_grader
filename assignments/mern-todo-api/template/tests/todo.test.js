// Hidden tests for mern-todo-api. Students never see this file.
import { beforeEach, describe, expect, test } from "bun:test";

import { createApp } from "../src/app.js";

const BASE = "http://localhost";

let app;

/** Send a request to the student's app and return [status, parsedBody]. */
async function call(method, path, body) {
  const init = { method };
  if (body !== undefined) {
    init.headers = { "content-type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const response = await app.fetch(new Request(`${BASE}${path}`, init));
  const text = await response.text();
  return [response.status, text.length ? JSON.parse(text) : null];
}

beforeEach(() => {
  app = createApp();
});

describe("GET /todos", () => {
  test("starts empty", async () => {
    const [status, body] = await call("GET", "/todos");
    expect(status).toBe(200);
    expect(body).toEqual([]);
  });

  test("lists created todos in insertion order", async () => {
    await call("POST", "/todos", { title: "first" });
    await call("POST", "/todos", { title: "second" });

    const [status, body] = await call("GET", "/todos");
    expect(status).toBe(200);
    expect(body.map((todo) => todo.title)).toEqual(["first", "second"]);
  });
});

describe("POST /todos", () => {
  test("creates a todo with 201 and defaults done to false", async () => {
    const [status, body] = await call("POST", "/todos", { title: "write tests" });
    expect(status).toBe(201);
    expect(body.title).toBe("write tests");
    expect(body.done).toBe(false);
    expect(body.id).toBeTruthy();
  });

  test("rejects a missing title with 400", async () => {
    const [status] = await call("POST", "/todos", {});
    expect(status).toBe(400);
  });

  test("rejects an empty title with 400", async () => {
    const [status] = await call("POST", "/todos", { title: "" });
    expect(status).toBe(400);
  });

  test("gives each todo a distinct id", async () => {
    const [, first] = await call("POST", "/todos", { title: "a" });
    const [, second] = await call("POST", "/todos", { title: "b" });
    expect(first.id).not.toBe(second.id);
  });
});

describe("GET /todos/:id", () => {
  test("returns the todo", async () => {
    const [, created] = await call("POST", "/todos", { title: "read me" });
    const [status, body] = await call("GET", `/todos/${created.id}`);
    expect(status).toBe(200);
    expect(body).toEqual(created);
  });

  test("404s for an unknown id", async () => {
    const [status] = await call("GET", "/todos/does-not-exist");
    expect(status).toBe(404);
  });
});

describe("PATCH /todos/:id", () => {
  test("marks a todo done", async () => {
    const [, created] = await call("POST", "/todos", { title: "finish" });
    const [status, body] = await call("PATCH", `/todos/${created.id}`, { done: true });
    expect(status).toBe(200);
    expect(body.done).toBe(true);
    expect(body.id).toBe(created.id);
  });

  test("404s for an unknown id", async () => {
    const [status] = await call("PATCH", "/todos/nope", { done: true });
    expect(status).toBe(404);
  });
});

describe("DELETE /todos/:id", () => {
  test("removes the todo and 204s", async () => {
    const [, created] = await call("POST", "/todos", { title: "delete me" });

    const [deleteStatus] = await call("DELETE", `/todos/${created.id}`);
    expect(deleteStatus).toBe(204);

    const [getStatus] = await call("GET", `/todos/${created.id}`);
    expect(getStatus).toBe(404);
  });

  test("404s for an unknown id", async () => {
    const [status] = await call("DELETE", "/todos/nope");
    expect(status).toBe(404);
  });
});

test("unknown routes 404", async () => {
  const [status] = await call("GET", "/not-a-route");
  expect(status).toBe(404);
});

test("each app instance owns its own store", async () => {
  await call("POST", "/todos", { title: "only mine" });
  app = createApp();
  const [, body] = await call("GET", "/todos");
  expect(body).toEqual([]);
});
