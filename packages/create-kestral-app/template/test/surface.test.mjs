import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import test from "node:test";

class Element {
  constructor() {
    this.attributes = {};
    this.children = [];
    this.disabled = false;
    this.hidden = false;
    this.listeners = new Map();
    this.textContent = "";
    this.value = "";
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  append(...children) {
    this.children.push(...children);
  }

  focus() {
    this.focused = true;
  }

  replaceChildren(...children) {
    this.children = children;
  }

  setAttribute(name, value) {
    this.attributes[name] = value;
  }
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

async function surfaceHarness(initialRecords = []) {
  const html = await readFile(new URL("../src/ui/index.html", import.meta.url), "utf8");
  const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
  assert.ok(script, "the scaffold surface must contain its inline controller");

  const ids = [
    "items",
    "empty",
    "status",
    "new-item",
    "add-form",
    "add-item",
    "suggest",
    "refresh",
    "draft",
    "draft-text",
    "add-draft",
  ];
  const elements = new Map(ids.map((id) => [id, new Element()]));
  elements.get("draft").hidden = true;
  let initialize;
  let invoke = async () => undefined;
  let create = async () => ({});
  let list = async () => ({ records: initialRecords, next_after: null });
  let replace = async () => ({});
  let deleteRecord = async () => ({});
  const createCalls = [];
  const listCalls = [];
  const replaceCalls = [];
  const deleteCalls = [];
  const reportedErrors = [];
  const appHost = {
    data: {
      v1: {
        list(...args) {
          listCalls.push(args);
          return list(...args);
        },
        create(...args) {
          createCalls.push(args);
          return create(...args);
        },
        replace(...args) {
          replaceCalls.push(args);
          return replace(...args);
        },
        delete(...args) {
          deleteCalls.push(args);
          return deleteRecord(...args);
        },
      },
    },
    invoke(...args) {
      return invoke(...args);
    },
    onInit(callback) {
      initialize = callback;
    },
    ready() {},
    reportError(message) {
      reportedErrors.push(message);
    },
  };
  const document = {
    createElement() {
      return new Element();
    },
    getElementById(id) {
      return elements.get(id);
    },
  };

  vm.runInNewContext(script, { document, window: { appHost } });
  assert.equal(typeof initialize, "function");
  await initialize();

  return {
    createCalls,
    deleteCalls,
    elements,
    listCalls,
    replaceCalls,
    reportedErrors,
    setInvoke(implementation) {
      invoke = implementation;
    },
    setCreate(implementation) {
      create = implementation;
    },
    setDelete(implementation) {
      deleteRecord = implementation;
    },
    setList(implementation) {
      list = implementation;
    },
    setReplace(implementation) {
      replace = implementation;
    },
    listener(id, type) {
      const listener = elements.get(id).listeners.get(type);
      assert.equal(typeof listener, "function", `${id} must handle ${type}`);
      return listener;
    },
  };
}

function record(id, title, done = false, revision = 1) {
  return { id, revision, value: { title, done } };
}

function firstRenderedTitle(harness) {
  return harness.elements.get("items").children[0]?.children[0]?.children[1]?.textContent;
}

test("failed and pending form creates preserve the user's draft and prevent duplicates", async () => {
  const harness = await surfaceHarness();
  const input = harness.elements.get("new-item");
  const addButton = harness.elements.get("add-item");
  const addDraftButton = harness.elements.get("add-draft");
  const submit = harness.listener("add-form", "submit");
  const createResult = deferred();
  harness.setCreate(() => createResult.promise);
  input.value = "Keep this draft";

  const first = submit({ preventDefault() {} });
  const duplicate = submit({ preventDefault() {} });
  assert.equal(input.disabled, true);
  assert.equal(addButton.disabled, true);
  assert.equal(addDraftButton.disabled, true);
  assert.equal(harness.createCalls.length, 1);

  createResult.reject(new Error("storage unavailable"));
  await Promise.all([first, duplicate]);
  assert.equal(input.value, "Keep this draft");
  assert.equal(input.disabled, false);
  assert.equal(addButton.disabled, false);
  assert.equal(addDraftButton.disabled, false);

  harness.setCreate(async () => ({}));
  await submit({ preventDefault() {} });
  assert.equal(input.value, "");
  assert.equal(harness.createCalls.length, 2);
});

test("a model draft stays reviewable after failure and clears only after creation", async () => {
  const harness = await surfaceHarness();
  const draft = harness.elements.get("draft");
  const draftText = harness.elements.get("draft-text");
  const addButton = harness.elements.get("add-item");
  const addDraftButton = harness.elements.get("add-draft");
  const addDraft = harness.listener("add-draft", "click");
  draft.hidden = false;
  draftText.textContent = "Review this suggestion";

  const failedCreate = deferred();
  harness.setCreate(() => failedCreate.promise);
  const failed = addDraft();
  assert.equal(addButton.disabled, true);
  assert.equal(addDraftButton.disabled, true);
  failedCreate.reject(new Error("storage unavailable"));
  await failed;
  assert.equal(draft.hidden, false);
  assert.equal(draftText.textContent, "Review this suggestion");

  harness.setCreate(async () => ({}));
  await addDraft();
  assert.equal(draft.hidden, true);
  assert.equal(draftText.textContent, "");
});

test("only the newest load may replace records, status, or error state", async () => {
  const harness = await surfaceHarness();
  const refresh = harness.listener("refresh", "click");
  const older = deferred();
  const newer = deferred();
  const responses = [older.promise, newer.promise];
  harness.setList(() => responses.shift());

  const olderLoad = refresh();
  const newerLoad = refresh();
  newer.resolve({ records: [record("new", "Newest result")], next_after: null });
  await newerLoad;
  assert.equal(firstRenderedTitle(harness), "Newest result");
  assert.equal(harness.elements.get("status").textContent, "1 item");

  older.resolve({ records: [record("old", "Stale result")], next_after: null });
  await olderLoad;
  assert.equal(firstRenderedTitle(harness), "Newest result");
  assert.equal(harness.elements.get("status").textContent, "1 item");

  const staleFailure = deferred();
  const current = deferred();
  const nextResponses = [staleFailure.promise, current.promise];
  harness.setList(() => nextResponses.shift());
  const staleLoad = refresh();
  const currentLoad = refresh();
  current.resolve({ records: [record("current", "Current result")], next_after: null });
  await currentLoad;
  const errorsBeforeStaleFailure = harness.reportedErrors.length;
  staleFailure.reject(new Error("stale network failure"));
  await staleLoad;

  assert.equal(firstRenderedTitle(harness), "Current result");
  assert.equal(harness.elements.get("status").textContent, "1 item");
  assert.equal(harness.reportedErrors.length, errorsBeforeStaleFailure);
});

test("a row serializes rapid toggle and delete actions through its reload", async () => {
  const original = record("row-1", "One item");
  const harness = await surfaceHarness([original]);
  const item = harness.elements.get("items").children[0];
  const checkbox = item.children[0].children[0];
  const remove = item.children[1];
  const change = checkbox.listeners.get("change");
  const removeItem = remove.listeners.get("click");
  const replacement = deferred();
  const reload = deferred();
  harness.setReplace(() => replacement.promise);
  harness.setList(() => reload.promise);
  checkbox.checked = true;

  const update = change();
  const duplicateToggle = change();
  const racingDelete = removeItem();
  assert.equal(harness.replaceCalls.length, 1);
  assert.equal(harness.deleteCalls.length, 0);
  assert.equal(checkbox.disabled, true);
  assert.equal(remove.disabled, true);

  replacement.resolve({});
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(harness.listCalls.length, 2);
  const deleteDuringReload = removeItem();
  assert.equal(harness.deleteCalls.length, 0);
  assert.equal(checkbox.disabled, true);
  assert.equal(remove.disabled, true);

  reload.resolve({ records: [record("row-1", "One item", true, 2)], next_after: null });
  await Promise.all([update, duplicateToggle, racingDelete, deleteDuringReload]);
  const currentItem = harness.elements.get("items").children[0];
  const currentCheckbox = currentItem.children[0].children[0];
  const currentRemove = currentItem.children[1];
  assert.equal(currentCheckbox.checked, true);
  assert.equal(currentCheckbox.disabled, false);
  assert.equal(currentRemove.disabled, false);
  assert.equal(harness.replaceCalls.length, 1);
  assert.equal(harness.deleteCalls.length, 0);
});

for (const nextTitle of ["A newer suggestion", "The same suggestion"]) {
  test(`saving an older draft preserves a newer model response: ${nextTitle}`, async () => {
    const harness = await surfaceHarness();
    const draft = harness.elements.get("draft");
    const draftText = harness.elements.get("draft-text");
    const suggest = harness.listener("suggest", "click");
    const addDraft = harness.listener("add-draft", "click");
    const response = (content) => ({
      result: { kind: "completed", result: { message: { content } } },
    });
    harness.setInvoke(async () => response("The same suggestion"));
    await suggest();

    const model = deferred();
    harness.setInvoke(() => model.promise);
    const suggesting = suggest();
    const create = deferred();
    harness.setCreate(() => create.promise);
    const saving = addDraft();
    assert.equal(harness.createCalls[0][1].title, "The same suggestion");

    model.resolve(response(nextTitle));
    await suggesting;
    create.resolve({});
    await saving;

    assert.equal(draft.hidden, false);
    assert.equal(draftText.textContent, nextTitle);
    assert.equal(harness.createCalls.length, 1);
    assert.equal(harness.elements.get("add-draft").disabled, false);
  });
}

test("the hidden attribute overrides the draft's flex layout", async () => {
  const html = await readFile(new URL("../src/ui/index.html", import.meta.url), "utf8");
  assert.match(html, /\[hidden\]\s*\{\s*display:\s*none\s*!important\s*;/);
});
