// Goose live dashboard — SSE stream with poll fallback.
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

  var statusEl = document.getElementById("status");
  var summaryEl = document.getElementById("summary-body");
  var aggregateEl = document.getElementById("aggregate-body");
  var requestsBody = document.getElementById("requests-body");
  var errorsBody = document.getElementById("errors-body");

  var pollTimer = null;
  var eventSource = null;
  var usingPoll = false;

  function setStatus(text, cls) {
    statusEl.textContent = text;
    statusEl.className = "status" + (cls ? " " + cls : "");
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

  function formatRate(n) {
    if (typeof n !== "number" || !isFinite(n)) return "—";
    return n.toFixed(2);
  }

  function formatPct(n) {
    if (typeof n !== "number" || !isFinite(n)) return "—";
    return (n * 100).toFixed(2) + "%";
  }

  function renderSnapshot(snap, modeLabel) {
    summaryEl.textContent = "";
    summaryEl.appendChild(kv("Phase", snap.phase));
    summaryEl.appendChild(kv("Duration (s)", snap.duration_secs));
    summaryEl.appendChild(
      kv("Users", snap.active_users + " / " + snap.maximum_users)
    );
    summaryEl.appendChild(kv("Total users", snap.total_users));
    summaryEl.appendChild(kv("Goose", snap.goose_version));
    summaryEl.appendChild(
      kv("Hosts", (snap.hosts && snap.hosts.length) ? snap.hosts.join(", ") : "—")
    );

    var agg = snap.aggregate || {};
    aggregateEl.textContent = "";
    aggregateEl.appendChild(kv("Requests", agg.total_requests));
    aggregateEl.appendChild(kv("Failures", agg.total_failures));
    aggregateEl.appendChild(kv("RPS", formatRate(agg.requests_per_second)));
    aggregateEl.appendChild(kv("Fail %", formatPct(agg.failure_rate)));
    aggregateEl.appendChild(kv("Avg ms", formatRate(agg.response_time_avg_ms)));
    var p = agg.percentile_ms || {};
    aggregateEl.appendChild(kv("p50 / p95 / p99", p.p50 + " / " + p.p95 + " / " + p.p99));

    requestsBody.textContent = "";
    var rows = snap.requests || [];
    if (rows.length === 0) {
      var empty = document.createElement("tr");
      var td = document.createElement("td");
      td.colSpan = 9;
      td.textContent = snap.flags && snap.flags.metrics_disabled
        ? "Metrics disabled"
        : "No requests yet";
      empty.appendChild(td);
      requestsBody.appendChild(empty);
    } else {
      for (var i = 0; i < rows.length; i++) {
        var r = rows[i];
        var tr = document.createElement("tr");
        tr.appendChild(textCell(r.method));
        tr.appendChild(textCell(r.name));
        tr.appendChild(textCell(r.request_count));
        tr.appendChild(textCell(r.failure_count));
        tr.appendChild(textCell(formatRate(r.requests_per_second)));
        tr.appendChild(textCell(formatRate(r.response_time_avg_ms)));
        var rp = r.percentile_ms || {};
        tr.appendChild(textCell(rp.p50));
        tr.appendChild(textCell(rp.p95));
        tr.appendChild(textCell(rp.p99));
        requestsBody.appendChild(tr);
      }
    }

    errorsBody.textContent = "";
    var errs = snap.errors || [];
    if (errs.length === 0) {
      var er = document.createElement("tr");
      var et = document.createElement("td");
      et.colSpan = 4;
      et.textContent = "No errors";
      er.appendChild(et);
      errorsBody.appendChild(er);
    } else {
      for (var j = 0; j < errs.length; j++) {
        var e = errs[j];
        var etr = document.createElement("tr");
        etr.appendChild(textCell(e.method));
        etr.appendChild(textCell(e.name));
        etr.appendChild(textCell(e.error));
        etr.appendChild(textCell(e.occurrences));
        errorsBody.appendChild(etr);
      }
    }

    var mode = modeLabel || "live";
    setStatus(mode + " · phase " + snap.phase, "ok");
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

  function fetchSnapshotOnce(modeLabel) {
    return fetch(snapshotUrl())
      .then(function (res) {
        if (res.status === 401) {
          setStatus(
            "Open this dashboard as http://host:port/?token=… (token required for metrics).",
            "error"
          );
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
    if (usingPoll) return;
    usingPoll = true;
    if (eventSource) {
      try {
        eventSource.close();
      } catch (_) {
        /* ignore */
      }
      eventSource = null;
    }
    setStatus(
      (reason ? reason + " — " : "") + "polling every 2s…",
      "error"
    );
    function tick() {
      fetchSnapshotOnce("poll").catch(function (err) {
        setStatus("Poll failed: " + err, "error");
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

    setStatus("Connecting…");
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
        setStatus("Bad snapshot event: " + err, "error");
      }
    });

    eventSource.addEventListener("closed", function () {
      try {
        eventSource.close();
      } catch (_) {
        /* ignore */
      }
      eventSource = null;
      stopPoll();
      setStatus("Load test finished", "ok");
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
        startPollFallback("SSE failed");
      }
    };
  }

  startSse();
})();
