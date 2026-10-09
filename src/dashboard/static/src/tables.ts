// The requests and errors tables: sorting, the requests filter, and both
// renders.

import { optionalElement, requireElement } from "./dom";
import { formatInt, formatRate, textCell } from "./format";

type SortDir = "asc" | "desc";
type SortType = "num" | "str";

interface SortState<K extends string> {
  key: K;
  type: SortType;
  dir: SortDir;
}

/** The RequestRow fields the requests table shows, percentiles inlined. */
interface FlatRequestRow {
  method: string;
  name: string;
  request_count: number;
  failure_count: number;
  requests_per_second: number;
  response_time_avg_ms: number;
  p50: number;
  p95: number;
  p99: number;
}

type RequestSortKey = keyof FlatRequestRow;
type ErrorSortKey = keyof ErrorRow;

// Columns a table header's data-sort attribute may name. Records, so the
// compiler requires every row field to be listed.
const REQUEST_SORT_COLUMNS: Record<RequestSortKey, true> = {
  method: true,
  name: true,
  request_count: true,
  failure_count: true,
  requests_per_second: true,
  response_time_avg_ms: true,
  p50: true,
  p95: true,
  p99: true,
};
const ERROR_SORT_COLUMNS: Record<ErrorSortKey, true> = {
  method: true,
  name: true,
  error: true,
  occurrences: true,
};
const REQUEST_SORT_KEYS = Object.keys(REQUEST_SORT_COLUMNS) as RequestSortKey[];
const ERROR_SORT_KEYS = Object.keys(ERROR_SORT_COLUMNS) as ErrorSortKey[];

const requestsBody = requireElement("requests-body", HTMLTableSectionElement);
const errorsBody = requireElement("errors-body", HTMLTableSectionElement);
const requestsFilter = optionalElement("requests-filter", HTMLInputElement);

let requestRows: FlatRequestRow[] = [];
let errorRows: ErrorRow[] = [];
let lastFlags: SnapshotFlags | null = null;
let reqSort: SortState<RequestSortKey> = {
  key: "request_count",
  type: "num",
  dir: "desc",
};
let errSort: SortState<ErrorSortKey> = {
  key: "occurrences",
  type: "num",
  dir: "desc",
};

function sortRows<T, K extends keyof T & string>(
  rows: T[],
  sort: SortState<K>
): T[] {
  const key = sort.key;
  const type = sort.type;
  const dir = sort.dir === "asc" ? 1 : -1;
  const copy = rows.slice();
  copy.sort((a, b) => {
    const av: unknown = a[key];
    const bv: unknown = b[key];
    if (type === "num") {
      const an = typeof av === "number" ? av : 0;
      const bn = typeof bv === "number" ? bv : 0;
      return (an - bn) * dir;
    }
    const as = av == null ? "" : String(av);
    const bs = bv == null ? "" : String(bv);
    if (as < bs) return -1 * dir;
    if (as > bs) return 1 * dir;
    return 0;
  });
  return copy;
}

function flattenRequest(r: RequestRow): FlatRequestRow {
  const p = r.percentile_ms;
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

function renderRequestTable(flags: SnapshotFlags | null): void {
  const filter = (requestsFilter && requestsFilter.value
    ? requestsFilter.value
    : ""
  )
    .toLowerCase()
    .trim();
  let rows = sortRows(requestRows, reqSort);
  if (filter) {
    rows = rows.filter((r) => {
      return (
        String(r.method).toLowerCase().indexOf(filter) >= 0 ||
        String(r.name).toLowerCase().indexOf(filter) >= 0
      );
    });
  }

  requestsBody.textContent = "";
  if (requestRows.length === 0) {
    const empty = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 9;
    td.className = "empty";
    if (flags && flags.metrics_disabled) {
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

function renderErrorTable(): void {
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

function wireSort<K extends string>(
  tableId: string,
  keys: readonly K[],
  getSort: () => SortState<K>,
  setSort: (s: SortState<K>) => void,
  rerender: () => void
): void {
  const table = document.getElementById(tableId);
  if (!table) return;
  const ths = table.querySelectorAll<HTMLTableCellElement>(
    "thead th[data-sort]"
  );
  for (let i = 0; i < ths.length; i++) {
    const th = ths[i];
    th.addEventListener("click", () => {
      const attr = th.getAttribute("data-sort");
      const key = keys.find((k) => k === attr);
      if (key === undefined) return;
      const typeAttr = th.getAttribute("data-type") || "str";
      const type: SortType = typeAttr === "num" ? "num" : "str";
      const sort = getSort();
      if (sort.key === key) {
        sort.dir = sort.dir === "asc" ? "desc" : "asc";
      } else {
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

/** Stores a snapshot's rows and renders both tables. */
export function setTableData(snap: DashboardSnapshot): void {
  lastFlags = snap.flags;
  requestRows = snap.requests.map(flattenRequest);
  errorRows = snap.errors.slice();
  renderRequestTable(snap.flags);
  renderErrorTable();
}

/** Wires the sortable headers and the requests filter. */
export function initTables(): void {
  wireSort(
    "requests-table",
    REQUEST_SORT_KEYS,
    () => reqSort,
    (s) => {
      reqSort = s;
    },
    () => {
      renderRequestTable(lastFlags);
    }
  );
  wireSort(
    "errors-table",
    ERROR_SORT_KEYS,
    () => errSort,
    (s) => {
      errSort = s;
    },
    () => {
      renderErrorTable();
    }
  );
  if (requestsFilter) {
    requestsFilter.addEventListener("input", () => {
      renderRequestTable(lastFlags);
    });
  }
}
