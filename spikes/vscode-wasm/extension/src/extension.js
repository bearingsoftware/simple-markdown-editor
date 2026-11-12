// MDEDIT-206 spike host: a CustomTextEditorProvider for *.md, priority "option". Buffer lives in the
// TextDocument here and in a wasm Session in the webview; edits cross as one byte-span replace.
const vscode = require("vscode");
const fs = require("node:fs");

const messages = []; // the test hook: every message a webview posted, in order
const panels = new Set();

// Strict from the first commit. The comment in docs/research/08-vscode-webview.md records what, if
// anything, a probe made us loosen.
function csp(webview) {
  const connect = process.env.SME_CSP_CONNECT ? ` connect-src ${webview.cspSource};` : ""; // probe only: see 08-vscode-webview.md
  return `default-src 'none'; script-src ${webview.cspSource} 'wasm-unsafe-eval'; style-src ${webview.cspSource};${connect}`;
}

function html(webview, media) {
  const u = (p) => webview.asWebviewUri(vscode.Uri.joinPath(media, p));
  return `<!doctype html>
<html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp(webview)}">
<link rel="stylesheet" href="${u("main.css")}"></head>
<body data-wasm="${u("core/sme_vscode_core_bg.wasm")}"><div id="root" contenteditable="true" spellcheck="false"></div>
<script type="module" src="${u("main.js")}"></script></body></html>`;
}

class Provider {
  constructor(context) {
    this.context = context;
  }

  async resolveCustomTextEditor(document, panel) {
    const media = vscode.Uri.joinPath(this.context.extensionUri, "media");
    panel.webview.options = { enableScripts: true, localResourceRoots: [media] };
    panel.webview.html = html(panel.webview, media);
    const entry = { document, panel, applying: false };
    panels.add(entry);

    const sendText = () => panel.webview.postMessage({ type: "text", text: document.getText(), version: document.version });

    const changeSub = vscode.workspace.onDidChangeTextDocument((e) => {
      if (e.document.uri.toString() !== document.uri.toString() || entry.applying) return;
      sendText();
    });
    panel.onDidDispose(() => {
      changeSub.dispose();
      panels.delete(entry);
    });

    panel.webview.onDidReceiveMessage(async (m) => {
      m.receivedAt = Date.now();
      const payload = m.payload;
      if (payload) {
        m.payloadLen = payload.length;
        delete m.payload;
      }
      messages.push(m);
      switch (m.type) {
        case "ready":
          sendText();
          break;
        case "wasm": {
          const bytes = fs.readFileSync(vscode.Uri.joinPath(media, "core/sme_vscode_core_bg.wasm").fsPath);
          panel.webview.postMessage({ type: "wasm", bytes: new Uint8Array(bytes) });
          break;
        }
        case "edit": {
          // One byte-span replace, already converted to a UTF-16 Range by the webview's core.
          const r = m.range;
          const range = new vscode.Range(r.sl, r.sc, r.el, r.ec);
          const edit = new vscode.WorkspaceEdit();
          edit.replace(document.uri, range, m.text);
          entry.applying = true;
          let ok;
          try {
            ok = await vscode.workspace.applyEdit(edit);
          } finally {
            entry.applying = false;
          }
          panel.webview.postMessage({ type: "ack", id: m.id, ok, version: document.version });
          break;
        }
        case "echo":
          panel.webview.postMessage({ type: "echo", id: m.id, payload });
          break;
        default:
          break;
      }
    });
  }
}

function activate(context) {
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider("mdeditSpike.editor", new Provider(context), {
      webviewOptions: { retainContextWhenHidden: false },
      supportsMultipleEditorsPerDocument: false,
    }),
  );
  return {
    messages,
    send: (msg) => {
      for (const e of panels) e.panel.webview.postMessage(msg);
    },
  };
}

module.exports = { activate };
