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
  gib: (b) => unit(b, 1024, ["B", "KiB", "MiB", "GiB", "TiB"]),      // RAM
  size: (b) => unit(b, 1000, ["B", "kB", "MB", "GB", "TB"]),          // disks
  rate: (b) => unit(b, 1000, ["B/s", "kB/s", "MB/s", "GB/s"]),        // throughput
  mbit: (b) => `${(b / 1e6).toFixed(1)} Mbit/s`,
  pct: (p) => `${Math.round(p)} %`,
  temp: (t) => `${t.toFixed(t >= 100 ? 0 : 1)} °C`,
  num: (n) => n.toLocaleString("en-US"),
  uptime(s) {
    const d = Math.floor(s / 86400), h = Math.floor(s / 3600) % 24, m = Math.floor(s / 60) % 60;
    return d ? `${d}d ${h}h ${m}m` : `${h}h ${m}m`;
  },
};

// ---------- Small HTML builders ----------

const LEVEL_TEXT = { ok: "OK", warn: "Warning", crit: "Critical" };

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
    if (area) plot += `<polygon class="area ${cls}" points="${x(0, n).toFixed(1)},${H} ${pts} ${W},${H}"/>`;
    plot += `<polyline class="line ${cls}" points="${pts}"/>`;
  }
  svg.innerHTML = `${grid}<g>${plot}</g>`;
  svg.graphData = { series, max };
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
      const top = (1 - Math.min(1, values[i] / data.max)) * 100;
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
  card("temps", t, () => {
    const temp = (v, lim) => (v == null ? NA() : `${hot(tempHeat(v, lim), fmt.temp(v))}${flag(level(v, lim))}`);
    const fans = (t.fans || []).length
      ? t.fans.map((f) => row(esc(f.label), `${fmt.num(f.rpm)} RPM`)).join("")
      : row("Fans", NA());
    return `<div class="rows">
      ${row("CPU", temp(t.cpu, limits.cpu_temp))}
      ${row("Mainboard", temp(t.board, limits.cpu_temp))}
      ${row(t.disk ? `Hottest disk · ${esc(t.disk.name)}` : "Hottest disk", t.disk ? temp(t.disk.temp, limits.disk_temp) : NA())}
      <div class="sep"></div>
      ${fans}
    </div>`;
  });
}

function renderMemory(m, limits) {
  card("memory", m, () => {
    const usedPct = (m.used / m.total) * 100;
    const cachePct = (m.cached / m.total) * 100;
    const free = Math.max(0, m.total - m.used - m.cached);
    const swapPct = m.swap_total ? (m.swap_used / m.swap_total) * 100 : 0;
    const lvl = level(m.percent, limits.usage);
    return `<div class="stats">${stat("Used", fmt.gib(m.used), `/ ${fmt.gib(m.total)}`)}</div>
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

function renderNetwork(n) {
  push(history.down, n.ok ? n.down : 0);
  push(history.up, n.ok ? n.up : 0);
  const target = Math.max(1e6, ...history.down, ...history.up) * 1.1;
  netMax = netMax ? netMax + (target - netMax) * (target > netMax ? 0.5 : 0.15) : target;
  const max = netMax;
  graph($("net-graph"), [
    { values: history.down, cls: "line-down", area: true },
    { values: history.up, cls: "line-up" },
  ], max);
  $("net-scale").textContent = fmt.rate(max);
  $("net-iface").textContent = n.iface || "";

  card("network", n, () => `<div class="stats">
      ${stat("↓ Download", fmt.rate(n.down), "", "down", 10)}
      ${stat("↑ Upload", fmt.rate(n.up), "", "up", 10)}
    </div>`);
}

function renderDocker(d) {
  const ok = card("docker", d, () => {
    if (!d.containers.length) return '<span class="muted">No containers</span>';
    return `<table class="fixed"><thead><tr><th>Container</th><th class="w-status">Status</th><th class="num w-cpu">CPU</th><th class="num w-mem">RAM</th></tr></thead><tbody>
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

  card("disks", disks, () => {
    if (!disks.items.length) return '<span class="muted">No disks found</span>';
    const smart = (v) => (v === true ? `<span class="pill lvl-ok">OK</span>`
      : v === false ? `<span class="pill lvl-crit">FAILED</span>` : NA());
    return `<table class="disks fixed"><thead><tr>
        <th class="w-disk">Disk</th><th>Used</th><th class="num w-size">Size</th><th class="num w-temp">Temp</th>
        <th class="num w-read">Read</th><th class="num w-hours">Power-on</th><th class="w-smart">SMART</th>
      </tr></thead><tbody>
      ${disks.items.map((d) => {
        const lvl = d.percent != null ? level(d.percent, limits.usage) : "ok";
        const tl = d.temp != null ? level(d.temp, limits.disk_temp) : "ok";
        return `<tr>
          <td class="name"><div>${esc(d.name)}</div><div class="model">${esc(d.model || "")}</div></td>
          <td class="use" data-label="Used">${d.used != null
            ? `${fmt.size(d.used)} / ${fmt.size(d.total)} · ${fmt.pct(d.percent)}${flag(lvl)}${bar(d.percent)}`
            : NA("not mounted")}</td>
          <td class="num" data-label="Size">${d.size ? fmt.size(d.size) : NA()}</td>
          <td class="num" data-label="Temp">${d.temp != null ? `${hot(tempHeat(d.temp, limits.disk_temp), `${d.temp} °C`)}${flag(tl)}` : NA()}</td>
          <td class="num" data-label="Read">${d.read != null ? fmt.rate(d.read) : NA()}</td>
          <td class="num" data-label="Power-on hours">${d.hours != null ? `${fmt.num(d.hours)} h` : NA()}</td>
          <td data-label="SMART">${smart(d.smart)}</td></tr>`;
      }).join("")}</tbody></table>`;
  });
}

function renderProcesses(p) {
  card("processes", p, () => `<table><thead><tr>
      <th>Name</th><th class="w-user">User</th><th class="num w-cpu">CPU</th><th class="num w-pct">RAM</th></tr></thead><tbody>
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
      const method = transcode ? `<span class="pill accent">Transcode</span>`
        : `<span class="pill">${s.method === "DirectStream" ? "Direct Stream" : "Direct Play"}</span>`;
      return `<div class="stream">
        <div class="title">${s.paused ? "❚❚ " : ""}${esc(s.title)}</div>
        <div class="info"><span>${esc(s.user)} · ${esc(s.device)}</span>
          <span>${method} ${s.bitrate ? fmt.mbit(s.bitrate) : ""}</span></div>
      </div>`;
    }).join("")}</div>`;
  });
  $("jf-count").textContent = ok ? `${j.streams.length} ${j.streams.length === 1 ? "Stream" : "Streams"}` : "";
}

function renderMinecraft(m) {
  card("minecraft", m, () => {
    const lvl = m.online ? "ok" : "crit";
    return `<div class="mc">
      <span class="pill lvl-${lvl}">${m.online ? "Online" : "Offline"}</span>
      ${m.online ? `<span class="big">${m.players}<small class="muted"> / ${m.max}</small></span><span class="muted">players</span>` : ""}
    </div>${m.online && m.version ? `<div class="muted" style="margin-top:10px">Version ${esc(m.version)}</div>` : ""}`;
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
}

// ---------- WebSocket with automatic reconnect ----------

let retryDelay = 1000;

function connect() {
  const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
  ws.onopen = () => {
    retryDelay = 1000;
    $("conn").textContent = "● Live";
    $("grid").classList.remove("stale");
  };
  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      if (data.history) {
        // First message: the last 2 minutes, kept by the server, so graphs start full.
        for (const key of ["cpu", "down", "up"]) history[key] = data.history[key].slice(-(HISTORY + 1));
        netMax = 0;
        return;
      }
      render(data);
    } catch (err) { console.error(err); }
  };
  ws.onclose = () => {
    $("conn").textContent = "○ Disconnected – reconnecting …";
    $("grid").classList.add("stale");
    setTimeout(connect, retryDelay);
    retryDelay = Math.min(retryDelay * 2, 10000);
  };
}

connect();
