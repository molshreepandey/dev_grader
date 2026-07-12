// Sample test — a taste of what the grader checks. Yours to edit; not graded.
import { expect, test } from "bun:test";

import { createApp } from "../src/app.js";

test("GET /todos starts empty", async () => {
  const app = createApp();
  const response = await app.fetch(new Request("http://localhost/todos"));
  expect(response.status).toBe(200);
  expect(await response.json()).toEqual([]);
});
