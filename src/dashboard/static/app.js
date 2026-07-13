// Goose live dashboard — SSE stream with poll fallback, charts, UX polish.
(function () {
  "use strict";

  // Auth bootstrap: read ?token= from the page URL, then strip it from the bar.
  var params = new URLSearchParams(window.location.search);
  var token = params.get("token") || "";
  if (token) {
    params.delete("token");
    var clean =
      window.location.pathname +
      (params.toString() ? "?" + params.toString() : "") +
      window.location.hash;
    try {
      window.history.replaceState({}, "", clean);
    } catch (_) {
      /* ignore */
    }
  }

  var bannerEl = document.getElementById("banner");
  var phaseBadge = document.getElementById("phase-badge");
  var hostsEl = document.getElementById("hosts");
  var durationEl = document.getElementById("duration");
  var connectionEl = document.getElementById("connection");
  var connectionLabel = document.getElementById("connection-label");
  var summaryEl = document.getElementById("summary-body");
  var aggregateEl = document.getElementById("aggregate-body");
  var requestsBody = document.getElementById("requests-body");
  var errorsBody = document.getElementById("errors-body");
  var requestsFilter = document.getElementById("requests-filter");
  var kpiUsers = document.getElementById("kpi-users");
  var kpiRps = document.getElementById("kpi-rps");
  var kpiFail = document.getElementById("kpi-fail");
  var kpiP95 = document.getElementById("kpi-p95");
  var kpiAvg = document.getElementById("kpi-avg");
  var subtitleEl = document.getElementById("subtitle");
  var footerNoteEl = document.getElementById("footer-note");
  var controlPanel = document.getElementById("control-panel");
  var ctrlStart = document.getElementById("ctrl-start");
  var ctrlStop = document.getElementById("ctrl-stop");
  var ctrlActive = document.getElementById("ctrl-active");
  var ctrlTarget = document.getElementById("ctrl-target");
  var ctrlApply = document.getElementById("ctrl-apply");
  var ctrlMinus = document.getElementById("ctrl-minus");
  var ctrlPlus = document.getElementById("ctrl-plus");
  var ctrlStep = document.getElementById("ctrl-step");
  var ctrlStatus = document.getElementById("ctrl-status");

  var pollTimer = null;
  var eventSource = null;
  var usingPoll = false;
  var authRequired = false;
  var authBlocked = false;
  var finished = false;

  // Control panel state (normative lastTarget/dirty algorithm)
  var controlEnabled = false;
  var lastTarget = null;
  var dirty = false;
  var step = 10;
  var inFlight = false;
  var lastSnap = null;
  var lastDisplayTarget = null;
  var currentPhase = "idle";
  var controlTokenMissing = false;

  // Table state
  var requestRows = [];
  var errorRows = [];
  var lastFlags = {};
  var reqSort = { key: "request_count", type: "num", dir: "desc" };
  var errSort = { key: "occurrences", type: "num", dir: "desc" };

  // Charts
  var chartRps = null;
  var chartUsers = null;
  var chartLatency = null;
  var chartsReady = typeof Chart !== "undefined";
  var chartLoadWarned = false;

  function setConnection(mode) {
    // mode: "live" | "poll" | "disconnected" | "connecting" | "closed"
    var cls = "disconnected";
    var label = "disconnected";
    if (mode === "live") {
      cls = "live";
      label = "SSE";
    } else if (mode === "poll") {
      cls = "poll";
      label = "poll";
    } else if (mode === "connecting") {
      cls = "disconnected";
      label = "connecting";
    } else if (mode === "closed") {
      cls = "finished";
      label = "finished";
    }
    connectionEl.className = "connection " + cls;
    connectionLabel.textContent = label;
  }

  function setBanner(text, kind) {
    if (!text) {
      bannerEl.textContent = "";
      bannerEl.className = "banner hidden";
      return;
    }
    bannerEl.textContent = text;
    bannerEl.className = "banner " + (kind || "info");
  }

  function setPhase(phase) {
    var p = (phase || "idle").toLowerCase();
    var known = {
      idle: true,
      increase: true,
      maintain: true,
      decrease: true,
      shutdown: true,
    };
    var cls = known[p] ? p : "idle";
    currentPhase = cls;
    phaseBadge.className = "phase-badge phase-" + cls;
    phaseBadge.textContent = p;
  }

  function setControlStatus(text, kind) {
    if (!ctrlStatus) return;
    ctrlStatus.textContent = text || "";
    ctrlStatus.className = "control-status" + (kind ? " " + kind : "");
  }

  function displayTargetFromSnap(snap) {
    if (lastTarget != null) return lastTarget;
    if (!snap) return null;
    if (typeof snap.maximum_users === "number" && isFinite(snap.maximum_users)) {
      return snap.maximum_users;
    }
    if (typeof snap.active_users === "number" && isFinite(snap.active_users)) {
      return snap.active_users;
    }
    return null;
  }

  function readStep() {
    var n = parseInt(ctrlStep && ctrlStep.value, 10);
    if (!isFinite(n) || n < 1) n = 10;
    step = n;
    return step;
  }

  function updateControlEnablement() {
    if (!controlPanel || !controlEnabled) return;

    var hasToken = !!token;
    var phase = currentPhase || "idle";
    var canStart = hasToken && !inFlight && phase === "idle";
    var canStop =
      hasToken && !inFlight && (phase === "increase" || phase === "maintain");
    var canUsers =
      hasToken &&
      !inFlight &&
      (phase === "idle" ||
        phase === "increase" ||
        phase === "maintain" ||
        phase === "decrease");

    if (ctrlStart) ctrlStart.disabled = !canStart;
    if (ctrlStop) ctrlStop.disabled = !canStop;
    if (ctrlApply) ctrlApply.disabled = !canUsers;
    if (ctrlMinus) ctrlMinus.disabled = !canUsers;
    if (ctrlPlus) ctrlPlus.disabled = !canUsers;
    if (ctrlTarget) ctrlTarget.disabled = !hasToken || inFlight;
    if (ctrlStep) ctrlStep.disabled = !hasToken || inFlight;

    if (!hasToken) {
      controlPanel.classList.add("disabled");
    } else {
      controlPanel.classList.remove("disabled");
    }
  }

  function updateControlFromSnapshot(snap) {
    if (!controlEnabled || !controlPanel || !snap) return;
    lastSnap = snap;

    if (ctrlActive) {
      ctrlActive.textContent =
        typeof snap.active_users === "number"
          ? formatInt(snap.active_users)
          : "—";
    }

    if (!dirty && ctrlTarget) {
      var dt = displayTargetFromSnap(snap);
      if (dt != null) {
        ctrlTarget.value = String(dt);
        lastDisplayTarget = dt;
      }
    }

    updateControlEnablement();
  }

  // Control POSTs: Bearer only — never append ?token= (do not use withToken).
  function postControl(path, body) {
    var headers = { "Content-Type": "application/json" };
    if (token) {
      headers["Authorization"] = "Bearer " + token;
    }
    return fetch(path, {
      method: "POST",
      headers: headers,
      body: body === undefined ? "{}" : JSON.stringify(body),
    });
  }

  function handleControlResponse(res, appliedUsers) {
    if (res.status === 401) {
      setBanner(
        "Open this dashboard as http://host:port/?token=… (token required for control).",
        "error"
      );
      setControlStatus("Unauthorized — reopen with ?token=", "error");
      return Promise.resolve(null);
    }
    if (res.status === 503) {
      setControlStatus("Control unavailable — retry", "error");
      return Promise.resolve(null);
    }
    return res.json().then(
      function (data) {
        if (!data) {
          setControlStatus("Unexpected control response", "error");
          return null;
        }
        if (data.ok) {
          setControlStatus(data.message || "OK", "ok");
          if (data.phase) {
            setPhase(data.phase);
          }
          if (
            typeof appliedUsers === "number" &&
            isFinite(appliedUsers) &&
            appliedUsers >= 1
          ) {
            lastTarget = appliedUsers;
            dirty = false;
            if (ctrlTarget) {
              ctrlTarget.value = String(appliedUsers);
              lastDisplayTarget = appliedUsers;
            }
          } else if (
            data.target_users != null &&
            typeof data.target_users === "number" &&
            data.target_users >= 1
          ) {
            lastTarget = data.target_users;
            dirty = false;
            if (ctrlTarget) {
              ctrlTarget.value = String(data.target_users);
              lastDisplayTarget = data.target_users;
            }
          }
          updateControlEnablement();
          return data;
        }
        setControlStatus(data.message || "Control rejected", "error");
        if (data.phase) {
          setPhase(data.phase);
        }
        updateControlEnablement();
        return data;
      },
      function () {
        setControlStatus("HTTP " + res.status, "error");
        return null;
      }
    );
  }

  function runControl(path, body, appliedUsers) {
    if (inFlight || !token) return;
    inFlight = true;
    updateControlEnablement();
    setControlStatus("Sending…", "info");
    postControl(path, body)
      .then(function (res) {
        return handleControlResponse(res, appliedUsers);
      })
      .catch(function (err) {
        setControlStatus("Request failed: " + err, "error");
      })
      .then(function () {
        inFlight = false;
        updateControlEnablement();
      });
  }

  function onStartClick() {
    runControl("/api/v1/control/start");
  }

  function onStopClick() {
    runControl("/api/v1/control/stop");
  }

  function onApplyClick() {
    var n = parseInt(ctrlTarget && ctrlTarget.value, 10);
    if (!isFinite(n) || n < 1) {
      setControlStatus("Target must be an integer ≥ 1", "error");
      return;
    }
    runControl("/api/v1/control/users", { users: n }, n);
  }

  function onStepClick(delta) {
    readStep();
    var base;
    if (dirty) {
      base = parseInt(ctrlTarget && ctrlTarget.value, 10);
    } else {
      base = displayTargetFromSnap(lastSnap);
    }
    if (!isFinite(base)) {
      base =
        lastSnap && typeof lastSnap.active_users === "number"
          ? lastSnap.active_users
          : 1;
    }
    var next = Math.max(1, base + delta * step);
    if (ctrlTarget) ctrlTarget.value = String(next);
    dirty = true;
    runControl("/api/v1/control/users", { users: next }, next);
  }

  function applyControlChrome() {
    if (subtitleEl) {
      subtitleEl.textContent = controlEnabled
        ? "Live dashboard · control"
        : "Live dashboard";
    }
    if (footerNoteEl) {
      footerNoteEl.textContent = controlEnabled
        ? "Start, stop, and adjust users from this panel (authenticated). Stop begins a cancel ramp (decrease) before idle. Advanced control remains on Controllers."
        : "Control this test via telnet :5116 or WebSocket :5117. This dashboard is read-only.";
    }
  }

  function initControlPanel() {
    if (!controlPanel) return;

    fetch("/api/v1/health")
      .then(function (res) {
        if (!res.ok) throw new Error("HTTP " + res.status);
        return res.json();
      })
      .then(function (health) {
        controlEnabled = !!(health && health.control_enabled);
        applyControlChrome();
        if (!controlEnabled) {
          controlPanel.classList.add("hidden");
          return;
        }
        controlPanel.classList.remove("hidden");
        if (!token) {
          controlTokenMissing = true;
          setBanner(
            "Open this dashboard as http://host:port/?token=… (token required for control).",
            "error"
          );
        }
        updateControlEnablement();
      })
      .catch(function () {
        // Health probe failed — leave panel hidden (observe-only fallback).
        controlEnabled = false;
        applyControlChrome();
        controlPanel.classList.add("hidden");
      });

    if (ctrlStart) ctrlStart.addEventListener("click", onStartClick);
    if (ctrlStop) ctrlStop.addEventListener("click", onStopClick);
    if (ctrlApply) ctrlApply.addEventListener("click", onApplyClick);
    if (ctrlMinus)
      ctrlMinus.addEventListener("click", function () {
        onStepClick(-1);
      });
    if (ctrlPlus)
      ctrlPlus.addEventListener("click", function () {
        onStepClick(1);
      });
    if (ctrlStep) {
      ctrlStep.addEventListener("change", readStep);
      ctrlStep.addEventListener("input", readStep);
    }
    if (ctrlTarget) {
      ctrlTarget.addEventListener("focus", function () {
        dirty = true;
      });
      ctrlTarget.addEventListener("input", function () {
        dirty = true;
      });
      ctrlTarget.addEventListener("blur", function () {
        if (!dirty) return;
        var n = parseInt(ctrlTarget.value, 10);
        if (
          lastDisplayTarget != null &&
          isFinite(n) &&
          n === lastDisplayTarget
        ) {
          dirty = false;
        }
      });
    }
  }

  function textCell(value) {
    var td = document.createElement("td");
    td.textContent = value == null ? "" : String(value);
    return td;
  }

  function kv(label, value) {
    var wrap = document.createElement("div");
    wrap.className = "kv";
    var k = document.createElement("span");
    k.className = "k";
    k.textContent = label;
    var v = document.createElement("span");
    v.className = "v";
    v.textContent = value == null ? "—" : String(value);
    wrap.appendChild(k);
    wrap.appendChild(v);
    return wrap;
  }

  function formatInt(n) {
    if (typeof n !== "number" || !isFinite(n)) return "—";
    return Math.round(n).toLocaleString();
  }

  function formatRate(n) {
    if (typeof n !== "number" || !isFinite(n)) return "—";
    return n.toLocaleString(undefined, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });
  }

  function formatPct(n) {
    if (typeof n !== "number" || !isFinite(n)) return "—";
    return (
      (n * 100).toLocaleString(undefined, {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      }) + "%"
    );
  }

  function formatDuration(secs) {
    if (typeof secs !== "number" || !isFinite(secs)) return "—";
    secs = Math.max(0, Math.floor(secs));
    var h = Math.floor(secs / 3600);
    var m = Math.floor((secs % 3600) / 60);
    var s = secs % 60;
    if (h > 0) return h + "h " + m + "m " + s + "s";
    if (m > 0) return m + "m " + s + "s";
    return s + "s";
  }

  function chartColors() {
    var dark =
      window.matchMedia &&
      window.matchMedia("(prefers-color-scheme: dark)").matches;
    // If no preference API, assume dark (default CSS is dark).
    if (
      window.matchMedia &&
      !window.matchMedia("(prefers-color-scheme: light)").matches &&
      !window.matchMedia("(prefers-color-scheme: dark)").matches
    ) {
      dark = true;
    }
    return {
      rps: dark ? "#3d8bfd" : "#0b5fff",
      fps: dark ? "#f07178" : "#b42318",
      users: dark ? "#3dd68c" : "#0a7a43",
      latency: dark ? "#e6b450" : "#9a6700",
      grid: dark ? "rgba(154, 167, 184, 0.2)" : "rgba(91, 107, 124, 0.2)",
      tick: dark ? "#9aa7b8" : "#5b6b7c",
    };
  }

  function baseChartOpts(colors) {
    // Fresh object per chart so Chart.js cannot share/mutate nested scales.
    return {
      responsive: true,
      maintainAspectRatio: false,
      animation: false,
      interaction: { mode: "index", intersect: false },
      plugins: {
        legend: {
          labels: { color: colors.tick, boxWidth: 12, font: { size: 11 } },
        },
      },
      scales: {
        x: {
          ticks: {
            color: colors.tick,
            maxTicksLimit: 8,
            font: { size: 10 },
          },
          grid: { color: colors.grid },
        },
        y: {
          beginAtZero: true,
          ticks: { color: colors.tick, font: { size: 10 } },
          grid: { color: colors.grid },
        },
      },
    };
  }

  function ensureCharts() {
    if (!chartsReady) return;
    var colors = chartColors();

    if (!chartRps) {
      chartRps = new Chart(document.getElementById("chart-rps"), {
        type: "line",
        data: {
          labels: [],
          datasets: [
            {
              label: "RPS",
              data: [],
              borderColor: colors.rps,
              backgroundColor: colors.rps,
              borderWidth: 1.5,
              pointRadius: 0,
              tension: 0.15,
            },
            {
              label: "Failures/s",
              data: [],
              borderColor: colors.fps,
              backgroundColor: colors.fps,
              borderWidth: 1.5,
              pointRadius: 0,
              tension: 0.15,
            },
          ],
        },
        options: baseChartOpts(colors),
      });
    }
    if (!chartUsers) {
      chartUsers = new Chart(document.getElementById("chart-users"), {
        type: "line",
        data: {
          labels: [],
          datasets: [
            {
              label: "Users",
              data: [],
              borderColor: colors.users,
              backgroundColor: colors.users,
              borderWidth: 1.5,
              pointRadius: 0,
              tension: 0.15,
              fill: false,
            },
          ],
        },
        options: baseChartOpts(colors),
      });
    }
    if (!chartLatency) {
      chartLatency = new Chart(document.getElementById("chart-latency"), {
        type: "line",
        data: {
          labels: [],
          datasets: [
            {
              label: "Avg ms",
              data: [],
              borderColor: colors.latency,
              backgroundColor: colors.latency,
              borderWidth: 1.5,
              pointRadius: 0,
              tension: 0.15,
            },
          ],
        },
        options: baseChartOpts(colors),
      });
    }
  }

  function seriesHasData(series) {
    if (!series) return false;
    var rps = series.rps || [];
    var fps = series.fps || [];
    var users = series.users || [];
    var lat = series.avg_latency_ms || [];
    return (
      rps.length > 0 || fps.length > 0 || users.length > 0 || lat.length > 0
    );
  }

  function updateCharts(series) {
    series = series || {};
    if (!chartsReady) {
      // One-shot warn when series data is present but Chart.js never loaded.
      if (!chartLoadWarned && seriesHasData(series)) {
        chartLoadWarned = true;
        setBanner(
          "Charts unavailable — Chart.js failed to load.",
          "warn"
        );
      }
      return;
    }
    ensureCharts();
    var start = typeof series.start_second === "number" ? series.start_second : 0;
    var rps = series.rps || [];
    var fps = series.fps || [];
    var users = series.users || [];
    var lat = series.avg_latency_ms || [];
    var len = Math.max(rps.length, fps.length, users.length, lat.length);
    var labels = [];
    var i;
    for (i = 0; i < len; i++) {
      labels.push(String(start + i));
    }

    if (chartRps) {
      chartRps.data.labels = labels;
      chartRps.data.datasets[0].data = rps;
      chartRps.data.datasets[1].data = fps;
      chartRps.update("none");
    }
    if (chartUsers) {
      chartUsers.data.labels = labels;
      chartUsers.data.datasets[0].data = users;
      chartUsers.update("none");
    }
    if (chartLatency) {
      chartLatency.data.labels = labels;
      chartLatency.data.datasets[0].data = lat;
      chartLatency.update("none");
    }
  }

  function sortRows(rows, sort) {
    var key = sort.key;
    var type = sort.type;
    var dir = sort.dir === "asc" ? 1 : -1;
    var copy = rows.slice();
    copy.sort(function (a, b) {
      var av = a[key];
      var bv = b[key];
      if (type === "num") {
        av = typeof av === "number" ? av : 0;
        bv = typeof bv === "number" ? bv : 0;
        return (av - bv) * dir;
      }
      av = av == null ? "" : String(av);
      bv = bv == null ? "" : String(bv);
      if (av < bv) return -1 * dir;
      if (av > bv) return 1 * dir;
      return 0;
    });
    return copy;
  }

  function flattenRequest(r) {
    var p = r.percentile_ms || {};
    return {
      method: r.method,
      name: r.name,
      request_count: r.request_count,
      failure_count: r.failure_count,
      requests_per_second: r.requests_per_second,
      response_time_avg_ms: r.response_time_avg_ms,
      p50: p.p50,
      p95: p.p95,
      p99: p.p99,
    };
  }

  function renderRequestTable(flags) {
    flags = flags || lastFlags || {};
    var filter = (requestsFilter.value || "").toLowerCase().trim();
    var rows = sortRows(requestRows, reqSort);
    if (filter) {
      rows = rows.filter(function (r) {
        return (
          String(r.method).toLowerCase().indexOf(filter) >= 0 ||
          String(r.name).toLowerCase().indexOf(filter) >= 0
        );
      });
    }

    requestsBody.textContent = "";
    if (requestRows.length === 0) {
      var empty = document.createElement("tr");
      var td = document.createElement("td");
      td.colSpan = 9;
      td.className = "empty";
      if (flags.metrics_disabled) {
        td.textContent =
          "Metrics disabled (--no-metrics). Charts and request tables are empty.";
      } else {
        td.textContent = "No requests yet";
      }
      empty.appendChild(td);
      requestsBody.appendChild(empty);
      return;
    }

    if (rows.length === 0) {
      var noMatch = document.createElement("tr");
      var ntd = document.createElement("td");
      ntd.colSpan = 9;
      ntd.className = "empty";
      ntd.textContent = "No matching requests";
      noMatch.appendChild(ntd);
      requestsBody.appendChild(noMatch);
      return;
    }

    for (var i = 0; i < rows.length; i++) {
      var r = rows[i];
      var tr = document.createElement("tr");
      tr.appendChild(textCell(r.method));
      tr.appendChild(textCell(r.name));
      tr.appendChild(textCell(formatInt(r.request_count)));
      tr.appendChild(textCell(formatInt(r.failure_count)));
      tr.appendChild(textCell(formatRate(r.requests_per_second)));
      tr.appendChild(textCell(formatRate(r.response_time_avg_ms)));
      tr.appendChild(textCell(formatInt(r.p50)));
      tr.appendChild(textCell(formatInt(r.p95)));
      tr.appendChild(textCell(formatInt(r.p99)));
      requestsBody.appendChild(tr);
    }
  }

  function renderErrorTable() {
    var rows = sortRows(errorRows, errSort);
    errorsBody.textContent = "";
    if (errorRows.length === 0) {
      var er = document.createElement("tr");
      var et = document.createElement("td");
      et.colSpan = 4;
      et.className = "empty";
      et.textContent = "No errors";
      er.appendChild(et);
      errorsBody.appendChild(er);
      return;
    }
    for (var j = 0; j < rows.length; j++) {
      var e = rows[j];
      var etr = document.createElement("tr");
      etr.appendChild(textCell(e.method));
      etr.appendChild(textCell(e.name));
      etr.appendChild(textCell(e.error));
      etr.appendChild(textCell(formatInt(e.occurrences)));
      errorsBody.appendChild(etr);
    }
  }

  function wireSort(tableId, getSort, setSort, rerender) {
    var table = document.getElementById(tableId);
    if (!table) return;
    var ths = table.querySelectorAll("thead th[data-sort]");
    for (var i = 0; i < ths.length; i++) {
      (function (th) {
        th.addEventListener("click", function () {
          var key = th.getAttribute("data-sort");
          var type = th.getAttribute("data-type") || "str";
          var sort = getSort();
          if (sort.key === key) {
            sort.dir = sort.dir === "asc" ? "desc" : "asc";
          } else {
            sort.key = key;
            sort.type = type;
            sort.dir = type === "num" ? "desc" : "asc";
          }
          setSort(sort);
          // Update header classes
          for (var j = 0; j < ths.length; j++) {
            ths[j].classList.remove("sorted", "asc", "desc");
          }
          th.classList.add("sorted", sort.dir);
          rerender();
        });
      })(ths[i]);
    }
  }

  function renderSnapshot(snap, modeLabel) {
    if (!snap) return;

    var flags = snap.flags || {};
    lastFlags = flags;
    setPhase(snap.phase);
    hostsEl.textContent =
      snap.hosts && snap.hosts.length ? snap.hosts.join(", ") : "—";
    durationEl.textContent = formatDuration(snap.duration_secs);

    kpiUsers.textContent =
      formatInt(snap.active_users) + " / " + formatInt(snap.maximum_users);
    var agg = snap.aggregate || {};
    kpiRps.textContent = formatRate(agg.requests_per_second);
    kpiFail.textContent = formatPct(agg.failure_rate);
    var p = agg.percentile_ms || {};
    kpiP95.textContent = formatInt(p.p95);
    kpiAvg.textContent = formatRate(agg.response_time_avg_ms);

    summaryEl.textContent = "";
    summaryEl.appendChild(kv("Phase", snap.phase));
    summaryEl.appendChild(kv("Duration", formatDuration(snap.duration_secs)));
    summaryEl.appendChild(
      kv(
        "Users",
        formatInt(snap.active_users) + " / " + formatInt(snap.maximum_users)
      )
    );
    summaryEl.appendChild(kv("Total users", formatInt(snap.total_users)));
    summaryEl.appendChild(kv("Goose", snap.goose_version));
    summaryEl.appendChild(
      kv(
        "Hosts",
        snap.hosts && snap.hosts.length ? snap.hosts.join(", ") : "—"
      )
    );
    if (flags.series_seconds) {
      summaryEl.appendChild(
        kv("Series window", formatInt(flags.series_seconds) + "s")
      );
    }

    aggregateEl.textContent = "";
    aggregateEl.appendChild(kv("Requests", formatInt(agg.total_requests)));
    aggregateEl.appendChild(kv("Failures", formatInt(agg.total_failures)));
    aggregateEl.appendChild(kv("RPS", formatRate(agg.requests_per_second)));
    aggregateEl.appendChild(kv("Fail %", formatPct(agg.failure_rate)));
    aggregateEl.appendChild(kv("Avg ms", formatRate(agg.response_time_avg_ms)));
    aggregateEl.appendChild(
      kv(
        "p50 / p95 / p99",
        formatInt(p.p50) + " / " + formatInt(p.p95) + " / " + formatInt(p.p99)
      )
    );
    if (agg.co_active) {
      aggregateEl.appendChild(kv("Coordinated omission", "active"));
    }

    requestRows = (snap.requests || []).map(flattenRequest);
    errorRows = (snap.errors || []).slice();
    renderRequestTable(flags);
    renderErrorTable();
    updateCharts(snap.series);
    updateControlFromSnapshot(snap);

    // Prefer metrics/auth banners; chart-load warning is sticky only when no other banner.
    // Keep the control-token-missing banner sticky while control is on without a token.
    if (flags.metrics_disabled) {
      setBanner(
        "Metrics are disabled (--no-metrics). The dashboard shell is live, but request/series data is empty.",
        "warn"
      );
    } else if (flags.requests_truncated || flags.errors_truncated) {
      var parts = [];
      if (flags.requests_truncated) parts.push("request rows truncated");
      if (flags.errors_truncated) parts.push("error rows truncated");
      setBanner(parts.join("; ") + " (showing top rows only).", "info");
    } else if (controlTokenMissing) {
      setBanner(
        "Open this dashboard as http://host:port/?token=… (token required for control).",
        "error"
      );
    } else if (authRequired) {
      // Clear previous auth banner once we have data.
      setBanner("");
      authRequired = false;
    } else if (chartLoadWarned && !chartsReady) {
      setBanner(
        "Charts unavailable — Chart.js failed to load.",
        "warn"
      );
    } else {
      setBanner("");
    }

    if (modeLabel === "live") {
      setConnection("live");
    } else if (modeLabel === "poll") {
      setConnection("poll");
    }
  }

  function withToken(path) {
    if (!token) return path;
    var sep = path.indexOf("?") >= 0 ? "&" : "?";
    return path + sep + "token=" + encodeURIComponent(token);
  }

  function snapshotUrl() {
    return withToken("/api/v1/snapshot");
  }

  function eventsUrl() {
    return withToken("/api/v1/events");
  }

  function stopPoll() {
    if (pollTimer != null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  function showAuthMissing() {
    authRequired = true;
    authBlocked = true;
    stopPoll();
    usingPoll = false;
    if (eventSource) {
      try {
        eventSource.close();
      } catch (_) {
        /* ignore */
      }
      eventSource = null;
    }
    setConnection("disconnected");
    setBanner(
      "Open this dashboard as http://host:port/?token=… (token required for metrics).",
      "error"
    );
  }

  function fetchSnapshotOnce(modeLabel) {
    if (authBlocked) return Promise.resolve(null);
    return fetch(snapshotUrl())
      .then(function (res) {
        if (res.status === 401) {
          showAuthMissing();
          return null;
        }
        if (!res.ok) {
          throw new Error("HTTP " + res.status);
        }
        return res.json();
      })
      .then(function (snap) {
        if (snap) {
          renderSnapshot(snap, modeLabel || "poll");
        }
        return snap;
      });
  }

  function startPollFallback(reason) {
    if (finished || authBlocked) return;
    if (usingPoll && pollTimer != null) return;
    usingPoll = true;
    if (eventSource) {
      try {
        eventSource.close();
      } catch (_) {
        /* ignore */
      }
      eventSource = null;
    }
    setConnection("poll");
    if (reason && !authRequired) {
      setBanner(reason + " — polling every 2s…", "warn");
    }
    function tick() {
      if (authBlocked) {
        stopPoll();
        return;
      }
      fetchSnapshotOnce("poll").catch(function (err) {
        if (authBlocked) return;
        setConnection("disconnected");
        setBanner("Poll failed: " + err, "error");
      });
    }
    tick();
    stopPoll();
    if (!authBlocked) {
      pollTimer = setInterval(tick, 2000);
    }
  }

  function startSse() {
    if (typeof EventSource === "undefined") {
      startPollFallback("SSE unavailable");
      return;
    }

    setConnection("connecting");
    try {
      eventSource = new EventSource(eventsUrl());
    } catch (err) {
      startPollFallback("SSE open failed");
      return;
    }

    var sawSnapshot = false;
    // After the first snapshot, EventSource auto-reconnects; if errors keep
    // stacking without a fresh snapshot, fall back to poll so the UI recovers.
    var sseErrorStreak = 0;
    var SSE_ERROR_FALLBACK_THRESHOLD = 3;

    eventSource.addEventListener("snapshot", function (ev) {
      try {
        var snap = JSON.parse(ev.data);
        sawSnapshot = true;
        sseErrorStreak = 0;
        usingPoll = false;
        stopPoll();
        renderSnapshot(snap, "live");
      } catch (err) {
        setBanner("Bad snapshot event: " + err, "error");
      }
    });

    eventSource.addEventListener("closed", function () {
      finished = true;
      try {
        eventSource.close();
      } catch (_) {
        /* ignore */
      }
      eventSource = null;
      stopPoll();
      setConnection("closed");
      setBanner("Load test finished.", "info");
    });

    eventSource.onerror = function () {
      if (finished) {
        return;
      }
      // EventSource reconnects automatically on transient errors; fall back to
      // poll when we never received a snapshot, or after repeated errors once
      // live (server gone, 503 cap, sticky-closed without closed event).
      if (!sawSnapshot) {
        try {
          eventSource.close();
        } catch (_) {
          /* ignore */
        }
        eventSource = null;
        // Probe once to distinguish auth failure from other SSE issues.
        fetch(snapshotUrl())
          .then(function (res) {
            if (res.status === 401) {
              showAuthMissing();
              return;
            }
            startPollFallback("SSE failed");
          })
          .catch(function () {
            startPollFallback("SSE failed");
          });
      } else {
        sseErrorStreak += 1;
        setConnection("disconnected");
        if (sseErrorStreak >= SSE_ERROR_FALLBACK_THRESHOLD) {
          try {
            eventSource.close();
          } catch (_) {
            /* ignore */
          }
          eventSource = null;
          startPollFallback("SSE reconnect failed");
        }
      }
    };
  }

  // Wire UI controls
  wireSort(
    "requests-table",
    function () {
      return reqSort;
    },
    function (s) {
      reqSort = s;
    },
    function () {
      renderRequestTable(lastFlags);
    }
  );
  wireSort(
    "errors-table",
    function () {
      return errSort;
    },
    function (s) {
      errSort = s;
    },
    function () {
      renderErrorTable();
    }
  );
  if (requestsFilter) {
    requestsFilter.addEventListener("input", function () {
      renderRequestTable(lastFlags);
    });
  }

  ensureCharts();
  initControlPanel();
  startSse();
})();
