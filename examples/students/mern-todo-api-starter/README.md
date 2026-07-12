# Assignment: Todo API (`mern-todo-api`)

Build the todo API in `src/`. Push this repository to **public** GitHub and submit its URL.

## What you must implement

`src/app.js` exports `createApp()`, which returns an object with a `fetch(request)` method — the
same shape `Bun.serve` expects, so the API is exercised by passing it a `Request` and inspecting
the `Response`. No socket is ever opened. Storage lives in `src/store.js`, in memory, and **each
`createApp()` gets its own store**.

| Route | Behaviour |
|---|---|
| `GET /todos` | `200` with the todos as a JSON array, in insertion order. Empty at first. |
| `POST /todos` | Body `{ "title": "..." }` → `201` with `{ id, title, done: false }`. Ids are unique. A missing or empty title → `400`. |
| `GET /todos/:id` | `200` with the todo, or `404`. |
| `PATCH /todos/:id` | Body `{ "done": true }` (or `{ "title": "..." }`) → `200` with the updated todo, or `404`. |
| `DELETE /todos/:id` | `204` with no body, or `404`. |
| anything else | `404`. |

`zod` is available for validation (it is the only dependency, and `package.json` is fixed by the
instructor — you cannot add libraries).

## How you are graded

Your whole repository is graded, but the instructor's `tests/` directory and `package.json` are
**stamped over** whatever you push: your copies of those are discarded, and a hidden test suite is
run against your `src/`. The sample test in `tests/` is for your own feedback only.

Run it locally with:

```bash
bun install
bun test
```
