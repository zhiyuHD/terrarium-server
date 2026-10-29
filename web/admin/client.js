// server/console/icons.ts
function icon(name) {
  const namespace = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(namespace, "svg");
  svg.setAttribute("class", "icon");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.8");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const use = document.createElementNS(namespace, "use");
  use.setAttribute("href", `${new URL("./assets/icons.svg", import.meta.url).href}#${name}`);
  svg.append(use);
  return svg;
}
function mountIcons() {
  for (const placeholder of Array.from(document.querySelectorAll("[data-icon]"))) {
    placeholder.replaceWith(icon(placeholder.dataset.icon));
  }
}

// server/console/client.ts
mountIcons();
var el = (id) => document.getElementById(id);
var input = (id) => el(id);
function node(tag, text = "", className = "") {
  const element = document.createElement(tag);
  element.textContent = text;
  element.className = className;
  return element;
}
function button(text, action, className = "", symbol) {
  const element = node("button", "", className);
  if (symbol) element.append(icon(symbol));
  if (text) element.append(node("span", text));
  element.type = "button";
  element.onclick = action;
  return element;
}
function iconLabel(element, symbol, text) {
  element.replaceChildren(icon(symbol), node("span", text));
  element.classList.add("with-icon");
}
var dateFormatter = new Intl.DateTimeFormat("zh-CN", { timeZone: "Asia/Shanghai", dateStyle: "short", timeStyle: "medium", hour12: false });
var date = (value) => value ? dateFormatter.format(new Date(value)) : "\u5C1A\u672A\u4E0A\u62A5";
var token = sessionStorage.getItem("terrarium-admin-token") ?? "";
var epoch = 0;
var refreshing = false;
var snapshot;
var activeTab = "boxes", boxPage = 1, feedbackPage = 1, statistics;
var feedbackMeta, searchTimer;
var boxIndex = new Map();
function remember(boxes) { for (const box of boxes) boxIndex.set(box.id, { ...boxIndex.get(box.id), ...box }); }
function refreshKey() { return JSON.stringify([activeTab,boxPage,filter,input("search").value.trim(),statisticsScope,feedbackPage,pendingOnly]); }
function boxQuery(view = "page") { return "boxes?" + new URLSearchParams({view,page:String(boxPage),pageSize:"50",q:input("search").value.trim(),version:filter}); }
function requestPage(page) { boxPage = page; void refresh(); }
function pager(host, page, pages, change) {
  host.replaceChildren(button("上一页",()=>change(page-1)),node("span",`第 ${page} / ${pages} 页`),button("下一页",()=>change(page+1)));
  host.firstChild.disabled = page <= 1; host.lastChild.disabled = page >= pages;
}
function renderActive() {
  if (!snapshot) return;
  if (activeTab === "boxes") renderBoxes();
  else if (activeTab === "statistics" && statistics) renderStatistics();
  else if (activeTab === "feedback") renderFeedback();
  else if (activeTab === "announcements") { audience(); renderNotices(); }
  else if (activeTab === "controls" && controls) renderControls();
}

var feedback = [];
var notices = [];
var controls;
var filter = "*";
var statisticsScope = "reported";
var pendingOnly = false;
var allTargets = false;
var scheduleEnabled = false;
var mutating = false;
var selected = /* @__PURE__ */ new Set();
var status = (message) => {
  el("status").textContent = message;
};
function logout() {
  ++epoch;
  token = "";
  sessionStorage.removeItem("terrarium-admin-token");
  selected.clear();
  boxIndex.clear(); statistics = void 0; feedbackMeta = void 0; clearTimeout(searchTimer);
  snapshot = void 0;
  feedback = [];
  notices = [];
  controls = void 0;
  el("modal").hidden = true;
  modalWork = void 0;
  el("app").hidden = true;
  el("login").hidden = false;
  input("token").value = "";
  for (const id of ["box-rows", "feedback-list", "notice-list", "stats", "statistics-charts"]) el(id).replaceChildren();
}
async function api(path, method = "GET", body) {
  const currentEpoch = epoch;
  const response = await fetch(`/api/admin/${path}`, { method, headers: { authorization: `Bearer ${token}`, ...body === void 0 ? {} : { "content-type": "application/json" } }, ...body === void 0 ? {} : { body: JSON.stringify(body) }, signal: AbortSignal.timeout(15e3), redirect: "error" });
  if (currentEpoch !== epoch) throw new Error("\u4F1A\u8BDD\u5DF2\u53D8\u5316");
  if (response.status === 401) {
    logout();
    throw new Error("\u51ED\u636E\u65E0\u6548\uFF0C\u8BF7\u91CD\u65B0\u767B\u5F55");
  }
  const data = await response.json();
  if (!response.ok) throw new Error(data.error ?? `HTTP ${response.status}`);
  return data;
}
async function refresh() {
  if (!token || refreshing) return;
  refreshing = true;
  const currentEpoch = epoch, key = refreshKey(), requestedTab = activeTab;
  try {
    const extra = requestedTab === "feedback" ? api("feedback?"+new URLSearchParams({view:"page",page:String(feedbackPage),pageSize:"50",pendingOnly:String(pendingOnly)}))
      : requestedTab === "statistics" ? api("boxes?view=statistics&scope="+encodeURIComponent(statisticsScope))
      : requestedTab === "controls" ? api("controls")
      : requestedTab === "announcements" ? api("announcements") : Promise.resolve(null);
    const [next, detail] = await Promise.all([api(boxQuery()), extra]);
    if (epoch !== currentEpoch || key !== refreshKey()) return;
    snapshot = next; boxPage = next.page; remember(next.boxes); for (const box of next.boxes) if (!box.registered) selected.delete(box.id); serverOffset = Date.parse(next.serverAt) - Date.now();
    if (requestedTab === "feedback") {
      feedback = detail.feedback; feedbackMeta = detail; feedbackPage = detail.page;
      const ids = [...new Set(feedback.map(item=>item.boxId).filter(Boolean))];
      if (ids.length) {
        const names = await api("boxes?view=lookup&ids="+encodeURIComponent(JSON.stringify(ids)));
        if (epoch !== currentEpoch || key !== refreshKey()) return;
        remember(names.boxes);
      }
    } else if (requestedTab === "statistics") statistics = detail.statistics;
    else if (requestedTab === "controls") controls = detail;
    else if (requestedTab === "announcements") notices = detail.announcements;
    el("login").hidden = true; el("app").hidden = false; status(""); render();
    el("sync-at").textContent = `更新于 ${date(next.serverAt)}`;
    el("server-info").textContent = `运行 ${Math.floor(next.uptimeSec / 3600)} 小时 · 主进程内存 ${(next.memoryBytes / 1048576).toFixed(1)} MiB · 时间均为北京时间`;
  } catch (error) {
    const message = error.message;
    if (epoch === currentEpoch) status(message);
    if (!token || el("app").hidden) el("login-error").textContent = message;
  } finally {
    refreshing = false;
    if (token && epoch === currentEpoch && key !== refreshKey()) void refresh();
  }
}
function tab(name) {
  activeTab = name; renderActive(); void refresh();
  for (const section of Array.from(document.querySelectorAll(".panel"))) section.hidden = section.id !== name;
  for (const item of Array.from(document.querySelectorAll("[data-tab]"))) {
    item.classList.toggle("active", item.dataset.tab === name);
    if (item.dataset.tab === name) el("page-title").textContent = item.textContent.trim();
  }
}
function version(box) {
  return box.clientVersion ?? (box.reportedAt ? "\u22640.7.3\uFF08\u63A8\u5B9A\uFF09" : "\u672A\u4E0A\u62A5");
}
function visibleBoxes() { return snapshot?.boxes ?? []; }
function selectionInfo() {
  if (!snapshot) return;
  el("selection-info").textContent = `符合筛选 ${snapshot.filtered} / ${snapshot.total} 个 · 本页 ${snapshot.boxes.length} 个 · 已选择 ${selected.size} 个（跨页保留，上限 100）`;
}
function audience() {
  input("scope-all").setAttribute("aria-pressed", String(allTargets));
  input("scope-selected").setAttribute("aria-pressed", String(!allTargets));
  el("audience").textContent = allTargets ? "\u5E7F\u64AD \xB7 \u516C\u7F51\u53EF\u89C1" : `\u5B9A\u5411 \xB7 ${selected.size} \u4E2A\u76D2\u5B50`;
  el("audience-names").textContent = allTargets ? "\u6240\u6709\u73B0\u6709\u53CA\u6709\u6548\u671F\u5185\u65B0\u6CE8\u518C\u7684\u5BA2\u6237\u7AEF\uFF0C\u516C\u7F51\u9875\u9762\u4E5F\u4F1A\u5C55\u793A\u3002" : [...selected].map((id) => `${boxIndex.get(id)?.name ?? id} (${id})`).join("\u3001") || "\u8BF7\u5148\u5728\u76D2\u5B50\u6982\u89C8\u4E2D\u9009\u62E9\u63A5\u6536\u8005\u3002";
}
function renderBoxes() {
  const rows = el("box-rows");
  const fragment = document.createDocumentFragment();
  const visible = visibleBoxes();
  for (const box of visible) {
    const row = node("tr");
    row.classList.toggle("selected", selected.has(box.id));
    const nameCell = node("td"), check = button("", () => {
      if (selected.has(box.id)) selected.delete(box.id);
      else { if (selected.size >= 100) { status("定向公告最多选择 100 个盒子；请缩小范围或使用广播。"); return; } selected.add(box.id); }
      row.classList.toggle("selected", selected.has(box.id));
      check.setAttribute("aria-checked", String(selected.has(box.id))); selectionInfo(); audience();
    }, "check", "check");
    check.setAttribute("role", "checkbox");
    check.setAttribute("aria-checked", String(selected.has(box.id)));
    check.setAttribute("aria-label", `\u9009\u62E9 ${box.name}`);
    check.disabled = !box.registered;
    const name = node("div");
    name.append(node("strong", box.name), node("span", `${box.kind === "land" ? "\u5730" : box.kind === "water" ? "\u6C34" : "\u5F85\u8BA4\u9886"} \xB7 ${box.id}`, "small muted"));
    nameCell.append(check, name);
    row.append(nameCell, node("td", version(box)), node("td", box.coopanionVersion ?? "\u672A\u4E0A\u62A5"), node("td", box.clientSystem ? `${box.clientSystem.name} · ${box.clientSystem.version}` : "\u672A\u4E0A\u62A5"));
    const connection = node("td"), live = node("span", "", box.wsConnections ? "pill" : "muted");
    iconLabel(live, box.wsConnections ? "wifi" : "wifi-off", box.wsConnections ? `WS \u5728\u7EBF (${box.wsConnections})` : "WS \u672A\u8FDE\u63A5");
    connection.append(live, node("span", `\u6700\u8FD1\u63D0\u4EA4\uFF1A${box.reportTransport === "websocket" ? "WS" : box.reportTransport === "http" ? "HTTP" : "\u672A\u77E5"}`, "small muted"));
    row.append(connection, node("td", date(box.reportedAt)), node("td", box.score === null ? "\u672A\u4E0A\u62A5" : String(box.score)));
    fragment.append(row);
  }
  if (!visible.length) {
    const row = node("tr"), cell = node("td", "\u6CA1\u6709\u7B26\u5408\u6761\u4EF6\u7684\u76D2\u5B50", "empty");
    cell.colSpan = 7;
    row.append(cell);
    fragment.append(row);
  }
  rows.replaceChildren(fragment);
  for (const id of ["box-pagination-top","box-pagination"]) pager(el(id),snapshot.page,snapshot.pages,requestPage);
  selectionInfo();
  audience();
}
function render() {
  if (!snapshot) return;
  const stats = el("stats");
  stats.replaceChildren();
  for (const [label, value] of [["\u5DF2\u6CE8\u518C\u76D2\u5B50", snapshot.counts.registered], ["WS \u8FDE\u63A5\u4E2D\u7684\u76D2\u5B50", snapshot.counts.online], ["\u5F85\u5904\u7406\u53CD\u9988", snapshot.counts.pendingFeedback], ["\u6709\u6548\u516C\u544A", snapshot.counts.activeNotices]]) {
    const card = node("div", "", "stat");
    card.append(node("strong", String(value)), node("span", String(label)));
    stats.append(card);
  }
  const versions = el("versions");
  versions.replaceChildren();
  for (const value of ["*", ...snapshot.versions]) {
    const control = button(value === "*" ? "\u5168\u90E8\u7248\u672C" : value, () => {
      filter = value; boxPage = 1; void refresh();
    });
    control.setAttribute("aria-pressed", String(filter === value));
    versions.append(control);
  }
  renderActive();
}
function statisticsSlices(rows) {
  if (rows.length <= 8) return rows;
  return [...rows.slice(0, 8), { label: "其他", count: rows.slice(8).reduce((total, row) => total + row.count, 0) }];
}
function statisticsChart({ title, rows }, total) {
  const card = node("article", "", "card chart-card");
  card.append(node("h3", title));
  if (!total) {
    card.append(node("p", "当前范围内没有已上报的盒子", "empty"));
    return card;
  }
  const colors = ["#68bd99", "#79a9ea", "#d9aa66", "#ba93da", "#dd879b", "#5fbcc8", "#bdc763", "#9f9cdb", "#899b9c"];
  const namespace = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(namespace, "svg");
  svg.setAttribute("viewBox", "0 0 180 180");
  svg.setAttribute("class", "distribution-chart");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", `${title}分布，共 ${total} 个盒子，数量和比例见下方明细`);
  const layout = node("div", "", "chart-content");
  const legend = node("ul", "", "chart-legend");
  let offset = 0;
  for (const [index, row] of statisticsSlices(rows).entries()) {
    const share = row.count / total;
    const circle = document.createElementNS(namespace, "circle");
    for (const [key, value] of Object.entries({ cx: 90, cy: 90, r: 64, fill: "none", stroke: colors[index], "stroke-width": 26, "stroke-dasharray": `${share * 402.12386} 402.12386`, "stroke-dashoffset": -offset * 402.12386, transform: "rotate(-90 90 90)" })) circle.setAttribute(key, String(value));
    const tooltip = document.createElementNS(namespace, "title");
    tooltip.textContent = `${row.label}：${row.count} 个 · ${(share * 100).toFixed(1)}%`;
    circle.append(tooltip);
    svg.append(circle);
    offset += share;
    const item = node("li");
    const swatch = node("span", "", "chart-swatch");
    swatch.style.backgroundColor = colors[index];
    swatch.setAttribute("aria-hidden", "true");
    item.append(swatch, node("span", row.label, "chart-label"), node("strong", `${row.count} · ${(share * 100).toFixed(1)}%`, "chart-value"));
    legend.append(item);
  }
  const center = document.createElementNS(namespace, "text");
  center.setAttribute("x", "90");
  center.setAttribute("y", "98");
  center.setAttribute("text-anchor", "middle");
  center.setAttribute("class", "chart-total");
  center.textContent = String(total);
  svg.append(center);
  layout.append(svg, legend);
  card.append(layout);
  if (rows.length > 8) {
    const details = node("ul", "", "chart-details");
    details.hidden = true;
    for (const row of rows) {
      const item = node("li");
      item.append(node("span", row.label, "chart-label"), node("strong", `${row.count} · ${(row.count / total * 100).toFixed(1)}%`));
      details.append(item);
    }
    const toggle = button(`全部 ${rows.length} 项明细`, () => {
      details.hidden = !details.hidden;
      toggle.setAttribute("aria-expanded", String(!details.hidden));
    });
    toggle.setAttribute("aria-expanded", "false");
    card.append(toggle, details);
  }
  return card;
}
function renderStatistics() {
  const data = statistics;
  el("statistics-summary").textContent = `当前统计 ${data.total} 个 · 全部已上报 ${data.reported} 个 · 近 24 小时 ${data.recent} 个 · 从未上报 ${data.unreported} 个（不计入图表）`;
  el("statistics-charts").replaceChildren(...data.charts.map(chart => statisticsChart(chart, data.total)));
}
for (const control of document.querySelectorAll("#statistics-scope [data-scope]")) {
  control.onclick = () => {
    statisticsScope = control.dataset.scope;
    for (const item of document.querySelectorAll("#statistics-scope [data-scope]")) item.setAttribute("aria-pressed", String(item === control));
    statistics = void 0; el("statistics-summary").textContent = "正在读取统计…"; void refresh();
  };
}
function renderFeedback() {
  const list = el("feedback-list");
  list.replaceChildren();
  for (const item of feedback) {
    const card = node("article", "", "card");
    card.append(node("span", item.resolvedAt ? "\u5DF2\u89E3\u51B3" : "\u5F85\u5904\u7406", "pill"), node("div", `#${item.id} \xB7 ${item.source} \xB7 ${date(item.createdAt)}`, "meta"), node("p", item.text, "wrap"));
    const box = boxIndex.get(item.boxId);
    card.append(node("p", `\u76D2\u5B50\uFF1A${box?.name ?? item.boxId ?? "\u533F\u540D\u53CD\u9988"}${item.contact ? ` \xB7 \u8054\u7CFB\u65B9\u5F0F\uFF1A${item.contact}` : ""}`, "muted small wrap"));
    const actions = node("div", "", "toolbar");
    if (item.boxId && box?.registered) actions.append(button("\u5B9A\u5411\u56DE\u590D", () => {
      selected.clear();
      selected.add(item.boxId);
      allTargets = false;
      input("notice-title").value = `\u53CD\u9988 #${item.id} \u56DE\u590D`;
      input("notice-text").value = "";
      audience();
      tab("announcements");
      input("notice-text").focus();
    }, "", "reply"));
    if (!item.resolvedAt) actions.append(button("\u6807\u8BB0\u5DF2\u89E3\u51B3", () => confirmAction(`\u89E3\u51B3\u53CD\u9988 #${item.id}`, item.text, async () => {
      await api(`feedback/${item.id}/resolve`, "POST");
    }), "primary", "circle-check"));
    else card.append(node("p", `\u89E3\u51B3\u4E8E ${date(item.resolvedAt)}`, "muted small"));
    card.append(actions);
    list.append(card);
  }
  if (!list.children.length) list.append(node("div", "暂无反馈", "card empty"));
  if (feedbackMeta) for (const id of ["feedback-pagination-top","feedback-pagination"]) pager(el(id),feedbackMeta.page,feedbackMeta.pages,page=>{feedbackPage=page;void refresh()});
}
function renderNotices() {
  el("notice-count").textContent = `${notices.length} 条`;
  const list = el("notice-list");
  list.replaceChildren();
  for (const notice of [...notices].reverse()) {
    const card = node("article", "", "card");
    card.append(node("h3", `投递 ID #${notice.id} ${notice.title || "\u65E0\u6807\u9898"}`), node("p", notice.text, "wrap"), node("p", notice.targets === "all" ? "\u5E7F\u64AD \xB7 \u516C\u7F51\u53EF\u89C1" : `\u5B9A\u5411\uFF1A${notice.targets.map((id) => boxIndex.get(id)?.name ?? id).join("\u3001")}`, "small muted wrap"), node("p", `${date(notice.createdAt)} \u81F3 ${date(notice.expiresAt)}${Date.parse(notice.expiresAt) <= Date.now() ? " \xB7 \u5DF2\u8FC7\u671F" : ""}`, "small muted"));
    card.append(button("\u64A4\u56DE", () => confirmAction(`\u64A4\u56DE\u516C\u544A #${notice.id}`, `${notice.title}
\u5BA2\u6237\u7AEF\u5C06\u4E0D\u518D\u62C9\u53D6\u6B64\u516C\u544A\uFF0C\u5DF2\u8FDB\u5165\u6A21\u578B\u4E0A\u4E0B\u6587\u7684\u4E8B\u4EF6\u4ECD\u7136\u4FDD\u7559\u3002`, async () => {
      await api(`announcements/${notice.id}`, "DELETE");
    }), "danger", "undo-2"));
    list.append(card);
  }
  if (!list.children.length) list.append(node("div", "\u6682\u65E0\u516C\u544A", "card empty"));
}
var serverOffset = 0;
function renderControls() {
  if (!controls || !snapshot) return;
  serverOffset = Date.parse(snapshot.serverAt) - Date.now();
  iconLabel(el("http-status"), controls.httpPush.enabled ? "power" : "power-off", controls.httpPush.enabled ? "\u5DF2\u5F00\u542F" : "\u5DF2\u5173\u95ED");
  el("http-cancel").disabled = !controls.httpPush.scheduledChange;
  const floor = controls.minimumClientVersion;
  el("minimum-status").textContent = floor?.version ? `\u81F3\u5C11 ${floor.version}` : "\u672A\u9650\u5236";
  el("minimum-schedule").textContent = floor?.scheduledChange ? `${date(floor.scheduledChange.at)} \xB7 ${floor.scheduledChange.version ? `\u81F3\u5C11 ${floor.scheduledChange.version}` : "\u53D6\u6D88\u7248\u672C\u9650\u5236"}` : "\u6682\u65E0\u5B9A\u65F6\u53D8\u66F4";
  el("minimum-cancel").disabled = !floor?.scheduledChange;
  el("logo-status").textContent = logoLabel(controls.logo?.version);
  el("logo-schedule").textContent = controls.logo?.scheduledChange ? `${date(controls.logo.scheduledChange.at)} · ${controls.logo.scheduledChange.version}` : "暂无定时变更";
  el("logo-cancel").disabled = !controls.logo?.scheduledChange;
  const forum = controls.forum;
  el("forum-status").textContent = forum?.enabled ? "已开放" : "未开放论坛";
  el("forum-schedule").textContent = forum?.scheduledChange ? `${date(forum.scheduledChange.at)} ${forum.scheduledChange.enabled ? "开放" : "关闭"}` : "暂无定时变更";
  el("forum-cancel").disabled = !forum?.scheduledChange;
  renderCountdown();
}
function renderCountdown() {
  const schedule = controls?.httpPush.scheduledChange;
  if (!schedule) {
    el("schedule-status").textContent = "\u6682\u65E0\u5B9A\u65F6\u4EFB\u52A1";
    return;
  }
  const seconds = Math.ceil((Date.parse(schedule.at) - Date.now() - serverOffset) / 1e3);
  el("schedule-status").textContent = `${date(schedule.at)} ${schedule.enabled ? "\u5F00\u542F" : "\u5173\u95ED"} \xB7 ${seconds > 0 ? `\u5269\u4F59 ${Math.floor(seconds / 86400)} \u5929 ${Math.floor(seconds % 86400 / 3600)} \u5C0F\u65F6 ${Math.floor(seconds % 3600 / 60)} \u5206 ${seconds % 60} \u79D2` : "\u5DF2\u5230\u8BA1\u5212\u65F6\u95F4\uFF0C\u7B49\u5F85\u5237\u65B0\u786E\u8BA4"}`;
}
var modalWork;
var previousFocus = null;
function closeModal() {
  if (mutating) return;
  el("modal").hidden = true;
  modalWork = void 0;
  previousFocus?.focus();
}
function confirmAction(title, text, work) {
  if (mutating) return;
  previousFocus = document.activeElement;
  modalWork = work;
  el("modal-title").textContent = title;
  el("modal-body").textContent = text;
  el("modal").hidden = false;
  el("modal-cancel").focus();
}
el("modal-confirm").onclick = async () => {
  if (!modalWork || mutating) return;
  mutating = true;
  el("modal-confirm").disabled = true;
  el("modal-cancel").disabled = true;
  try {
    await modalWork();
    mutating = false;
    closeModal();
    await refresh();
  } catch (error) {
    el("modal-body").append(node("p", `\u64CD\u4F5C\u5931\u8D25\uFF1A${error.message}`));
  } finally {
    mutating = false;
    el("modal-confirm").disabled = false;
    el("modal-cancel").disabled = false;
  }
};
el("modal-cancel").onclick = closeModal;
el("modal").addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeModal();
  if (event.key === "Tab") {
    const first = el("modal-cancel"), last = el("modal-confirm");
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
});
el("login-form").onsubmit = async (event) => {
  event.preventDefault();
  if (refreshing) return;
  ++epoch;
  token = input("token").value.trim();
  sessionStorage.setItem("terrarium-admin-token", token);
  el("login-error").textContent = "";
  await refresh();
  input("token").value = "";
};
el("logout").onclick = logout;
el("refresh").onclick = () => {
  void refresh();
};
for (const item of Array.from(document.querySelectorAll("[data-tab]"))) item.onclick = () => tab(item.dataset.tab);
input("search").oninput = () => {clearTimeout(searchTimer);boxPage=1;searchTimer=setTimeout(()=>void refresh(),250)};
el("select-visible").onclick = () => {
  const ids = visibleBoxes().filter(box=>box.registered).map(box=>box.id);
  if (new Set([...selected,...ids]).size > 100) { status("定向公告最多选择 100 个盒子。"); return; }
  for (const id of ids) selected.add(id);
  renderBoxes();
};
el("select-filtered").onclick = async () => {
  const key = refreshKey();
  try {
    const result = await api(boxQuery("ids"));
    if (key !== refreshKey()) return;
    if (result.tooMany || new Set([...selected,...result.boxes.map(box=>box.id)]).size>100) { status("结果超过定向公告的 100 个上限，请缩小筛选范围或选择广播。"); return; }
    remember(result.boxes);for(const box of result.boxes)selected.add(box.id);renderBoxes();
  } catch(error) {status(error.message);}
};
el("clear-selected").onclick = () => {
  selected.clear();
  renderBoxes();
};
el("compose-selected").onclick = () => {
  allTargets = false;
  audience();
  tab("announcements");
};
el("scope-selected").onclick = () => {
  allTargets = false;
  audience();
};
el("scope-all").onclick = () => {
  allTargets = true;
  audience();
};
el("feedback-filter").onclick = () => {
  pendingOnly = !pendingOnly; feedbackPage = 1;
  el("feedback-filter").setAttribute("aria-pressed", String(pendingOnly));
  void refresh();
};
for (const item of Array.from(document.querySelectorAll("[data-enable]"))) item.onclick = () => {
  scheduleEnabled = item.dataset.enable === "true";
  for (const other of Array.from(document.querySelectorAll("[data-enable]"))) other.setAttribute("aria-pressed", String(other === item));
};
for (const enabled of [true, false]) el(enabled ? "http-on" : "http-off").onclick = () => confirmAction(`\u7ACB\u5373${enabled ? "\u5F00\u542F" : "\u5173\u95ED"} HTTP Push`, `${enabled ? "\u65E7\u7248\u5BA2\u6237\u7AEF\u53EF\u4EE5\u901A\u8FC7 HTTP \u63D0\u4EA4\u3002" : "\u4FDD\u7559 HTTP \u63A5\u53E3\uFF0C\u5FFD\u7565\u63D0\u4EA4\u5E76\u8FD4\u56DE\u672A\u63D0\u4EA4\u6210\u529F\u63D0\u793A\uFF0C\u540C\u65F6\u53D1\u9001\u5347\u7EA7\u516C\u544A\u3002"}
\u8FD9\u4F1A\u6E05\u9664\u73B0\u6709\u5B9A\u65F6\u4EFB\u52A1\u3002`, async () => {
  await api("controls/http-push", "PUT", { enabled });
});
el("http-cancel").onclick = () => confirmAction("\u53D6\u6D88\u5B9A\u65F6\u4EFB\u52A1", "\u4FDD\u7559\u5F53\u524D\u751F\u6548\u7684 HTTP Push \u5F00\u5173\u3002", async () => {
  await api("controls/http-push", "PUT", { cancel: true });
});
el("schedule-form").onsubmit = (event) => {
  event.preventDefault();
  const value = input("schedule-at").value, at = /* @__PURE__ */ new Date(`${value.replace(" ", "T")}:00+08:00`);
  const canonical = Number.isFinite(at.getTime()) ? new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Shanghai", year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false }).format(at) : "";
  if (canonical !== value || at.getTime() <= Date.now() + serverOffset) {
    status("\u8BF7\u8F93\u5165\u6709\u6548\u7684\u672A\u6765\u5317\u4EAC\u65F6\u95F4\uFF0C\u683C\u5F0F\u4E3A YYYY-MM-DD HH:mm");
    return;
  }
  const body = { enabled: scheduleEnabled, at: at.toISOString() };
  confirmAction("\u786E\u8BA4\u5B9A\u65F6\u5207\u6362", `${value}\uFF08\u5317\u4EAC\u65F6\u95F4\uFF09${body.enabled ? "\u5F00\u542F" : "\u5173\u95ED"} HTTP Push\u3002
\u5C06\u66FF\u6362\u73B0\u6709\u5B9A\u65F6\u4EFB\u52A1\u3002`, async () => {
    await api("controls/http-push", "PUT", body);
  });
};
el("notice-form").onsubmit = (event) => {
  event.preventDefault();
  const hours = Number(input("expiry").value), text = input("notice-text").value.trim();
  if (!text || !Number.isFinite(hours) || hours <= 0 || hours > 720 || !allTargets && (selected.size === 0 || selected.size > 100)) {
    status("\u8BF7\u8F93\u5165\u516C\u544A\u5185\u5BB9\u30010\uFF5E720 \u5C0F\u65F6\u6709\u6548\u671F\uFF0C\u5E76\u9009\u62E9 1\uFF5E100 \u4E2A\u76D2\u5B50\u6216\u5E7F\u64AD\u3002");
    return;
  }
  const body = { title: input("notice-title").value.trim(), text, targets: allTargets ? "all" : [...selected], expiresAt: new Date(Date.now() + serverOffset + hours * 36e5).toISOString() };
  const recipients = allTargets ? "\u6240\u6709\u5BA2\u6237\u7AEF\uFF08\u5305\u62EC\u65B0\u6CE8\u518C\u5BA2\u6237\u7AEF\uFF09\u53CA\u516C\u7F51\u9875\u9762" : [...selected].map((id) => `${boxIndex.get(id)?.name ?? id} (${id})`).join("\n");
  confirmAction("\u786E\u8BA4\u53D1\u5E03\u516C\u544A", `${body.title}

${body.text}

\u63A5\u6536\u8005\uFF1A
${recipients}

\u6709\u6548\u81F3\uFF1A${date(body.expiresAt)}`, async () => {
    await api("announcements", "POST", body);
    input("notice-text").value = "";
  });
};
if (localStorage.getItem("terrarium-admin-theme") === "light") document.body.classList.add("light");
el("theme").onclick = () => {
  document.body.classList.toggle("light");
  localStorage.setItem("terrarium-admin-theme", document.body.classList.contains("light") ? "light" : "dark");
};
setInterval(() => {
  if (!document.hidden) {
    void refresh();
    renderCountdown();
  }
}, 15e3);
setInterval(() => {
  if (!document.hidden && token) renderCountdown();
}, 1e3);
if (token) void refresh();
el("minimum-form").onsubmit = (event) => {
  event.preventDefault();
  const version2 = input("minimum-version").value.trim(), at = input("minimum-at").value.trim();
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version2) || at && !/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/.test(at)) {
    status("\u8BF7\u8F93\u5165\u7248\u672C\u53F7\u53CA\u6709\u6548\u7684\u5317\u4EAC\u65F6\u95F4");
    return;
  }
  confirmAction("\u786E\u8BA4\u6700\u4F4E\u5BA2\u6237\u7AEF\u7248\u672C", `\u6700\u4F4E\u7248\u672C ${version2}\uFF0C${at ? `${at}\uFF08\u5317\u4EAC\u65F6\u95F4\uFF09\u751F\u6548` : "\u7ACB\u5373\u751F\u6548"}\u3002\u4F4E\u7248\u672C\u548C\u672A\u4E0A\u62A5\u7248\u672C\u7684\u63D0\u4EA4\u5C06\u88AB\u5FFD\u7565\u3002`, async () => {
    await api("controls/minimum-client-version", "PUT", { version: version2, ...at ? { at: at.replace(" ", "T") + ":00+08:00" } : {} });
  });
};
el("minimum-off").onclick = () => confirmAction("\u53D6\u6D88\u7248\u672C\u9650\u5236", "\u5141\u8BB8\u6240\u6709\u5BA2\u6237\u7AEF\u7248\u672C\u63D0\u4EA4\u3002HTTP Push \u5F00\u5173\u4ECD\u5355\u72EC\u751F\u6548\u3002", async () => {
  await api("controls/minimum-client-version", "PUT", { version: null });
});
el("minimum-cancel").onclick = () => confirmAction("\u53D6\u6D88\u7248\u672C\u5B9A\u65F6\u53D8\u66F4", "\u4FDD\u7559\u5F53\u524D\u6700\u4F4E\u5BA2\u6237\u7AEF\u7248\u672C\u3002", async () => {
  await api("controls/minimum-client-version", "PUT", { cancel: true });
});

for (const [id, body] of [["forum-on", {enabled:true}], ["forum-off", {enabled:false}], ["forum-cancel", {cancel:true}]]) {
  el(id).onclick = () => confirmAction("更改论坛开放状态", body.cancel ? "取消定时变更，保留当前状态。" : body.enabled ? "开放论坛阅读和 AI 互动。" : "暂停论坛阅读和互动，保留帖子。", async () => { await api("controls/forum", "PUT", body); });
}
el("forum-form").onsubmit = event => {
  event.preventDefault(); const value = input("forum-at").value;
  const at = new Date(`${value.replace(" ","T")}:00+08:00`);
  const canonical = Number.isFinite(at.getTime()) ? new Intl.DateTimeFormat("sv-SE", {timeZone:"Asia/Shanghai",year:"numeric",month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit",hour12:false}).format(at) : "";
  if (canonical !== value || at.getTime() <= Date.now()+serverOffset) {status("请输入有效的未来北京时间"); return;}
  confirmAction("定时开放论坛", `${value}（北京时间）开放论坛。`, async () => {await api("controls/forum","PUT",{enabled:true,at:at.toISOString()});});
};

function logoLabel(version) { return ({"0.8.6":"闲看生机 · 0.8.6","0.8.7":"新邻初见 · 第一阶段 · 0.8.7","0.8.8":"新邻初见 · 第二阶段 · 0.8.8","0.8.9":"照记影踪 · 0.8.9","0.8.10":"新邻初见 · 第三阶段 · 0.8.10","0.9.0":"满庭新事 · 0.9.0"})[version] ?? String(version); }
let logoScheduleVersion = "0.8.8";
for (const button of document.querySelectorAll("#logo-phase-choice button")) button.onclick=()=>{logoScheduleVersion=button.dataset.version;for(const other of document.querySelectorAll("#logo-phase-choice button"))other.setAttribute("aria-pressed",String(other===button));};
for (const [id,body] of [["logo-old",{version:"0.8.6"}],["logo-new",{version:"0.8.7"}],["logo-phase2",{version:"0.8.8"}],["logo-photo",{version:"0.8.9"}],["logo-phase3",{version:"0.8.10"}],["logo-garden",{version:"0.9.0"}],["logo-cancel",{cancel:true}]]) el(id).onclick=()=>confirmAction("更换首页 Logo",body.cancel?"取消定时更换。":`显示 ${body.version} 的版本 Logo。`,async()=>{await api("controls/logo","PUT",body)});
el("logo-form").onsubmit=event=>{event.preventDefault();const value=input("logo-at").value;const at=new Date(`${value.replace(" ","T")}:00+08:00`);const canonical=Number.isFinite(at.getTime())?new Intl.DateTimeFormat("sv-SE",{timeZone:"Asia/Shanghai",year:"numeric",month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit",hour12:false}).format(at):"";if(canonical!==value||at.getTime()<=Date.now()+serverOffset){status("请输入有效的未来北京时间");return}confirmAction("定时更换 Logo",`${value}（北京时间）显示${logoLabel(logoScheduleVersion)}。`,async()=>{await api("controls/logo","PUT",{version:logoScheduleVersion,at:at.toISOString()})})};
