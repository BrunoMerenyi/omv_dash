// OMV Dashboard – receives one snapshot per second over a WebSocket and renders it.
"use strict";

const HISTORY = 120; // seconds of graph history kept in the browser
const history = { cpu: [], down: [], up: [] };
const $ = (id) => document.getElementById(id);

// ---------- Formatting ----------

const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (c) => (
  { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]
));

function unit(value, base, units, digits = 1) {
  let i = 0;
  while (Math.abs(value) >= base && i < units.length - 1) { value /= base; i++; }
  return `${value.toFixed(i === 0 ? 0 : digits)} ${units[i]}`;
}
const fmt = {
  gib: (b) => unit(b, 1024, ["B", "KiB", "MiB", "GiB", "TiB"]),      // RAM and disks
  size: (b) => unit(b, 1024, ["B", "KiB", "MiB", "GiB", "TiB"]),
  bits: (b) => unit(b, 1000, ["bps", "kbps", "Mbps", "Gbps"]),        // bits per second
  rate: (bytes) => fmt.bits(bytes * 8),                               // byte counters -> bps
  mbit: (b) => fmt.bits(b),
  pct: (p) => `${Math.round(p)} %`,
  temp: (t) => `${t.toFixed(t >= 100 ? 0 : 1)} °C`,
  num: (n) => n.toLocaleString("en-US"),
  uptime(s) {
    const d = Math.floor(s / 86400), h = Math.floor(s / 3600) % 24, m = Math.floor(s / 60) % 60;
    return d ? `${d}d ${h}h ${m}m` : `${h}h ${m}m`;
  },
};

// ---------- Small HTML builders ----------

const LEVEL_TEXT = { ok: "healthy", warn: "warning", crit: "critical" };

const NA = (reason) => `<span class="na">Not available${reason ? `<small>${esc(reason)}</small>` : ""}</span>`;
const loading = '<span class="na">Loading …</span>';

// Returns "ok", "warn" or "crit" for a value and a [warn, crit] pair of limits.
const level = (value, [warn, crit]) => (value >= crit ? "crit" : value >= warn ? "warn" : "ok");

// A level is never shown by colour alone: warn/crit always add a text flag.
const flag = (lvl) => (lvl === "ok" ? "" : `<span class="flag lvl-${lvl}">${lvl === "crit" ? "CRIT" : "HIGH"}</span>`);

// btop-style heat colour: green (idle) -> yellow -> red (busy). Same stops as --heat-* in CSS.
const clamp = (p) => Math.min(100, Math.max(0, p));
const heat = (p) => `hsl(${Math.round(130 - 1.3 * clamp(p))} 70% 65%)`;
const hot = (p, text) => `<span style="color:${heat(p)}">${text}</span>`;
// Temperatures on the same scale: 30 °C = green, the critical limit = red.
const tempHeat = (t, [, crit]) => ((t - 30) / (crit - 30)) * 100;

// The track shows the full heat gradient; a grey "rest" covers the unused part.
// Only the rest's width changes, so the colours stay put while it animates.
const bar = (percent) =>
  `<div class="bar heat" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(percent)}">` +
  `<span class="rest" style="width:${100 - clamp(percent)}%"></span></div>`;

// `chars`: reserve room for that many characters, so a changing value
// (9 % -> 10 %, kB/s -> MB/s) never pushes the stats next to it around.
const stat = (label, value, extra = "", cls = "", chars = 0) =>
  `<div class="stat"><div class="label">${label}</div><div class="value ${cls}"${chars ? ` style="min-width:${chars}ch"` : ""}>` +
  `${value}${extra ? `<small>${extra}</small>` : ""}</div></div>`;

const row = (key, value) => `<div class="row"><span class="k">${key}</span><span class="v">${value}</span></div>`;

// ---------- Smooth updates ----------
// Instead of replacing a card's HTML every second (which makes bars jump),
// patch() changes only the attributes and text that differ. The elements stay
// the same, so CSS transitions can animate bar widths to their new value.

const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

function patch(el, html) {
  const next = document.createElement("div");
  next.innerHTML = html;
  morph(el, next);
}

function morph(current, next) {
  const a = current.childNodes, b = next.childNodes;
  if (a.length !== b.length) {             // structure changed: just replace
    current.replaceChildren(...Array.from(b));
    return;
  }
  for (let i = 0; i < b.length; i++) {
    const x = a[i], y = b[i];
    if (x.nodeName !== y.nodeName) { x.replaceWith(y.cloneNode(true)); continue; }
    if (x.nodeType === Node.TEXT_NODE) { if (x.data !== y.data) x.data = y.data; continue; }
    if (x.nodeType !== Node.ELEMENT_NODE) continue;
    for (const { name, value } of y.attributes) if (x.getAttribute(name) !== value) x.setAttribute(name, value);
    for (const { name } of Array.from(x.attributes)) if (!y.hasAttribute(name)) x.removeAttribute(name);
    morph(x, y);
  }
}

// Render a card body, or "Not available" if the collector reported a problem.
function card(id, section, render) {
  const el = $(id);
  if (!section || !section.ok) {
    patch(el, section && section.pending ? loading : NA(section && section.error));
    return false;
  }
  patch(el, render(section));
  return true;
}

// ---------- Graphs (inline SVG, 300 x 60 units, stretched to the card width) ----------

function push(list, value) {
  list.push(value);
  if (list.length > HISTORY + 1) list.shift(); // one extra point, hidden left of the edge
}

/** Draw a scrolling history graph. Each series: { values, cls, area }. */
function graph(svg, series, max) {
  const W = 300, H = 60, step = W / (HISTORY - 1);
  const x = (i, n) => W - (n - 1 - i) * step;    // newest point at the right edge
  const y = (v) => H - Math.min(1, v / max) * (H - 2) - 1;
  const grid = [0.25, 0.5, 0.75].map((f) => `<line class="grid-line" x1="0" x2="${W}" y1="${H * f}" y2="${H * f}"/>`).join("");
  let plot = "";
  if (series.some((s) => s.cls === "heat")) {
    plot += `<defs><linearGradient id="heat-grad" gradientUnits="userSpaceOnUse" x1="0" y1="${H}" x2="0" y2="0">
      <stop offset="0" stop-color="${heat(0)}"/><stop offset=".5" stop-color="${heat(50)}"/><stop offset="1" stop-color="${heat(100)}"/>
    </linearGradient></defs>`;
  }
  for (const { values, cls, area } of series) {
    const n = values.length;
    if (n < 2) continue;
    const pts = values.map((v, i) => `${x(i, n).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
    if (area) plot += `<polygon class="area ${cls}" mask="url(#fade)" points="${x(0, n).toFixed(1)},${H} ${pts} ${W},${H}"/>`;
    plot += `<polyline class="line ${cls}" points="${pts}"/>`;
  }
  svg.innerHTML = `${grid}<g>${plot}</g>`;
  // Where each series sits (0–100 % from the top) – used by the hover tooltip.
  svg.graphData = { series, top: series.map(() => (v) => (y(v) / H) * 100) };
  if (svg.refreshTip) svg.refreshTip();
  // Start one step to the right and glide left over the next second, so the
  // graph scrolls continuously instead of jumping once per update.
  if (!reduceMotion) {
    svg.lastChild.animate(
      [{ transform: `translateX(${step}px)` }, { transform: "translateX(0)" }],
      { duration: 1000, easing: "linear" },
    );
  }
}

// ---------- Small history graphs (disk IO, temperatures, fans) ----------

const SPARK_W = 60, SPARK_H = 20;  // viewBox units; the graph is stretched to its box
const diskHistory = {};            // disk name -> recent busy %
const tempHistory = {};            // "cpu", "board", "disk", "fan:<label>" -> recent values

/**
 * A small history graph. lo/hi: value range, points: seconds shown.
 * tip: { src: "disk:<name>" | "temp:<key>", label, unit } for the hover tooltip.
 */
function spark(values = [], lo = 0, hi = 100, points = 60, cls = "", tip = null) {
  const step = SPARK_W / (points - 1);
  const data = tip ? ` data-src="${esc(tip.src)}" data-label="${esc(tip.label)}" data-unit="${tip.unit}" data-points="${points}"` : "";
  const open = `<svg class="spark ${cls}" viewBox="0 0 ${SPARK_W} ${SPARK_H}" preserveAspectRatio="none" data-step="${step}"${data} aria-hidden="true">`;
  const last = values.slice(-(points + 1));
  if (last.length < 2) return `${open}<g></g></svg>`;
  const x = (i) => SPARK_W - (last.length - 1 - i) * step;
  const y = (v) => SPARK_H - Math.min(1, Math.max(0, (v - lo) / (hi - lo))) * (SPARK_H - 2) - 1;
  const pts = last.map((v, i) => `${x(i).toFixed(2)},${y(v).toFixed(2)}`).join(" ");
  return `${open}<g><polygon class="area" mask="url(#fade)" points="${x(0).toFixed(2)},${SPARK_H} ${pts} ${SPARK_W},${SPARK_H}"/>` +
    `<polyline class="line" points="${pts}"/></g></svg>`;
}

/** Let every small graph inside `root` glide one step left, like the big graphs. */
function slideSparks(root) {
  if (reduceMotion) return;
  for (const svg of root.querySelectorAll(".spark")) {
    svg.firstElementChild.animate(
      [{ transform: `translateX(${svg.dataset.step}px)` }, { transform: "translateX(0)" }],
      { duration: 1000, easing: "linear" },
    );
  }
}

// ---------- Tooltips for the small graphs (same look as the big ones) ----------

const sparkTip = { svg: null, x: 0 };

function setupSparkTips() {
  document.body.insertAdjacentHTML("beforeend", '<div class="ftip" hidden></div><div class="fline" hidden></div>');
  const move = (event) => {
    sparkTip.svg = event.target.closest ? event.target.closest(".spark[data-src]") : null;
    sparkTip.x = event.clientX;
    showSparkTip();
  };
  document.addEventListener("pointermove", move);
  document.addEventListener("pointerdown", move); // phones: tap / drag
  window.addEventListener("scroll", () => { sparkTip.svg = null; showSparkTip(); }, { passive: true });
}

/** Show (or refresh, after each update) the tooltip for the hovered small graph. */
function showSparkTip() {
  const tip = document.querySelector(".ftip"), line = document.querySelector(".fline");
  const svg = sparkTip.svg;
  const hide = () => { tip.hidden = line.hidden = true; };
  if (!svg || !svg.isConnected) return hide();
  const r = svg.getBoundingClientRect();
  const points = Number(svg.dataset.points);
  const ago = Math.round((1 - Math.min(1, Math.max(0, (sparkTip.x - r.left) / r.width))) * (points - 1));
  const [kind, ...rest] = svg.dataset.src.split(":");
  const values = (kind === "disk" ? diskHistory : tempHistory)[rest.join(":")] || [];
  const v = values[values.length - 1 - ago];
  if (v === undefined) return hide();

  const x = r.left + (1 - ago / (points - 1)) * r.width; // snap to that second
  const unit = svg.dataset.unit;
  const value = unit === "rpm" ? `${fmt.num(Math.round(v))} rpm` : unit === "°C" ? fmt.temp(v) : `${Math.round(v)} %`;
  tip.innerHTML = `<div class="muted">${ago ? `${ago} s ago` : "now"}</div>${esc(svg.dataset.label)} · ${value}`;
  tip.hidden = line.hidden = false;
  Object.assign(line.style, { left: `${x}px`, top: `${r.top}px`, height: `${r.height}px` });
  Object.assign(tip.style, {
    left: `${x}px`, top: `${r.top}px`,
    transform: x > innerWidth - 240 ? "translate(calc(-100% - 10px), -100%)" : "translate(10px, -100%)",
  });
}

// ---------- Graph tooltips ----------
// Hover (or drag a finger across) a graph to see the exact values at that second.

function seriesColor(cls, value) {
  return cls === "heat" ? heat(value) : `var(--net-${cls === "line-up" ? "up" : "down"})`;
}

function setupTooltip(svg, format) {
  const box = svg.parentElement;
  box.insertAdjacentHTML("beforeend", '<div class="crosshair" hidden></div><div class="tip" hidden></div><div class="dots"></div>');
  const line = box.querySelector(".crosshair");
  const tip = box.querySelector(".tip");
  const dots = box.querySelector(".dots");
  let fraction = null; // pointer position across the graph, 0 = left, 1 = right

  const hide = () => { line.hidden = tip.hidden = true; dots.innerHTML = ""; };

  svg.refreshTip = () => {
    const data = svg.graphData;
    if (fraction === null || !data) return hide();
    const ago = Math.round((1 - fraction) * (HISTORY - 1));  // seconds before now
    const values = data.series.map((s) => s.values[s.values.length - 1 - ago]);
    if (values[0] === undefined) return hide();               // no data that far back yet

    const x = (1 - ago / (HISTORY - 1)) * 100;               // snap to that second
    line.hidden = tip.hidden = false;
    line.style.left = tip.style.left = `${x}%`;
    tip.classList.toggle("flip", x > 60);                    // keep the box inside the card
    tip.innerHTML = `<div class="muted">${ago ? `${ago} s ago` : "now"}</div>${format(values)}`;
    dots.innerHTML = data.series.map((s, i) => {
      const top = data.top[i](values[i]);
      return `<i style="left:${x}%;top:${top}%;background:${seriesColor(s.cls, values[i])}"></i>`;
    }).join("");
  };

  svg.addEventListener("pointermove", (event) => {
    const rect = svg.getBoundingClientRect();
    fraction = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    svg.refreshTip();
  });
  svg.addEventListener("pointerleave", () => { fraction = null; hide(); });
}

setupSparkTips();
setupTooltip($("cpu-graph"), ([total]) => `Total ${hot(total, fmt.pct(total))}`);
setupTooltip($("net-graph"), ([down, up]) =>
  `<div style="color:var(--net-down)">↓ ${fmt.rate(down)}</div><div style="color:var(--net-up)">↑ ${fmt.rate(up)}</div>`);

// ---------- Sections ----------

function renderHeader(s) {
  $("title").textContent = s.title;
  document.title = `${s.title} Dashboard`;
  $("demo").hidden = !s.demo;
  $("ip").textContent = s.host.ip || "Not available";
  $("uptime").textContent = s.host.uptime != null ? fmt.uptime(s.host.uptime) : "Not available";

  const { level: lvl, reasons } = s.status;
  const el = $("status");
  el.className = `status lvl-${lvl}`;
  el.title = reasons.join(", ");
  patch(el, `<span>${LEVEL_TEXT[lvl]}</span>` +
    (reasons.length ? `<span class="reasons">· ${reasons.map(esc).join(", ")}</span>` : ""));
}

// A 270° "dome" gauge (CPU usage and temperature): the arc fills clockwise from bottom-left to bottom-right.
// The stroke uses the heat gradient left to right, so a fuller arc reaches
// further into yellow and red. pathLength="100" lets the fill be set in percent.
function gauge(value, percent, unit, label) {
  const shown = value == null ? "–" : Math.round(value);
  const p = value == null ? 0 : clamp(percent);
  const arc = "M14.8 57.2 A30 30 0 1 1 57.2 57.2"; // circle r=30 around (36,36), 135° -> 45°
  return `<div class="gauge" role="img" aria-label="${label}: ${value == null ? "not available" : `${shown} ${unit}`}">
    <svg viewBox="0 0 72 72" aria-hidden="true">
      <defs><linearGradient id="gauge-heat" gradientUnits="userSpaceOnUse" x1="6" y1="0" x2="66" y2="0">
        <stop offset="0" stop-color="${heat(0)}"/><stop offset=".5" stop-color="${heat(50)}"/><stop offset="1" stop-color="${heat(100)}"/>
      </linearGradient></defs>
      <path class="gauge-track" d="${arc}" pathLength="100"/>
      <path class="gauge-fill" d="${arc}" pathLength="100" style="stroke-dasharray:${p.toFixed(1)} 100${p ? "" : ";opacity:0"}"/>
    </svg>
    <div class="gauge-text"><span style="color:${value == null ? "var(--muted)" : heat(p)}">${shown}</span><small>${unit}</small></div>
  </div>`;
}

// Load average: how many threads were running or waiting, averaged over 1, 5
// and 15 minutes. Coloured relative to the thread count (16 on 16 threads = red).
function loadStat([one, five, fifteen] = [], threads) {
  if (one == null) return stat("Load", "–");
  return `<div class="stat" title="Load average over 1, 5 and 15 minutes. Above ${threads} means more work than threads.">
    <div class="label">Load · 1/5/15 min</div>
    <div class="value" style="min-width:12ch">${hot(one / threads * 100, one.toFixed(2))}<small> ${five.toFixed(2)}  ${fifteen.toFixed(2)}</small></div>
  </div>`;
}

function renderCpu(cpu, temps, limits) {
  push(history.cpu, cpu.ok ? cpu.percent : 0);
  graph($("cpu-graph"), [{ values: history.cpu, cls: "heat", area: true }], 100);
  const counts = cpu.ok && cpu.cores ? ` · ${cpu.cores}C/${cpu.threads.length}T` : "";
  $("cpu-model").textContent = (cpu.model || "") + counts;
  const temp = temps && temps.ok ? temps.cpu : null; // null -> gauge shows "–"

  const ok = card("cpu", cpu, (c) => `<div class="stats">
      <div class="stat gauge-stat"><div class="label">Usage</div>${gauge(c.percent, c.percent, "%", "CPU usage")}</div>
      <div class="stat gauge-stat"><div class="label">Temp</div>${gauge(temp,
        temp == null ? 0 : tempHeat(temp, limits.cpu_temp), "°C", "CPU temperature")}</div>
      ${stat("Clock", c.freq_mhz ? fmt.num(c.freq_mhz) : "–", c.freq_mhz ? "MHz" : "", "", 7)}
      ${loadStat(c.load, c.threads.length)}
    </div>`);
  // One row per physical core, with a labelled bar for each of its threads.
  const cores = ok ? cpu.core_threads || cpu.threads.map((_, i) => [i]) : [];
  $("threads").style.setProperty("--per-core", Math.max(1, ...cores.map((c) => c.length)));
  patch($("threads"), cores.map((threads, core) => {
    const avg = threads.reduce((sum, i) => sum + cpu.threads[i], 0) / threads.length;
    return `<div class="core">
      <span class="core-name">Core ${core + 1}</span>
      <span class="pct core-avg" title="Core ${core + 1} average">${hot(avg, `${Math.round(avg)} %`)}</span>
      ${threads.map((i) => `<div class="thread" title="Thread ${i}: ${cpu.threads[i]} %">
        <span class="tid">T${i}</span>${bar(cpu.threads[i])}<span class="pct">${hot(cpu.threads[i], `${cpu.threads[i]} %`)}</span>
      </div>`).join("")}
    </div>`;
  }).join(""));
}

function renderTemps(t, limits) {
  const fanKey = (f) => `fan:${f.label}`;
  if (t.ok) {
    for (const [key, value] of [["cpu", t.cpu], ["board", t.board], ["disk", t.disk && t.disk.temp]]) {
      if (value != null) push(tempHistory[key] ||= [], value);
    }
    for (const f of t.fans || []) push(tempHistory[fanKey(f)] ||= [], f.rpm);
  }
  card("temps", t, () => {
    const tempRow = (key, label, value, [, crit]) => `<div class="trow">
        <span class="k">${label}</span>${spark(tempHistory[key], 25, crit, 120, "", { src: `temp:${key}`, label, unit: "°C" })}
        <span class="v">${value == null ? NA() : `${hot(tempHeat(value, [0, crit]), fmt.temp(value))}${flag(level(value, key === "disk" ? limits.disk_temp : limits.cpu_temp))}`}</span>
      </div>`;
    const fanRow = (f) => {
      const hist = tempHistory[fanKey(f)] || [];
      // Fans: scale to their own recent range so changes are visible (0 rpm would make a solid block).
      return `<div class="trow">
        <span class="k">${esc(f.label)}</span>${spark(hist, Math.min(f.rpm, ...hist) * 0.9, Math.max(f.rpm, ...hist) * 1.1, 120, "fan", { src: `temp:${fanKey(f)}`, label: f.label, unit: "rpm" })}
        <span class="v">${fmt.num(f.rpm)} <small class="unit">rpm</small></span>
      </div>`;
    };
    return `<div class="trows">
      ${tempRow("cpu", "cpu", t.cpu, limits.cpu_temp)}
      ${tempRow("board", "mainboard", t.board, limits.cpu_temp)}
      ${tempRow("disk", t.disk ? `disk · ${esc(t.disk.name)}` : "hottest disk", t.disk && t.disk.temp, limits.disk_temp)}
      <div class="sep"></div>
      ${(t.fans || []).length ? t.fans.map(fanRow).join("") : `<div class="trow"><span class="k">fans</span><span></span><span class="v">${NA()}</span></div>`}
    </div>`;
  });
  slideSparks($("temps"));
}

function renderMemory(m, limits) {
  card("memory", m, () => {
    const usedPct = (m.used / m.total) * 100;
    const cachePct = (m.cached / m.total) * 100;
    const free = Math.max(0, m.total - m.used - m.cached);
    const swapPct = m.swap_total ? (m.swap_used / m.swap_total) * 100 : 0;
    const lvl = level(m.percent, limits.usage);
    return `<div class="stats">${stat("Used", fmt.gib(m.used), `/ ${fmt.gib(m.total)}`, "", 14)}</div>
      <div class="bar" role="meter" aria-label="RAM" aria-valuenow="${Math.round(usedPct)}" style="margin-bottom:14px">
        <span style="width:${usedPct}%;background:${heat(usedPct)}"></span><span class="cache" style="width:${cachePct}%"></span>
      </div>
      <div class="rows">
        ${row(`<i class="dot" style="background:${heat(usedPct)}"></i>Used`, `${fmt.gib(m.used)} · ${fmt.pct(usedPct)}${flag(lvl)}`)}
        ${row('<i class="dot" style="background:var(--mem-cache)"></i>Cache', fmt.gib(m.cached))}
        ${row('<i class="dot" style="background:var(--track)"></i>Free', fmt.gib(free))}
        <div class="sep"></div>
        ${row("Swap", m.swap_total ? `${fmt.gib(m.swap_used)} / ${fmt.gib(m.swap_total)}` : "none")}
        ${m.swap_total ? bar(swapPct) : ""}
      </div>`;
  });
}

let netMax = 0; // smoothed top of the network graph

// Let the top of a graph glide to a new scale instead of jumping.
const glide = (current, target) => (current ? current + (target - current) * (target > current ? 0.5 : 0.15) : target);

function renderNetwork(n) {
  push(history.down, n.ok ? n.down : 0);
  push(history.up, n.ok ? n.up : 0);
  netMax = glide(netMax, Math.max(1e6, ...history.down, ...history.up) * 1.1);
  graph($("net-graph"), [
    { values: history.down, cls: "line-down", area: true },
    { values: history.up, cls: "line-up", area: true },
  ], netMax);
  $("net-scale").textContent = fmt.rate(netMax);
  $("net-iface").textContent = n.iface || "";

  card("network", n, () => `<div class="stats">
      ${stat("↓ Download", fmt.rate(n.down), "", "down", 10)}
      ${stat("↑ Upload", fmt.rate(n.up), "", "up", 10)}
    </div>`);
}

function renderDocker(d) {
  const ok = card("docker", d, () => {
    if (!d.containers.length) return '<span class="muted">No containers</span>';
    return `<table class="fixed"><thead><tr><th>Container</th><th class="w-status">Status</th><th class="num w-cpu" title="100 % = one full CPU thread">CPU</th><th class="num w-mem">RAM</th></tr></thead><tbody>
      ${d.containers.map((c) => {
        const lvl = c.state === "running" ? (c.status.includes("unhealthy") ? "warn" : "ok") : "crit";
        const label = c.status.includes("unhealthy") ? "unhealthy" : c.state;
        return `<tr><td class="name" title="${esc(c.status)}">${esc(c.name)}</td>
          <td><span class="state lvl-${lvl}">${esc(label)}</span></td>
          <td class="num">${c.cpu != null ? hot(c.cpu, `${c.cpu.toFixed(1)} %`) : "–"}</td>
          <td class="num">${c.mem != null ? fmt.gib(c.mem) : "–"}</td></tr>`;
      }).join("")}</tbody></table>`;
  });
  const running = ok ? d.containers.filter((c) => c.state === "running").length : 0;
  $("docker-count").textContent = ok ? `${running} / ${d.containers.length} running` : "";
}

function renderStorage(pool, disks, limits) {
  $("pool-path").textContent = pool.path || "";
  card("pool", pool, (p) => {
    const lvl = level(p.percent, limits.usage);
    return `<div class="pool">
      <div><div class="stat"><div class="label">mergerfs-Pool</div>
        <div class="value">${fmt.size(p.used)} <small>/ ${fmt.size(p.total)}</small></div></div></div>
      ${bar(p.percent)}
      <div class="num">${fmt.pct(p.percent)}${flag(lvl)}<div class="muted">${fmt.size(p.free)} free</div></div>
    </div>`;
  });

  for (const d of disks.items || []) push(diskHistory[d.name] ||= [], d.busy ?? 0);

  card("disks", disks, () => {
    if (!disks.items.length) return '<span class="muted">No disks found</span>';
    return `<div class="disk-list">${disks.items.map((d) => {
      const tl = d.temp != null ? level(d.temp, limits.disk_temp) : "ok";
      const smart = d.smart === false ? '<span class="state lvl-crit">SMART failed</span>'
        : d.smart === true && d.bad > 0 ? `<span class="state lvl-warn" title="${esc(d.bad_detail)}">${fmt.num(d.bad)} bad sectors</span>`
        : d.smart === true ? '<span class="state lvl-ok">SMART OK</span>' : '<span class="muted">no SMART</span>';
      const mounted = d.used != null;
      const free = mounted ? d.total - d.used : 0;
      return `<div class="disk">
        <div class="disk-head">
          <span class="disk-name">${esc(d.name)}</span><span class="model">${esc(d.model || "")}</span>
          <span class="facts">
            <span title="Temperature">${d.temp != null ? `${hot(tempHeat(d.temp, limits.disk_temp), `${d.temp} °C`)}${flag(tl)}` : '<span class="muted">– °C</span>'}</span>
            <span title="Power-on hours">${d.hours != null ? `${fmt.num(d.hours)} h` : '<span class="muted">– h</span>'}</span>
            <span title="Size">${d.size ? fmt.size(d.size) : "–"}</span>
            ${smart}
          </span>
        </div>
        <div class="meter io">
          <span class="k" title="How busy the disk is (100 % = it can't do more), last 60 s. Read ${fmt.rate(d.read || 0)} · Write ${fmt.rate(d.write || 0)}">IO</span>${spark(diskHistory[d.name], 0, 100, 60, "", { src: `disk:${d.name}`, label: `${d.name} busy`, unit: "%" })}
          <span class="pct">${d.busy != null ? hot(d.busy, `${Math.round(d.busy)} %`) : "–"}</span><span class="v"></span>
        </div>
        <div class="meter two">
          <span class="k">Used</span>${mounted ? bar(d.percent) : '<span class="muted">not mounted</span>'}
          <span class="pct">${mounted ? hot(d.percent, fmt.pct(d.percent)) : ""}</span>
          <span class="v">${mounted ? `<span class="uf"><span>${fmt.size(d.used)}</span><span class="muted">${fmt.size(free)} free</span></span>` : ""}</span>
        </div>
      </div>`;
    }).join("")}</div>`;
  });
  slideSparks($("disks"));
}

function renderProcesses(p) {
  card("processes", p, () => `<table class="fixed"><thead><tr>
      <th>Name</th><th class="w-user">User</th><th class="num w-cpu" title="100 % = one full CPU thread">CPU</th><th class="num w-pct">RAM</th></tr></thead><tbody>
    ${p.items.map((x) => `<tr><td class="name" title="PID ${x.pid}">${esc(x.name)}</td>
      <td class="muted ellipsis">${esc(x.user ?? "–")}</td>
      <td class="num">${hot(x.cpu, `${x.cpu.toFixed(1)} %`)}</td>
      <td class="num">${x.mem != null ? `${x.mem.toFixed(1)} %` : "–"}</td></tr>`).join("")}
    </tbody></table>`);
}

function renderJellyfin(j) {
  const ok = card("jellyfin", j, () => {
    if (!j.streams.length) return '<span class="muted">No active streams</span>';
    return `<div class="streams">${j.streams.map((s) => {
      const transcode = s.method === "Transcode";
      const method = transcode ? '<span class="method transcode">transcode</span>'
        : `<span class="method">${s.method === "DirectStream" ? "direct stream" : "direct play"}</span>`;
      return `<div class="stream">
        <div class="title">${s.paused ? "❚❚ " : ""}${esc(s.title)}</div>
        <div class="info"><span>${esc(s.user)} · ${esc(s.device)}</span>
          <span>${method} <span class="bitrate">${s.bitrate ? fmt.mbit(s.bitrate) : ""}</span></span></div>
        ${s.progress != null ? `<div class="progress" title="${s.progress} % watched"><span style="width:${s.progress}%"></span></div>` : ""}
        ${s.reason ? `<div class="reason">Transcoding: ${esc(s.reason)}</div>` : ""}
      </div>`;
    }).join("")}</div>`;
  });
  $("jf-count").textContent = ok ? `${j.streams.length} ${j.streams.length === 1 ? "Stream" : "Streams"}` : "";
}

function renderMinecraft(m) {
  card("minecraft", m, () => {
    const lvl = m.online ? "ok" : "crit";
    return `<div class="mc">
      <span class="state lvl-${lvl}">${m.online ? "online" : "offline"}</span>
      ${m.online ? `<span class="big">${m.players}<small class="muted"> / ${m.max}</small></span><span class="muted">players</span>` : ""}
    </div>${m.online && m.names && m.names.length ? `<div class="names">${m.names.map((n) => `<span>${esc(n)}</span>`).join("")}</div>` : ""}
    ${m.online && m.version ? `<div class="muted" style="margin-top:10px">Version ${esc(m.version)}</div>` : ""}`;
  });
}

function render(s) {
  renderHeader(s);
  renderCpu(s.cpu, s.temps, s.limits);
  renderTemps(s.temps, s.limits);
  renderMemory(s.memory, s.limits);
  renderNetwork(s.network);
  renderDocker(s.docker);
  renderStorage(s.pool, s.disks, s.limits);
  renderProcesses(s.processes);
  renderJellyfin(s.jellyfin);
  renderMinecraft(s.minecraft);
  showSparkTip(); // keep an open tooltip up to date
}

// ---------- WebSocket with automatic reconnect ----------

let retryDelay = 1000;

function connect() {
  const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
  ws.onopen = () => {
    retryDelay = 1000;
    $("conn").textContent = "live";
    $("conn").className = "conn";
    $("grid").classList.remove("stale");
  };
  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      if (data.history) {
        // First message: the last 2 minutes, kept by the server, so graphs start full.
        for (const key of ["cpu", "down", "up"]) history[key] = data.history[key].slice(-(HISTORY + 1));
        for (const [name, values] of Object.entries(data.history.disks || {})) diskHistory[name] = values.slice(-(HISTORY + 1));
        for (const [key, values] of Object.entries(data.history.temps || {})) tempHistory[key] = values.slice(-(HISTORY + 1));
        netMax = 0;
        return;
      }
      render(data);
    } catch (err) { console.error(err); }
  };
  ws.onclose = () => {
    $("conn").textContent = "reconnecting …";
    $("conn").className = "conn lvl-warn";
    $("grid").classList.add("stale");
    setTimeout(connect, retryDelay);
    retryDelay = Math.min(retryDelay * 2, 10000);
  };
}

connect();
