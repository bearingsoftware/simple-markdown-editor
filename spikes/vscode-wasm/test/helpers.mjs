import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { JSDOM } from "jsdom";
import * as glue from "../pkg-hl/sme_vscode_core.js";

const here = dirname(fileURLToPath(import.meta.url));
glue.initSync({ module: readFileSync(resolve(here, "../pkg-hl/sme_vscode_core_bg.wasm")) });
export const core = glue;
export const enc = new TextEncoder();
export const dec = new TextDecoder();
export const dom = new JSDOM("<!doctype html><body></body>");
export const doc = dom.window.document;
