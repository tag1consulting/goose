"use strict";
// Goose live dashboard — SSE stream with poll fallback, charts, UX polish.
// Source of truth: compile with `npm run build` in this directory to regenerate app.js.
(function () {
    "use strict";
    // ---------------------------------------------------------------------------
    // Auth bootstrap: read ?token= from the page URL, then strip it from the bar.
    // ---------------------------------------------------------------------------
    const params = new URLSearchParams(window.location.search);
    let token = params.get("token") || "";
    if (token) {
        params.delete("token");
        const clean = window.location.pathname +
            (params.toString() ? "?" + params.toString() : "") +
            window.location.hash;
        try {
            window.history.replaceState({}, "", clean);
        }
        catch (_a) {
            /* ignore */
        }
    }
    // ---------------------------------------------------------------------------
    // DOM refs
    // ---------------------------------------------------------------------------
    const bannerEl = document.getElementById("banner");
    const phaseBadge = document.getElementById("phase-badge");
    const hostsEl = document.getElementById("hosts");
    const durationEl = document.getElementById("duration");
    const connectionEl = document.getElementById("connection");
    const connectionLabel = document.getElementById("connection-label");
    const summaryEl = document.getElementById("summary-body");
    const aggregateEl = document.getElementById("aggregate-body");
    const requestsBody = document.getElementById("requests-body");
    const errorsBody = document.getElementById("errors-body");
    const requestsFilter = document.getElementById("requests-filter");
    const kpiUsers = document.getElementById("kpi-users");
    const kpiRps = document.getElementById("kpi-rps");
    const kpiFail = document.getElementById("kpi-fail");
    const kpiP95 = document.getElementById("kpi-p95");
    const kpiAvg = document.getElementById("kpi-avg");
    const subtitleEl = document.getElementById("subtitle");
    const footerNoteEl = document.getElementById("footer-note");
    const controlPanel = document.getElementById("control-panel");
    const ctrlStart = document.getElementById("ctrl-start");
    const ctrlStop = document.getElementById("ctrl-stop");
    const ctrlActive = document.getElementById("ctrl-active");
    const ctrlTarget = document.getElementById("ctrl-target");
    const ctrlApply = document.getElementById("ctrl-apply");
    const ctrlMinus = document.getElementById("ctrl-minus");
    const ctrlPlus = document.getElementById("ctrl-plus");
    const ctrlStep = document.getElementById("ctrl-step");
    const ctrlStatus = document.getElementById("ctrl-status");
    // ---------------------------------------------------------------------------
    // Runtime state
    // ---------------------------------------------------------------------------
    let pollTimer = null;
    let eventSource = null;
    let usingPoll = false;
    let authRequired = false;
    let authBlocked = false;
    let finished = false;
    // Control panel state (normative lastTarget/dirty algorithm)
    let controlEnabled = false;
    let lastTarget = null;
    let dirty = false;
    let step = 10;
    let inFlight = false;
    let lastSnap = null;
    let lastDisplayTarget = null;
    let currentPhase = "idle";
    let controlTokenMissing = false;
    // Table state
    let requestRows = [];
    let errorRows = [];
    let lastFlags = {};
    let reqSort = { key: "request_count", type: "num", dir: "desc" };
    let errSort = { key: "occurrences", type: "num", dir: "desc" };
    // Charts
    let chartRps = null;
    let chartUsers = null;
    let chartLatency = null;
    const chartsReady = typeof Chart !== "undefined";
    let chartLoadWarned = false;
    // ---------------------------------------------------------------------------
    // UI helpers
    // ---------------------------------------------------------------------------
    function setConnection(mode) {
        let cls = "disconnected";
        let label = "disconnected";
        if (mode === "live") {
            cls = "live";
            label = "SSE";
        }
        else if (mode === "poll") {
            cls = "poll";
            label = "poll";
        }
        else if (mode === "connecting") {
            cls = "disconnected";
            label = "connecting";
        }
        else if (mode === "closed") {
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
        const p = (phase || "idle").toLowerCase();
        const known = {
            idle: true,
            increase: true,
            maintain: true,
            decrease: true,
            shutdown: true,
        };
        const cls = known[p] ? p : "idle";
        currentPhase = cls;
        phaseBadge.className = "phase-badge phase-" + cls;
        phaseBadge.textContent = p;
    }
    function setControlStatus(text, kind) {
        if (!ctrlStatus)
            return;
        ctrlStatus.textContent = text || "";
        ctrlStatus.className = "control-status" + (kind ? " " + kind : "");
    }
    function displayTargetFromSnap(snap) {
        if (lastTarget != null)
            return lastTarget;
        if (!snap)
            return null;
        if (typeof snap.maximum_users === "number" &&
            isFinite(snap.maximum_users)) {
            return snap.maximum_users;
        }
        if (typeof snap.active_users === "number" &&
            isFinite(snap.active_users)) {
            return snap.active_users;
        }
        return null;
    }
    function readStep() {
        let n = parseInt(ctrlStep && ctrlStep.value ? ctrlStep.value : "", 10);
        if (!isFinite(n) || n < 1)
            n = 10;
        step = n;
        return step;
    }
    function updateControlEnablement() {
        if (!controlPanel || !controlEnabled)
            return;
        const hasToken = !!token;
        const phase = currentPhase || "idle";
        const canStart = hasToken && !inFlight && phase === "idle";
        const canStop = hasToken && !inFlight && (phase === "increase" || phase === "maintain");
        const canUsers = hasToken &&
            !inFlight &&
            (phase === "idle" ||
                phase === "increase" ||
                phase === "maintain" ||
                phase === "decrease");
        if (ctrlStart)
            ctrlStart.disabled = !canStart;
        if (ctrlStop)
            ctrlStop.disabled = !canStop;
        if (ctrlApply)
            ctrlApply.disabled = !canUsers;
        if (ctrlMinus)
            ctrlMinus.disabled = !canUsers;
        if (ctrlPlus)
            ctrlPlus.disabled = !canUsers;
        if (ctrlTarget)
            ctrlTarget.disabled = !hasToken || inFlight;
        if (ctrlStep)
            ctrlStep.disabled = !hasToken || inFlight;
        if (!hasToken) {
            controlPanel.classList.add("disabled");
        }
        else {
            controlPanel.classList.remove("disabled");
        }
    }
    function updateControlFromSnapshot(snap) {
        if (!controlEnabled || !controlPanel || !snap)
            return;
        lastSnap = snap;
        if (ctrlActive) {
            ctrlActive.textContent =
                typeof snap.active_users === "number"
                    ? formatInt(snap.active_users)
                    : "—";
        }
        if (!dirty && ctrlTarget) {
            const dt = displayTargetFromSnap(snap);
            if (dt != null) {
                ctrlTarget.value = String(dt);
                lastDisplayTarget = dt;
            }
        }
        updateControlEnablement();
    }
    // Control POSTs: Bearer only — never append ?token= (do not use withToken).
    function postControl(path, body) {
        const headers = {
            "Content-Type": "application/json",
        };
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
            setBanner("Open this dashboard as http://host:port/?token=… (token required for control).", "error");
            setControlStatus("Unauthorized — reopen with ?token=", "error");
            return Promise.resolve(null);
        }
        if (res.status === 503) {
            // Prefer server message: timeout vs busy vs unavailable. Never blind-retry
            // on timeout — the action may still be applying on the load generator.
            return res.json().then((body) => {
                const err = body && body.error ? String(body.error) : "";
                const msg = body && body.message
                    ? String(body.message)
                    : "Control unavailable";
                if (err === "timeout") {
                    setControlStatus("Timed out — check phase/users before retrying", "error");
                }
                else if (err === "busy") {
                    setControlStatus("Control busy — wait and retry", "error");
                }
                else {
                    setControlStatus(msg, "error");
                }
                return null;
            }, () => {
                setControlStatus("Control unavailable", "error");
                return null;
            });
        }
        return res.json().then((data) => {
            if (!data) {
                setControlStatus("Unexpected control response", "error");
                return null;
            }
            if (data.ok) {
                setControlStatus(data.message || "OK", "ok");
                if (data.phase) {
                    setPhase(data.phase);
                }
                if (typeof appliedUsers === "number" &&
                    isFinite(appliedUsers) &&
                    appliedUsers >= 1) {
                    lastTarget = appliedUsers;
                    dirty = false;
                    if (ctrlTarget) {
                        ctrlTarget.value = String(appliedUsers);
                        lastDisplayTarget = appliedUsers;
                    }
                }
                else if (data.target_users != null &&
                    typeof data.target_users === "number" &&
                    data.target_users >= 1) {
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
        }, () => {
            setControlStatus("HTTP " + res.status, "error");
            return null;
        });
    }
    function runControl(path, body, appliedUsers) {
        if (inFlight || !token)
            return;
        inFlight = true;
        updateControlEnablement();
        setControlStatus("Sending…", "info");
        postControl(path, body)
            .then((res) => handleControlResponse(res, appliedUsers))
            .catch((err) => {
            setControlStatus("Request failed: " + String(err), "error");
        })
            .then(() => {
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
        const n = parseInt(ctrlTarget && ctrlTarget.value ? ctrlTarget.value : "", 10);
        if (!isFinite(n) || n < 1) {
            setControlStatus("Target must be an integer ≥ 1", "error");
            return;
        }
        runControl("/api/v1/control/users", { users: n }, n);
    }
    function onStepClick(delta) {
        readStep();
        let base;
        if (dirty) {
            base = parseInt(ctrlTarget && ctrlTarget.value ? ctrlTarget.value : "", 10);
        }
        else {
            const fromSnap = displayTargetFromSnap(lastSnap);
            base = fromSnap == null ? NaN : fromSnap;
        }
        if (!isFinite(base)) {
            base =
                lastSnap && typeof lastSnap.active_users === "number"
                    ? lastSnap.active_users
                    : 1;
        }
        const next = Math.max(1, base + delta * step);
        if (ctrlTarget)
            ctrlTarget.value = String(next);
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
        if (!controlPanel)
            return;
        fetch("/api/v1/health")
            .then((res) => {
            if (!res.ok)
                throw new Error("HTTP " + res.status);
            return res.json();
        })
            .then((health) => {
            controlEnabled = !!(health && health.control_enabled);
            applyControlChrome();
            if (!controlEnabled) {
                controlPanel.classList.add("hidden");
                return;
            }
            controlPanel.classList.remove("hidden");
            if (!token) {
                controlTokenMissing = true;
                setBanner("Open this dashboard as http://host:port/?token=… (token required for control).", "error");
            }
            updateControlEnablement();
        })
            .catch(() => {
            // Health probe failed — leave panel hidden (observe-only fallback).
            controlEnabled = false;
            applyControlChrome();
            controlPanel.classList.add("hidden");
        });
        if (ctrlStart)
            ctrlStart.addEventListener("click", onStartClick);
        if (ctrlStop)
            ctrlStop.addEventListener("click", onStopClick);
        if (ctrlApply)
            ctrlApply.addEventListener("click", onApplyClick);
        if (ctrlMinus)
            ctrlMinus.addEventListener("click", () => {
                onStepClick(-1);
            });
        if (ctrlPlus)
            ctrlPlus.addEventListener("click", () => {
                onStepClick(1);
            });
        if (ctrlStep) {
            ctrlStep.addEventListener("change", readStep);
            ctrlStep.addEventListener("input", readStep);
        }
        if (ctrlTarget) {
            ctrlTarget.addEventListener("focus", () => {
                dirty = true;
            });
            ctrlTarget.addEventListener("input", () => {
                dirty = true;
            });
            ctrlTarget.addEventListener("blur", () => {
                if (!dirty)
                    return;
                const n = parseInt(ctrlTarget.value, 10);
                if (lastDisplayTarget != null &&
                    isFinite(n) &&
                    n === lastDisplayTarget) {
                    dirty = false;
                }
            });
        }
    }
    function textCell(value) {
        const td = document.createElement("td");
        td.textContent = value == null ? "" : String(value);
        return td;
    }
    function kv(label, value) {
        const wrap = document.createElement("div");
        wrap.className = "kv";
        const k = document.createElement("span");
        k.className = "k";
        k.textContent = label;
        const v = document.createElement("span");
        v.className = "v";
        v.textContent = value == null ? "—" : String(value);
        wrap.appendChild(k);
        wrap.appendChild(v);
        return wrap;
    }
    function formatInt(n) {
        if (typeof n !== "number" || !isFinite(n))
            return "—";
        return Math.round(n).toLocaleString();
    }
    function formatRate(n) {
        if (typeof n !== "number" || !isFinite(n))
            return "—";
        return n.toLocaleString(undefined, {
            minimumFractionDigits: 2,
            maximumFractionDigits: 2,
        });
    }
    function formatPct(n) {
        if (typeof n !== "number" || !isFinite(n))
            return "—";
        return ((n * 100).toLocaleString(undefined, {
            minimumFractionDigits: 2,
            maximumFractionDigits: 2,
        }) + "%");
    }
    function formatDuration(secs) {
        if (typeof secs !== "number" || !isFinite(secs))
            return "—";
        let remaining = Math.max(0, Math.floor(secs));
        const h = Math.floor(remaining / 3600);
        const m = Math.floor((remaining % 3600) / 60);
        const s = remaining % 60;
        if (h > 0)
            return h + "h " + m + "m " + s + "s";
        if (m > 0)
            return m + "m " + s + "s";
        return s + "s";
    }
    function chartColors() {
        let dark = window.matchMedia &&
            window.matchMedia("(prefers-color-scheme: dark)").matches;
        // If no preference API, assume dark (default CSS is dark).
        if (window.matchMedia &&
            !window.matchMedia("(prefers-color-scheme: light)").matches &&
            !window.matchMedia("(prefers-color-scheme: dark)").matches) {
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
        if (!chartsReady || typeof Chart === "undefined")
            return;
        const colors = chartColors();
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
        if (!series)
            return false;
        const rps = series.rps || [];
        const fps = series.fps || [];
        const users = series.users || [];
        const lat = series.avg_latency_ms || [];
        return (rps.length > 0 || fps.length > 0 || users.length > 0 || lat.length > 0);
    }
    function updateCharts(series) {
        series = series || {};
        if (!chartsReady) {
            // One-shot warn when series data is present but Chart.js never loaded.
            if (!chartLoadWarned && seriesHasData(series)) {
                chartLoadWarned = true;
                setBanner("Charts unavailable — Chart.js failed to load.", "warn");
            }
            return;
        }
        ensureCharts();
        const start = typeof series.start_second === "number" ? series.start_second : 0;
        const rps = series.rps || [];
        const fps = series.fps || [];
        const users = series.users || [];
        const lat = series.avg_latency_ms || [];
        const len = Math.max(rps.length, fps.length, users.length, lat.length);
        const labels = [];
        for (let i = 0; i < len; i++) {
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
        const key = sort.key;
        const type = sort.type;
        const dir = sort.dir === "asc" ? 1 : -1;
        const copy = rows.slice();
        copy.sort((a, b) => {
            let av = a[key];
            let bv = b[key];
            if (type === "num") {
                const an = typeof av === "number" ? av : 0;
                const bn = typeof bv === "number" ? bv : 0;
                return (an - bn) * dir;
            }
            const as = av == null ? "" : String(av);
            const bs = bv == null ? "" : String(bv);
            if (as < bs)
                return -1 * dir;
            if (as > bs)
                return 1 * dir;
            return 0;
        });
        return copy;
    }
    function flattenRequest(r) {
        const p = r.percentile_ms || {};
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
        const filter = (requestsFilter && requestsFilter.value
            ? requestsFilter.value
            : "")
            .toLowerCase()
            .trim();
        let rows = sortRows(requestRows, reqSort);
        if (filter) {
            rows = rows.filter((r) => {
                return (String(r.method).toLowerCase().indexOf(filter) >= 0 ||
                    String(r.name).toLowerCase().indexOf(filter) >= 0);
            });
        }
        requestsBody.textContent = "";
        if (requestRows.length === 0) {
            const empty = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 9;
            td.className = "empty";
            if (flags.metrics_disabled) {
                td.textContent =
                    "Metrics disabled (--no-metrics). Charts and request tables are empty.";
            }
            else {
                td.textContent = "No requests yet";
            }
            empty.appendChild(td);
            requestsBody.appendChild(empty);
            return;
        }
        if (rows.length === 0) {
            const noMatch = document.createElement("tr");
            const ntd = document.createElement("td");
            ntd.colSpan = 9;
            ntd.className = "empty";
            ntd.textContent = "No matching requests";
            noMatch.appendChild(ntd);
            requestsBody.appendChild(noMatch);
            return;
        }
        for (let i = 0; i < rows.length; i++) {
            const r = rows[i];
            const tr = document.createElement("tr");
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
        const rows = sortRows(errorRows, errSort);
        errorsBody.textContent = "";
        if (errorRows.length === 0) {
            const er = document.createElement("tr");
            const et = document.createElement("td");
            et.colSpan = 4;
            et.className = "empty";
            et.textContent = "No errors";
            er.appendChild(et);
            errorsBody.appendChild(er);
            return;
        }
        for (let j = 0; j < rows.length; j++) {
            const e = rows[j];
            const etr = document.createElement("tr");
            etr.appendChild(textCell(e.method));
            etr.appendChild(textCell(e.name));
            etr.appendChild(textCell(e.error));
            etr.appendChild(textCell(formatInt(e.occurrences)));
            errorsBody.appendChild(etr);
        }
    }
    function wireSort(tableId, getSort, setSort, rerender) {
        const table = document.getElementById(tableId);
        if (!table)
            return;
        const ths = table.querySelectorAll("thead th[data-sort]");
        for (let i = 0; i < ths.length; i++) {
            const th = ths[i];
            th.addEventListener("click", () => {
                const key = th.getAttribute("data-sort");
                if (!key)
                    return;
                const typeAttr = th.getAttribute("data-type") || "str";
                const type = typeAttr === "num" ? "num" : "str";
                const sort = getSort();
                if (sort.key === key) {
                    sort.dir = sort.dir === "asc" ? "desc" : "asc";
                }
                else {
                    sort.key = key;
                    sort.type = type;
                    sort.dir = type === "num" ? "desc" : "asc";
                }
                setSort(sort);
                // Update header classes
                for (let j = 0; j < ths.length; j++) {
                    ths[j].classList.remove("sorted", "asc", "desc");
                }
                th.classList.add("sorted", sort.dir);
                rerender();
            });
        }
    }
    function renderSnapshot(snap, modeLabel) {
        if (!snap)
            return;
        const flags = snap.flags || {};
        lastFlags = flags;
        setPhase(snap.phase);
        hostsEl.textContent =
            snap.hosts && snap.hosts.length ? snap.hosts.join(", ") : "—";
        durationEl.textContent = formatDuration(snap.duration_secs);
        kpiUsers.textContent =
            formatInt(snap.active_users) + " / " + formatInt(snap.maximum_users);
        const agg = snap.aggregate || {};
        kpiRps.textContent = formatRate(agg.requests_per_second);
        kpiFail.textContent = formatPct(agg.failure_rate);
        const p = agg.percentile_ms || {};
        kpiP95.textContent = formatInt(p.p95);
        kpiAvg.textContent = formatRate(agg.response_time_avg_ms);
        summaryEl.textContent = "";
        summaryEl.appendChild(kv("Phase", snap.phase));
        summaryEl.appendChild(kv("Duration", formatDuration(snap.duration_secs)));
        summaryEl.appendChild(kv("Users", formatInt(snap.active_users) + " / " + formatInt(snap.maximum_users)));
        summaryEl.appendChild(kv("Total users", formatInt(snap.total_users)));
        summaryEl.appendChild(kv("Goose", snap.goose_version));
        summaryEl.appendChild(kv("Hosts", snap.hosts && snap.hosts.length ? snap.hosts.join(", ") : "—"));
        if (flags.series_seconds) {
            summaryEl.appendChild(kv("Series window", formatInt(flags.series_seconds) + "s"));
        }
        aggregateEl.textContent = "";
        aggregateEl.appendChild(kv("Requests", formatInt(agg.total_requests)));
        aggregateEl.appendChild(kv("Failures", formatInt(agg.total_failures)));
        aggregateEl.appendChild(kv("RPS", formatRate(agg.requests_per_second)));
        aggregateEl.appendChild(kv("Fail %", formatPct(agg.failure_rate)));
        aggregateEl.appendChild(kv("Avg ms", formatRate(agg.response_time_avg_ms)));
        aggregateEl.appendChild(kv("p50 / p95 / p99", formatInt(p.p50) + " / " + formatInt(p.p95) + " / " + formatInt(p.p99)));
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
            setBanner("Metrics are disabled (--no-metrics). The dashboard shell is live, but request/series data is empty.", "warn");
        }
        else if (flags.requests_truncated || flags.errors_truncated) {
            const parts = [];
            if (flags.requests_truncated)
                parts.push("request rows truncated");
            if (flags.errors_truncated)
                parts.push("error rows truncated");
            setBanner(parts.join("; ") + " (showing top rows only).", "info");
        }
        else if (controlTokenMissing) {
            setBanner("Open this dashboard as http://host:port/?token=… (token required for control).", "error");
        }
        else if (authRequired) {
            // Clear previous auth banner once we have data.
            setBanner("");
            authRequired = false;
        }
        else if (chartLoadWarned && !chartsReady) {
            setBanner("Charts unavailable — Chart.js failed to load.", "warn");
        }
        else {
            setBanner("");
        }
        if (modeLabel === "live") {
            setConnection("live");
        }
        else if (modeLabel === "poll") {
            setConnection("poll");
        }
    }
    function withToken(path) {
        if (!token)
            return path;
        const sep = path.indexOf("?") >= 0 ? "&" : "?";
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
            }
            catch (_a) {
                /* ignore */
            }
            eventSource = null;
        }
        setConnection("disconnected");
        setBanner("Open this dashboard as http://host:port/?token=… (token required for metrics).", "error");
    }
    function fetchSnapshotOnce(modeLabel) {
        if (authBlocked)
            return Promise.resolve(null);
        return fetch(snapshotUrl())
            .then((res) => {
            if (res.status === 401) {
                showAuthMissing();
                return null;
            }
            if (!res.ok) {
                throw new Error("HTTP " + res.status);
            }
            return res.json();
        })
            .then((snap) => {
            if (snap) {
                renderSnapshot(snap, modeLabel || "poll");
            }
            return snap;
        });
    }
    function startPollFallback(reason) {
        if (finished || authBlocked)
            return;
        if (usingPoll && pollTimer != null)
            return;
        usingPoll = true;
        if (eventSource) {
            try {
                eventSource.close();
            }
            catch (_a) {
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
            fetchSnapshotOnce("poll").catch((err) => {
                if (authBlocked)
                    return;
                setConnection("disconnected");
                setBanner("Poll failed: " + String(err), "error");
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
        }
        catch (_a) {
            startPollFallback("SSE open failed");
            return;
        }
        let sawSnapshot = false;
        // After the first snapshot, EventSource auto-reconnects; if errors keep
        // stacking without a fresh snapshot, fall back to poll so the UI recovers.
        let sseErrorStreak = 0;
        const SSE_ERROR_FALLBACK_THRESHOLD = 3;
        eventSource.addEventListener("snapshot", (ev) => {
            try {
                const snap = JSON.parse(ev.data);
                sawSnapshot = true;
                sseErrorStreak = 0;
                usingPoll = false;
                stopPoll();
                renderSnapshot(snap, "live");
            }
            catch (err) {
                setBanner("Bad snapshot event: " + String(err), "error");
            }
        });
        eventSource.addEventListener("closed", () => {
            finished = true;
            try {
                if (eventSource)
                    eventSource.close();
            }
            catch (_a) {
                /* ignore */
            }
            eventSource = null;
            stopPoll();
            setConnection("closed");
            setBanner("Load test finished.", "info");
        });
        eventSource.onerror = () => {
            if (finished) {
                return;
            }
            // EventSource reconnects automatically on transient errors; fall back to
            // poll when we never received a snapshot, or after repeated errors once
            // live (server gone, 503 cap, sticky-closed without closed event).
            if (!sawSnapshot) {
                try {
                    if (eventSource)
                        eventSource.close();
                }
                catch (_a) {
                    /* ignore */
                }
                eventSource = null;
                // Probe once to distinguish auth failure from other SSE issues.
                fetch(snapshotUrl())
                    .then((res) => {
                    if (res.status === 401) {
                        showAuthMissing();
                        return;
                    }
                    startPollFallback("SSE failed");
                })
                    .catch(() => {
                    startPollFallback("SSE failed");
                });
            }
            else {
                sseErrorStreak += 1;
                setConnection("disconnected");
                if (sseErrorStreak >= SSE_ERROR_FALLBACK_THRESHOLD) {
                    try {
                        if (eventSource)
                            eventSource.close();
                    }
                    catch (_b) {
                        /* ignore */
                    }
                    eventSource = null;
                    startPollFallback("SSE reconnect failed");
                }
            }
        };
    }
    // Wire UI controls
    wireSort("requests-table", () => reqSort, (s) => {
        reqSort = s;
    }, () => {
        renderRequestTable(lastFlags);
    });
    wireSort("errors-table", () => errSort, (s) => {
        errSort = s;
    }, () => {
        renderErrorTable();
    });
    if (requestsFilter) {
        requestsFilter.addEventListener("input", () => {
            renderRequestTable(lastFlags);
        });
    }
    ensureCharts();
    initControlPanel();
    startSse();
})();
