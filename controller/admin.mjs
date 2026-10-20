// server/admin.ts
import { realpathSync } from "node:fs";

// server/admin-network.ts
function ipv4(value) {
  if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(value)) return;
  const parts = value.split(".").map(Number);
  if (parts.some((part) => part > 255)) return;
  return parts.reduce((result, part) => result * 256 + part >>> 0, 0);
}
function privateAddress(host) {
  const value = ipv4(host);
  return value !== void 0 && (host.startsWith("10.") || host.startsWith("192.168.") || host.startsWith("127.") || value >>> 20 === 2753);
}

// server/admin.ts
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
async function runAdmin(args) {
  const { values, positionals } = parseArgs({ args, allowPositionals: true, options: {
    server: { type: "string", default: process.env.CORTICO_TERRARIUM_ADMIN_URL ?? (process.env.CORTICO_TERRARIUM_ADMIN_HOST ? `http://${process.env.CORTICO_TERRARIUM_ADMIN_HOST}:${process.env.CORTICO_TERRARIUM_ADMIN_PORT ?? 8788}` : "http://127.0.0.1:8787") },
    "token-file": { type: "string" },
    title: { type: "string" },
    text: { type: "string" },
    "text-file": { type: "string" },
    to: { type: "string", multiple: true },
    "expires-hours": { type: "string", default: "168" },
    after: { type: "string", default: "0" },
    at: { type: "string" },
    help: { type: "boolean" }
  } });
  if (values.help) return 'publish --text "\u516C\u544A\u5185\u5BB9" [--title "\u6807\u9898"] [--to box-id] [--expires-hours 168] | list | revoke <id> | feedback-list [--after <id>] | feedback-resolve <id> | controls | http-push on|off [--at <ISO timestamp with timezone>] | http-push cancel | minimum-version <version>|off [--at <ISO timestamp>] | minimum-version cancel\nOptional: --text-file <file>, --server <url>, --token-file <file>. Default token file: ./data/admin-token.';
  const [command, id] = positionals;
  if (!["publish", "list", "revoke", "feedback-list", "feedback-resolve", "controls", "http-push", "minimum-version"].includes(command) || (command === "minimum-version" ? positionals.length !== 2 || !/^(?:off|cancel|(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))$/.test(id ?? "") : command === "http-push" ? !["on", "off", "cancel"].includes(id) || positionals.length !== 2 : ["revoke", "feedback-resolve"].includes(command) ? !/^[1-9][0-9]*$/.test(id ?? "") || positionals.length !== 2 : positionals.length !== 1)) throw new Error("Unknown admin command or invalid ID; see --help");
  if (values.at !== void 0 && (!["http-push", "minimum-version"].includes(command) || id === "cancel")) throw new Error("--at requires a switch or minimum version value");
  const url = new URL(command === "controls" ? "/api/admin/controls" : command === "http-push" ? "/api/admin/controls/http-push" : command === "minimum-version" ? "/api/admin/controls/minimum-client-version" : command.startsWith("feedback-") ? `/api/admin/feedback${command === "feedback-resolve" ? `/${id}/resolve` : ""}` : `/api/admin/announcements${command === "revoke" ? `/${id}` : ""}`, values.server);
  if (command === "feedback-list") {
    const after = Number(values.after);
    if (!Number.isSafeInteger(after) || after < 0) throw new Error("Feedback cursor must be a nonnegative integer");
    url.searchParams.set("after", String(after));
  }
  if (url.protocol !== "https:" && !(url.protocol === "http:" && (["localhost", "[::1]"].includes(url.hostname) || privateAddress(url.hostname)))) throw new Error("Public admin requests require HTTPS");
  const token = process.env.CORTICO_TERRARIUM_ADMIN_TOKEN ?? (await readFile(values["token-file"] ?? process.env.CORTICO_TERRARIUM_ADMIN_TOKEN_FILE ?? "./data/admin-token", "utf8")).trim();
  if (!token) throw new Error("Admin token is empty");
  let body;
  if (command === "http-push") body = JSON.stringify(id === "cancel" ? { cancel: true } : { enabled: id === "on", ...values.at === void 0 ? {} : { at: values.at } });
  if (command === "minimum-version") body = JSON.stringify(id === "cancel" ? { cancel: true } : { version: id === "off" ? null : id, ...values.at === void 0 ? {} : { at: values.at } });
  if (command === "publish") {
    if (values.text !== void 0 && values["text-file"] !== void 0) throw new Error("Supply --text or --text-file");
    const hours = Number(values["expires-hours"]);
    if (!Number.isFinite(hours) || hours <= 0 || hours > 720) throw new Error("Expiry must be greater than zero and at most 720 hours");
    const text = values["text-file"] ? await readFile(values["text-file"], "utf8") : values.text;
    const targets = values.to?.flatMap((value) => value.split(",").map((id2) => id2.trim()).filter(Boolean));
    body = JSON.stringify({ text, title: values.title, targets: targets ? [...new Set(targets)] : "all", expiresAt: new Date(Date.now() + hours * 36e5).toISOString() });
  }
  const response = await fetch(url, { method: ["http-push", "minimum-version"].includes(command) ? "PUT" : command === "publish" || command === "feedback-resolve" ? "POST" : command === "revoke" ? "DELETE" : "GET", headers: { authorization: `Bearer ${token}`, ...body ? { "content-type": "application/json" } : {} }, body, redirect: "error", signal: AbortSignal.timeout(1e4) });
  const result = await response.json();
  if (!response.ok) throw new Error(`Admin HTTP ${response.status}: ${JSON.stringify(result)}`);
  return result;
}
if (process.argv[1] && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  void runAdmin(process.argv.slice(2)).then((result) => console.log(typeof result === "string" ? result : JSON.stringify(result, null, 2))).catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
export {
  runAdmin
};
