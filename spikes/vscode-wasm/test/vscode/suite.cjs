// Runs inside VS Code's extension host. Probes 6 (edit and undo through WorkspaceEdit), 7 (postMessage cost)
// and 3 (analysis timings in a real webview), plus the CSP finding. Plain asserts, no test framework.
const vscode = require("vscode");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const fnv = (u8) => {
  let h = 0x811c9dc5;
  for (let i = 0; i < u8.length; i++) h = Math.imul(h ^ u8[i], 0x01000193) >>> 0;
  return h;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function waitFor(pred, label, ms = 60000) {
  const t = Date.now();
  for (;;) {
    const v = pred();
    if (v) return v;
    if (Date.now() - t > ms) throw new Error(`timed out waiting for ${label}`);
    await sleep(50);
  }
}

// Mixed line endings (CRLF and LF), trailing whitespace, * and _ emphasis side by side, 2-, 3- and 4-byte characters.
const FIXTURE = Buffer.from(
  "# Títle 汉  \r\n\r\nplain *star* and _under_ side by side  \nline with trailing   \r\n\n- item \u{1F600} end\n",
  "utf8",
);

exports.run = async () => {
  const results = { vscode: vscode.version, tests: [], facts: {} };
  const test = async (name, fn) => {
    try {
      await fn();
      results.tests.push({ name, ok: true });
      console.log("PASS", name);
    } catch (e) {
      results.tests.push({ name, ok: false, error: String(e && e.stack ? e.stack : e) });
      console.log("FAIL", name, e);
    }
  };

  const file = path.join(process.env.SME_SCRATCH, "ws", "fixture.md");
  fs.writeFileSync(file, FIXTURE);
  const uri = vscode.Uri.file(file);
  const ext = vscode.extensions.getExtension("bearingsoftware.mdedit-spike");
  const api = await ext.activate();
  const stateFor = (kind, hash) => api.messages.find((m) => m.type === "state" && m.kind === kind && m.hash === hash);
  const lastState = () => [...api.messages].reverse().find((m) => m.type === "state");
  let doc;

  await test("the editor opens with strict CSP and the wasm boots", async () => {
    await vscode.commands.executeCommand("vscode.openWith", uri, "mdeditSpike.editor");
    const booted = await waitFor(() => api.messages.find((m) => m.type === "booted"), "booted");
    assert.ok(!api.messages.some((m) => m.type === "error"), JSON.stringify(api.messages.find((m) => m.type === "error")));
    results.facts.boot = booted.boot;
    doc = vscode.workspace.textDocuments.find((d) => d.uri.fsPath === uri.fsPath);
    assert.ok(doc);
    await waitFor(() => api.messages.find((m) => m.type === "state"), "first render");
  });

  await test("fact: what VS Code does to mixed line endings on open", async () => {
    const text = doc.getText();
    const asBytes = Buffer.from(text, "utf8");
    results.facts.diskBytes = FIXTURE.length;
    results.facts.docBytes = asBytes.length;
    results.facts.docEol = doc.eol === vscode.EndOfLine.CRLF ? "CRLF" : "LF";
    results.facts.docKeepsMixedEndings = asBytes.equals(FIXTURE);
    results.facts.docCountsCR = (text.match(/\r/g) ?? []).length;
    results.facts.diskCountsCR = (FIXTURE.toString().match(/\r/g) ?? []).length;
  });

  const original = () => Buffer.from(doc.getText(), "utf8");
  let before;

  await test("webview and document agree before any edit", async () => {
    before = original();
    await waitFor(() => stateFor("text", fnv(before)), "webview rendered the document's bytes");
  });

  const inserts = [
    ["after the star emphasis text", () => original().indexOf("star") + 2, "Z"],
    ["after a 4-byte emoji, on a later line", () => original().indexOf("\u{1F600}") + 4, "Y"],
    ["at the start of a line that has CRLF above and trailing spaces", () => original().indexOf("line with"), "W"],
  ];
  let id = 1000;
  for (const [name, at, data] of inserts) {
    await test(`edit: ${name} is exactly one inserted byte span`, async () => {
      const b0 = original();
      const off = at();
      const expected = Buffer.concat([b0.subarray(0, off), Buffer.from(data), b0.subarray(off)]);
      const n = api.messages.length;
      api.send({ type: "sim-insert", byte: off, data });
      const ack = await waitFor(() => api.messages.slice(n).find((m) => m.type === "acked"), "ack");
      assert.equal(ack.ok, true);
      const after = original();
      assert.ok(after.equals(expected), `document bytes differ from original by more than the insertion\n${JSON.stringify(after.toString())}\n${JSON.stringify(expected.toString())}`);
      assert.equal(after.length, b0.length + Buffer.byteLength(data));
      // The webview's own buffer, edited locally through one replace(), must match the document byte for byte.
      const st = await waitFor(() => api.messages.slice(n).find((m) => m.type === "state" && m.kind === "local-edit"), "local state");
      assert.equal(st.hash, fnv(after), "webview Session bytes equal the document bytes");
      id++;
    });
  }

  let afterEdits;
  await test("undo in the source view reverts the webview-made edits and the webview follows", async () => {
    afterEdits = original();
    const editor = await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside, preview: false });
    assert.equal(editor.document, doc);
    let undos = 0;
    while (!original().equals(before) && undos < 6) {
      await vscode.commands.executeCommand("undo");
      undos++;
      await sleep(100);
    }
    results.facts.undosNeededForThreeEdits = undos;
    assert.ok(original().equals(before), "document is back to its pre-edit bytes");
    await waitFor(() => stateFor("text", fnv(before)) && api.messages.filter((m) => m.type === "state" && m.kind === "text" && m.hash === fnv(before)).length >= 2, "webview re-rendered the undone bytes");
  });

  await test("a source-view edit reaches the webview", async () => {
    const editor = vscode.window.activeTextEditor;
    assert.equal(editor.document, doc);
    await editor.edit((b) => b.insert(new vscode.Position(0, 0), "Q"));
    const want = fnv(original());
    await waitFor(() => api.messages.filter((m) => m.type === "state" && m.hash === want).length >= 1, "webview shows the source edit");
  });

  await test("fact: saved bytes versus the original file (mixed endings)", async () => {
    await doc.save();
    const disk = fs.readFileSync(file);
    const expected = Buffer.concat([Buffer.from("Q"), FIXTURE]);
    results.facts.savedBytes = disk.length;
    results.facts.savedEqualsOriginalPlusQ = disk.equals(expected);
    results.facts.savedEqualsDocumentText = disk.equals(Buffer.from(doc.getText(), "utf8"));
  });

  await test("bench: wasm timings and postMessage cost inside the real webview", async () => {
    const n = api.messages.length;
    api.send({ type: "bench" });
    const b = await waitFor(() => api.messages.slice(n).find((m) => m.type === "bench"), "bench", 240000);
    results.bench = b.result;
    const echoes = api.messages.slice(n).filter((m) => m.type === "echo" && m.sentAt);
    const oneWay = echoes.slice(2).map((m) => m.receivedAt - m.sentAt).sort((x, y) => x - y);
    results.facts.postMessage_1MB_webview_to_host_ms = { n: oneWay.length, median: oneWay[oneWay.length >> 1], max: oneWay[oneWay.length - 1], note: "wall clock across two processes, 1 ms resolution" };
  });

  fs.writeFileSync(process.env.SME_RESULTS, JSON.stringify(results, null, 1));
  const failed = results.tests.filter((t) => !t.ok);
  console.log(`vscode e2e: ${results.tests.length - failed.length}/${results.tests.length} passed`);
  if (failed.length) throw new Error(`${failed.length} failed`);
};
