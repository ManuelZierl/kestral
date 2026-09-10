import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import test from "node:test";
import vm from "node:vm";

const html = await readFile(new URL("../src/ui/index.html", import.meta.url), "utf8");
const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert.ok(script, "test the actual packaged surface script, not a second implementation");
const appId = "com.ma-zierl.daily-review";
const day = "2026-09-08";
const id = number => `00000000-0000-4000-8000-${String(number).padStart(12, "0")}`;
const task = (number, title = `Task ${number}`, parentId = null, done = false) => ({
  id: id(number), revision: 1, createdAt: `${day}T12:00:00Z`, updatedAt: `${day}T12:00:00Z`,
  value: { title, parentId, done, day },
});
const saved = (body = "Saved note", onDay = day) => ({
  id: id(900), revision: 1, createdAt: `${day}T12:00:00Z`, updatedAt: `${day}T12:00:00Z`,
  value: { day: onDay, body },
});
const proposal = (record, generation = 1, overrides = {}) => ({
  artifact_type: "task-title-proposal",
  content: {
    targetAppId: appId, targetKind: "record", collection: "tasks",
    resourceId: `app-data:${appId}:tasks:record:${record.id}`,
    targetRevision: record.revision, targetGeneration: generation,
    payload: { title: "Proposed title" }, ...overrides,
  },
});
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};

// Small DOM/event adapter keeps the standalone app tests dependency-free.
// Only the app's host boundary is faked; all handlers, reads, CAS arguments,
// rendering, error recovery, and draft transitions execute the real script.
class Element {
  constructor(tag = "div") {
    this.tagName = tag; this.children = []; this.listeners = new Map();
    this.style = {}; this.dataset = {}; this.attributes = {};
    this.value = ""; this.textContent = ""; this.className = "";
    this.hidden = false; this.disabled = false; this.checked = false;
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(key, value) { this.attributes[key] = value; }
  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(listener);
  }
  async fire(type, properties = {}) {
    const event = { defaultPrevented: false, preventDefault() { this.defaultPrevented = true; }, ...properties };
    if (!this.disabled) await Promise.all((this.listeners.get(type) || []).map(listener => listener(event)));
    return event;
  }
}

async function launch(options = {}) {
  const elements = new Map();
  for (const [, tag, attrs, name] of html.matchAll(/<([a-z][\w-]*)\b([^>]*\bid="([^"]+)"[^>]*)>/g)) {
    const element = new Element(tag);
    element.hidden = /\bhidden\b/.test(attrs);
    element.disabled = /\bdisabled\b/.test(attrs);
    elements.set(`#${name}`, element);
  }
  const select = selector => {
    assert.ok(elements.has(selector), `unknown element ${selector}`);
    return elements.get(selector);
  };
  const all = () => {
    const nodes = [];
    const walk = node => { nodes.push(node); node.children.forEach(walk); };
    elements.forEach(walk);
    return nodes;
  };
  const document = {
    querySelector: select,
    createElement: tag => new Element(tag),
    querySelectorAll: selector => {
      const [className, tag] = selector.split(" ");
      return all().filter(node => node.className.split(" ").includes(className.slice(1)))
        .flatMap(node => node.children.filter(child => child.tagName === tag));
    },
  };
  const state = {
    tasks: structuredClone(options.tasks || [task(1)]),
    notes: structuredClone(options.notes || [saved()]),
    artifacts: structuredClone(options.artifacts || []), generation: 1,
  };
  const hooks = options.hooks || {};
  const calls = { reads: [], writes: [], invokes: [], errors: [] };
  let init, clock = `${day}T12:00:00`, nextId = 20000;
  const timers = [], windowEvents = new Map();
  const appHost = {
    onInit: cb => { init = cb; }, ready() {}, reportError: message => calls.errors.push(message),
    listArtifacts: async () => { if (hooks.artifacts) await hooks.artifacts(); return structuredClone(state.artifacts); },
    invoke: async (...args) => {
      calls.invokes.push(structuredClone(args));
      return hooks.invoke ? hooks.invoke() : { result: { kind: "completed", result: { message: { content: "- Plan tomorrow" } } } };
    },
    data: { v2: {
      readSnapshot: async request => {
        calls.reads.push(structuredClone(request));
        if (hooks.read) {
          const override = await hooks.read(request, state);
          if (override) return override;
        }
        if (request.expectedGeneration !== undefined && request.expectedGeneration !== state.generation) throw Error("generation conflict");
        return {
          generation: state.generation,
          results: request.reads.map(read => {
            assert.equal(read.kind, "record-list");
            const query = read.query || {};
            let records = [...state[read.collection]].sort((a, b) => a.id.localeCompare(b.id));
            if (query.index) {
              assert.equal(query.index, "day");
              records = records.filter(record => record.value.day === query.equals);
            }
            if (query.after) records = records.filter(record => record.id > query.after);
            const page = records.slice(0, query.limit);
            return { kind: "record-list", records: structuredClone(page), nextAfter: records.length > page.length ? page.at(-1).id : null };
          }),
        };
      },
    } },
  };
  for (const operation of ["create", "replace"]) appHost.data.v2[operation] = async request => {
    calls.writes.push({ operation, ...structuredClone(request) });
    if (hooks.write) await hooks.write(request, state);
    if (request.expectedGeneration !== state.generation) throw Error("generation conflict");
    const records = state[request.collection];
    if (operation === "create") {
      if (request.collection === "notes" && records.some(record => record.value.day === request.value.day)) throw Error("day must be unique");
      records.push({ id: id(nextId++), revision: 1, createdAt: clock, updatedAt: clock, value: structuredClone(request.value) });
    } else {
      const record = records.find(record => record.id === request.id);
      if (!record || record.revision !== request.expectedRevision) throw Error("revision conflict");
      record.value = structuredClone(request.value);
      record.revision++;
    }
    state.generation++;
    return { generation: state.generation };
  };
  const window = { appHost, addEventListener: (name, cb) => windowEvents.set(name, cb) };
  const context = vm.createContext({ document, window, crypto: { randomUUID }, console,
    setInterval: cb => timers.push(cb),
    Date: class extends Date { constructor(...args) { super(...(args.length ? args : [clock])); } },
  });
  vm.runInContext(script, context, { filename: "daily-review/src/ui/index.html" });
  if (options.initialize !== false) await init();
  return {
    state, hooks, calls, select, init,
    click: name => select(name).fire("click"),
    type: async (name, text) => { select(name).value = text; await select(name).fire("input"); },
    submit: () => select("#task-form").fire("submit"),
    tick: async value => {
      clock = value;
      timers.forEach(cb => cb());
      await new Promise(resolve => setImmediate(resolve));
    },
  };
}
const titles = element => element.children.map(row => row.children[1].textContent);

test("task changes and refresh preserve unsaved notes", async () => {
  const h = await launch();
  await h.type("#note", "Unsaved draft");
  await h.type("#task-input", "- [ ] Another task");
  await h.submit();
  assert.equal(h.select("#note").value, "Unsaved draft");
  assert.equal(h.select("#preview").textContent, "Saved note");
  assert.equal(h.state.tasks.at(-1).value.title, "Another task");
  await h.click("#refresh");
  assert.equal(h.select("#note").value, "Unsaved draft");
});

test("failed save preserves the draft and its old revision even after refresh", async () => {
  const h = await launch();
  await h.type("#note", "My local draft");
  h.state.notes[0].value.body = "Other window's note";
  h.state.notes[0].revision++;
  h.state.generation++;
  await h.click("#save-note");
  assert.equal(h.select("#note").value, "My local draft");
  assert.match(h.select("#status").textContent, /generation conflict/);
  await h.click("#refresh");
  assert.equal(h.select("#preview").textContent, "Other window's note");
  await h.click("#save-note");
  assert.equal(h.calls.writes.at(-1).expectedRevision, 1);
  assert.match(h.select("#status").textContent, /revision conflict/);
  assert.equal(h.state.notes[0].value.body, "Other window's note");
  await h.click("#discard-note");
  assert.equal(h.select("#note").value, "Other window's note");
});

test("typing during a note save is kept and rebased only onto our acknowledged revision", async () => {
  const h = await launch();
  const gate = deferred();
  h.hooks.write = () => gate.promise;
  await h.type("#note", "Submitted body");
  const saving = h.click("#save-note");
  await h.type("#note", "Newer unsaved edits");
  gate.resolve();
  await saving;
  assert.equal(h.state.notes[0].value.body, "Submitted body");
  assert.equal(h.select("#note").value, "Newer unsaved edits");
  delete h.hooks.write;
  await h.click("#save-note");
  assert.equal(h.calls.writes.at(-1).expectedRevision, 2);
  assert.equal(h.state.notes[0].value.body, "Newer unsaved edits");
});

test("a failed initial read cannot create records with a fabricated generation", async () => {
  const h = await launch({ hooks: { read: () => { throw Error("offline"); } } });
  await h.type("#task-input", "Do not create");
  await h.submit();
  assert.equal(h.calls.writes.length, 0);
  assert.match(h.select("#status").textContent, /offline/);
});

test("a committed task is not reported as failed when its follow-up refresh fails", async () => {
  const h = await launch();
  await h.type("#task-input", "Committed once");
  h.hooks.read = () => { throw Error("refresh unavailable"); };
  await h.submit();
  assert.equal(h.state.tasks.length, 2);
  assert.equal(h.select("#task-input").value, "");
  assert.match(h.select("#status").textContent, /Task added, but refresh failed; do not repeat/);
  await h.submit();
  assert.equal(h.calls.writes.length, 1);
});

test("task creation neither clears newer typing nor submits competing writes", async () => {
  const h = await launch(), gate = deferred();
  h.hooks.write = () => gate.promise;
  await h.type("#task-input", "First task");
  const creating = h.submit();
  await h.type("#task-input", "Next task draft");
  await h.submit();
  gate.resolve();
  await creating;
  assert.equal(h.select("#task-input").value, "Next task draft");
  assert.equal(h.calls.writes.length, 1);
});

test("all task pages are read at one generation and today's note uses the unique day index", async () => {
  const records = Array.from({ length: 105 }, (_, n) => task(n + 1));
  const history = Array.from({ length: 105 }, (_, n) => ({ ...saved("History", `2025-01-${n}`), id: id(1000 + n) }));
  const h = await launch({ tasks: records, notes: [...history, saved("Today's note")] });
  assert.equal(h.select("#tasks").children.length, 105);
  assert.equal(h.select("#note").value, "Today's note");
  assert.deepEqual(h.calls.reads[0].reads[1].query, { index: "day", equals: day, limit: 1 });
  assert.equal(h.calls.reads[1].expectedGeneration, 1);
  assert.equal(h.calls.reads[1].reads[0].query.after, id(100));
});

test("a write between task pages never publishes a partial or mixed snapshot", async () => {
  const h = await launch();
  await h.type("#note", "Keep me");
  h.state.tasks = Array.from({ length: 101 }, (_, n) => task(n + 1));
  h.hooks.read = (request, state) => { if (request.expectedGeneration !== undefined) state.generation++; };
  await h.click("#refresh");
  assert.deepEqual(titles(h.select("#tasks")), ["Task 1"]);
  assert.equal(h.select("#note").value, "Keep me");
  assert.match(h.select("#status").textContent, /generation conflict/);
});

test("a non-progressing page cursor is rejected rather than looping forever", async () => {
  const h = await launch();
  h.hooks.read = request => ({ generation: 1, results: request.reads.map(read => ({
    kind: "record-list", records: read.collection === "tasks" ? [task(1)] : [],
    nextAfter: read.collection === "tasks" ? id(1) : null,
  })) });
  await h.click("#refresh");
  assert.match(h.select("#status").textContent, /pagination cursor/);
});

test("hierarchies render parent-first and archive only when the root is complete", async () => {
  const h = await launch({ tasks: [task(1, "Grandchild", id(2)), task(2, "Child", id(3), true), task(3, "Root")] });
  assert.deepEqual(titles(h.select("#tasks")), ["Root", "Child", "Grandchild"]);
  assert.equal(h.select("#tasks").children[2].style.marginLeft, "1.8rem");
  assert.equal(h.select("#archive").children.length, 0);
  const checkbox = h.select("#tasks").children[0].children[0];
  checkbox.checked = true;
  await checkbox.fire("change");
  assert.deepEqual(titles(h.select("#archive")), ["Root", "Child", "Grandchild"]);
  assert.equal(h.select("#tasks").children.length, 0);
});

test("Tab creates a child, but Shift+Tab and empty Tab retain normal focus navigation", async () => {
  const h = await launch();
  await h.select("#tasks").children[0].children[1].fire("click");
  await h.type("#task-input", "Child task");
  const shift = await h.select("#task-input").fire("keydown", { key: "Tab", shiftKey: true });
  assert.equal(shift.defaultPrevented, false);
  assert.equal(h.calls.writes.length, 0);
  const tab = await h.select("#task-input").fire("keydown", { key: "Tab", shiftKey: false });
  assert.equal(tab.defaultPrevented, true);
  assert.equal(h.state.tasks.at(-1).value.parentId, id(1));
  const empty = await h.select("#task-input").fire("keydown", { key: "Tab" });
  assert.equal(empty.defaultPrevented, false);
});

test("refreshing proposals also refreshes the target revisions and rejects stale cards", async () => {
  const h = await launch({ artifacts: [proposal(task(1))] });
  assert.equal(h.select("#proposals").children[0].children[2].disabled, false);
  h.state.tasks[0].revision++;
  h.state.tasks[0].value.title = "Changed elsewhere";
  h.state.generation++;
  await h.click("#refresh-proposals");
  const apply = h.select("#proposals").children[0].children[2];
  assert.equal(apply.disabled, true);
  await apply.fire("click");
  assert.equal(h.calls.writes.length, 0);
});

test("a race after proposal display still uses the artifact's generation and revision", async () => {
  const h = await launch({ artifacts: [proposal(task(1))] });
  await h.type("#note", "Local draft");
  h.state.tasks[0].revision++;
  h.state.tasks[0].value.title = "Concurrent edit";
  h.state.generation++;
  await h.select("#proposals").children[0].children[2].fire("click");
  assert.equal(h.calls.writes[0].expectedGeneration, 1);
  assert.equal(h.calls.writes[0].expectedRevision, 1);
  assert.equal(h.state.tasks[0].value.title, "Concurrent edit");
  assert.equal(h.select("#note").value, "Local draft");
  assert.match(h.select("#status").textContent, /generation conflict/);
});

test("proposal identifiers must bind this app's exact task collection", async () => {
  const h = await launch({ artifacts: [proposal(task(1), 1, { resourceId: `app-data:other:tasks:record:${id(1)}` })] });
  assert.equal(h.select("#proposals").children.length, 0);
  assert.equal(h.calls.writes.length, 0);
});

test("AI denial leaves local work usable; successful suggestions are staged, not auto-applied", async () => {
  const h = await launch({ tasks: [task(1), task(2, "Visible child", id(1))] });
  h.hooks.invoke = () => ({ result: { kind: "refused", reason: "approval-denied" } });
  await h.click("#suggest");
  assert.match(h.select("#status").textContent, /approval-denied/);
  assert.equal(h.calls.writes.length, 0);
  delete h.hooks.invoke;
  await h.click("#suggest");
  assert.equal(h.select("#draft").hidden, false);
  assert.equal(h.calls.writes.length, 0);
  assert.match(h.calls.invokes.at(-1)[1].messages[1].content, /Visible child/);
  assert.equal(h.select("#ai-context").textContent, h.calls.invokes.at(-1)[1].messages[1].content);
  await h.type("#task-input", "Unrelated task draft");
  await h.click("#accept");
  assert.equal(h.select("#task-input").value, "Unrelated task draft");
  assert.equal(h.state.tasks.at(-1).value.title, "Plan tomorrow");
  assert.equal(h.select("#draft").hidden, true);
});

test("proposal read failures do not hide the error or disable local notes", async () => {
  const h = await launch({ hooks: { artifacts: () => { throw Error("artifact service unavailable"); } } });
  assert.match(h.select("#proposal-status").textContent, /artifact service unavailable/);
  await h.type("#note", "Still useful offline");
  await h.click("#save-note");
  assert.equal(h.state.notes[0].value.body, "Still useful offline");
  assert.match(h.select("#proposal-status").textContent, /artifact service unavailable/);
});

test("midnight advances clean notes but never attaches an unsaved draft to a new date", async () => {
  const h = await launch();
  await h.type("#note", "Yesterday's draft");
  await h.tick("2026-09-09T00:01:00");
  assert.match(h.select("#date").textContent, /2026-09-08.*unsaved/);
  assert.equal(h.select("#note").value, "Yesterday's draft");
  await h.click("#save-note");
  assert.equal(h.state.notes[0].value.day, "2026-09-08");
  assert.equal(h.state.notes[0].value.body, "Yesterday's draft");
  assert.equal(h.select("#date").textContent, "2026-09-09");
  assert.equal(h.select("#note").value, "");
});

test("failed checkbox writes restore the stored check state without discarding the note", async () => {
  const h = await launch();
  await h.type("#note", "Preserve me");
  h.hooks.write = () => { throw Error("disk full"); };
  const checkbox = h.select("#tasks").children[0].children[0];
  checkbox.checked = true;
  await checkbox.fire("change");
  assert.equal(h.select("#tasks").children[0].children[0].checked, false);
  assert.equal(h.select("#note").value, "Preserve me");
  assert.match(h.select("#status").textContent, /disk full/);
});

test("an archived or removed selected parent cannot hide a newly entered child", async () => {
  const h = await launch();
  await h.select("#tasks").children[0].children[1].fire("click");
  const checkbox = h.select("#tasks").children[0].children[0];
  checkbox.checked = true;
  await checkbox.fire("change");
  await h.type("#task-input", "New draft");
  const tab = await h.select("#task-input").fire("keydown", { key: "Tab" });
  assert.equal(tab.defaultPrevented, false);
  assert.equal(h.calls.writes.length, 1);
  await h.submit();
  assert.equal(h.state.tasks.at(-1).value.parentId, null);
});


test("typing during a midnight refresh keeps the visible note's original date", async () => {
  const h = await launch(), gate = deferred();
  h.hooks.read = async request => {
    if (request.reads.some(read => read.collection === "notes" && read.query.equals === "2026-09-09")) {
      await gate.promise;
    }
  };
  await h.tick("2026-09-09T00:01:00");
  await h.type("#note", "Edits started before the new date appeared");
  gate.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.match(h.select("#date").textContent, /2026-09-08.*unsaved/);
  assert.equal(h.select("#note").value, "Edits started before the new date appeared");
  assert.equal(h.calls.reads.at(-1).reads[1].query.equals, "2026-09-08");
  delete h.hooks.read;
  await h.click("#save-note");
  assert.equal(h.calls.writes.at(-1).value.day, "2026-09-08");
  assert.equal(h.state.notes[0].value.day, "2026-09-08");
  assert.equal(h.state.notes[0].value.body, "Edits started before the new date appeared");
});
