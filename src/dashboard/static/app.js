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

  var pollTimer = null;
  var eventSource = null;
  var usingPoll = false;
  var authRequired = false;
  var finished = false;

  // Table state
  var requestRows = [];
  var errorRows = [];
  var reqSort = { key: "request_count", type: "num", dir: "desc" };
  var errSort = { key: "occurrences", type: "num", dir: "desc" };

  // Charts
  var chartRps = null;
  var chartUsers = null;
  var chartLatency = null;
  var chartsReady = typeof Chart !== "undefined";

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
      cls = "live";
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
    phaseBadge.className = "phase-badge phase-" + cls;
    phaseBadge.textContent = p;
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

  function ensureCharts() {
    if (!chartsReady) return;
    var colors = chartColors();
    var commonOpts = {
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
        options: commonOpts,
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
        options: commonOpts,
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
        options: commonOpts,
      });
    }
  }

  function updateCharts(series) {
    if (!chartsReady) return;
    ensureCharts();
    series = series || {};
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
      if (flags && flags.metrics_disabled) {
        td.textContent = "Metrics disabled (--no-metrics). Charts and request tables are empty.";
      } else {
        td.textContent = "No requests yet";
      }
      empty.appendChild(td);
      requestsBody.appendChild(empty);
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
    } else if (authRequired) {
      // Clear previous auth banner once we have data.
      setBanner("");
      authRequired = false;
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
    setConnection("disconnected");
    setBanner(
      "Open this dashboard as http://host:port/?token=… (token required for metrics).",
      "error"
    );
  }

  function fetchSnapshotOnce(modeLabel) {
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
    if (finished) return;
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
      fetchSnapshotOnce("poll").catch(function (err) {
        setConnection("disconnected");
        setBanner("Poll failed: " + err, "error");
      });
    }
    tick();
    stopPoll();
    pollTimer = setInterval(tick, 2000);
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

    eventSource.addEventListener("snapshot", function (ev) {
      try {
        var snap = JSON.parse(ev.data);
        sawSnapshot = true;
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
      // EventSource reconnects automatically on transient errors; only fall
      // back after we never received a snapshot (auth failure, 503, etc.).
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
      } else if (!finished) {
        setConnection("disconnected");
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
      renderRequestTable({});
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
      renderRequestTable({});
    });
  }

  ensureCharts();
  startSse();
})();
