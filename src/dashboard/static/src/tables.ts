// The scenarios, transactions, requests and errors tables: one shared
// sortable table (sorting, optional filter, empty states, render) and the
// column definitions of each.

import { shownTime } from "./coadjust";
import { optionalElement, requireElement } from "./dom";
import { formatInt, formatRate, textCell } from "./format";

type SortDir = "asc" | "desc";
type SortType = "num" | "str";

interface SortState<K extends string> {
  key: K;
  type: SortType;
  dir: SortDir;
}

/** One column: the row field a `th[data-sort]` names, and its cell text. */
interface Column<R> {
  key: keyof R & string;
  type: SortType;
  format: (row: R) => string;
  /** The cell's hover text, or null for none. */
  title?: (row: R) => string | null;
}

interface TableFilter<R> {
  input: HTMLInputElement | null;
  /** True when the row matches `needle`, already lower case and trimmed. */
  matches: (row: R, needle: string) => boolean;
  noMatchText: string;
}

interface TableOptions<R> {
  tableId: string;
  body: HTMLTableSectionElement;
  columns: Column<R>[];
  sort: SortState<keyof R & string>;
  emptyText: string;
  /** The text shown instead of rows when the table is off, or null. */
  disabledText: (flags: SnapshotFlags) => string | null;
  filter?: TableFilter<R>;
}

interface SortableTable<R> {
  setRows(rows: R[], flags: SnapshotFlags): void;
  /** Wires the sortable headers and the filter input. */
  init(): void;
}

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

function sortableTable<R>(options: TableOptions<R>): SortableTable<R> {
  const { body, columns, filter } = options;
  let sort = options.sort;
  let rows: R[] = [];
  let flags: SnapshotFlags | null = null;

  function emptyRow(text: string): void {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = columns.length;
    td.className = "empty";
    td.textContent = text;
    tr.appendChild(td);
    body.appendChild(tr);
  }

  function render(): void {
    body.textContent = "";
    const disabled = flags ? options.disabledText(flags) : null;
    if (disabled !== null || rows.length === 0) {
      emptyRow(disabled !== null ? disabled : options.emptyText);
      return;
    }

    let shown = sortRows(rows, sort);
    if (filter) {
      const needle = (filter.input && filter.input.value
        ? filter.input.value
        : ""
      )
        .toLowerCase()
        .trim();
      if (needle) {
        shown = shown.filter((r) => filter.matches(r, needle));
        if (shown.length === 0) {
          emptyRow(filter.noMatchText);
          return;
        }
      }
    }

    for (let i = 0; i < shown.length; i++) {
      const row = shown[i];
      const tr = document.createElement("tr");
      for (let c = 0; c < columns.length; c++) {
        const column = columns[c];
        const td = textCell(column.format(row));
        const title = column.title ? column.title(row) : null;
        if (title !== null) td.title = title;
        tr.appendChild(td);
      }
      body.appendChild(tr);
    }
  }

  function wireSort(): void {
    const table = document.getElementById(options.tableId);
    if (!table) return;
    const ths = table.querySelectorAll<HTMLTableCellElement>(
      "thead th[data-sort]"
    );
    for (let i = 0; i < ths.length; i++) {
      const th = ths[i];
      th.addEventListener("click", () => {
        const attr = th.getAttribute("data-sort");
        const column = columns.find((c) => c.key === attr);
        if (column === undefined) return;
        if (sort.key === column.key) {
          sort = { ...sort, dir: sort.dir === "asc" ? "desc" : "asc" };
        } else {
          sort = {
            key: column.key,
            type: column.type,
            dir: column.type === "num" ? "desc" : "asc",
          };
        }
        for (let j = 0; j < ths.length; j++) {
          ths[j].classList.remove("sorted", "asc", "desc");
        }
        th.classList.add("sorted", sort.dir);
        render();
      });
    }
  }

  return {
    setRows(next: R[], nextFlags: SnapshotFlags): void {
      rows = next;
      flags = nextFlags;
      render();
    },
    init(): void {
      wireSort();
      if (filter && filter.input) {
        filter.input.addEventListener("input", render);
      }
    },
  };
}

function includes(value: string, needle: string): boolean {
  return value.toLowerCase().indexOf(needle) >= 0;
}

/** Timing cells stay empty until something has been timed. */
function timing(runCount: number, text: string): string {
  return runCount > 0 ? text : "";
}

/** The ScenarioRow fields the scenarios table shows, percentiles inlined. */
interface FlatScenarioRow {
  scenario_index: number;
  scenario_name: string;
  users: number;
  run_count: number;
  runs_per_second: number;
  /** run_count / users, or 0 when no user has finished a run. */
  runs_per_user: number;
  response_time_avg_ms: number;
  p50: number;
  p95: number;
  p99: number;
}

/** The TransactionRow fields the transactions table shows. */
interface FlatTransactionRow {
  /** Sorts the `#` column: scenario first, then transaction. */
  order: number;
  scenario_index: number;
  transaction_index: number;
  scenario_name: string;
  transaction_name: string;
  run_count: number;
  failure_count: number;
  runs_per_second: number;
  response_time_avg_ms: number;
  p50: number;
  p95: number;
  p99: number;
}

/**
 * The RequestRow fields the requests table shows, percentiles inlined. The
 * response times are the adjusted ones when the row has them, so the table
 * sorts on what it shows, and `measured` holds each one's hover text.
 */
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
  measured: {
    response_time_avg_ms: string | null;
    p50: string | null;
    p95: string | null;
    p99: string | null;
  };
}

function flattenScenario(r: ScenarioRow): FlatScenarioRow {
  const p = r.percentile_ms;
  return {
    scenario_index: r.scenario_index,
    scenario_name: r.scenario_name,
    users: r.users,
    run_count: r.run_count,
    runs_per_second: r.runs_per_second,
    runs_per_user: r.users > 0 ? r.run_count / r.users : 0,
    response_time_avg_ms: r.response_time_avg_ms,
    p50: p.p50,
    p95: p.p95,
    p99: p.p99,
  };
}

function flattenTransaction(r: TransactionRow): FlatTransactionRow {
  const p = r.percentile_ms;
  return {
    order: r.scenario_index * 1000000 + r.transaction_index,
    scenario_index: r.scenario_index,
    transaction_index: r.transaction_index,
    scenario_name: r.scenario_name,
    transaction_name: r.transaction_name,
    run_count: r.run_count,
    failure_count: r.failure_count,
    runs_per_second: r.runs_per_second,
    response_time_avg_ms: r.response_time_avg_ms,
    p50: p.p50,
    p95: p.p95,
    p99: p.p99,
  };
}

function flattenRequest(r: RequestRow): FlatRequestRow {
  const p = r.percentile_ms;
  const co = r.co_adjusted != null ? r.co_adjusted : null;
  const cp = co ? co.percentile_ms : null;
  const avg = shownTime(
    r.response_time_avg_ms,
    co ? co.response_time_avg_ms : null,
    formatRate
  );
  const p50 = shownTime(p.p50, cp ? cp.p50 : null, formatInt);
  const p95 = shownTime(p.p95, cp ? cp.p95 : null, formatInt);
  const p99 = shownTime(p.p99, cp ? cp.p99 : null, formatInt);
  return {
    method: r.method,
    name: r.name,
    request_count: r.request_count,
    failure_count: r.failure_count,
    requests_per_second: r.requests_per_second,
    response_time_avg_ms: avg.value,
    p50: p50.value,
    p95: p95.value,
    p99: p99.value,
    measured: {
      response_time_avg_ms: avg.title,
      p50: p50.title,
      p95: p95.title,
      p99: p99.title,
    },
  };
}

const scenariosTable = sortableTable<FlatScenarioRow>({
  tableId: "scenarios-table",
  body: requireElement("scenarios-body", HTMLTableSectionElement),
  columns: [
    {
      key: "scenario_index",
      type: "num",
      format: (r) => String(r.scenario_index + 1),
    },
    { key: "scenario_name", type: "str", format: (r) => r.scenario_name },
    { key: "users", type: "num", format: (r) => formatInt(r.users) },
    { key: "run_count", type: "num", format: (r) => formatInt(r.run_count) },
    {
      key: "runs_per_second",
      type: "num",
      format: (r) => formatRate(r.runs_per_second),
    },
    {
      key: "runs_per_user",
      type: "num",
      format: (r) => (r.users > 0 ? formatRate(r.runs_per_user) : ""),
    },
    {
      key: "response_time_avg_ms",
      type: "num",
      format: (r) => timing(r.run_count, formatRate(r.response_time_avg_ms)),
    },
    { key: "p50", type: "num", format: (r) => timing(r.run_count, formatInt(r.p50)) },
    { key: "p95", type: "num", format: (r) => timing(r.run_count, formatInt(r.p95)) },
    { key: "p99", type: "num", format: (r) => timing(r.run_count, formatInt(r.p99)) },
  ],
  sort: { key: "scenario_index", type: "num", dir: "asc" },
  emptyText: "No scenarios yet",
  disabledText: (flags) => {
    if (flags.metrics_disabled) return "Metrics disabled (--no-metrics).";
    if (flags.scenario_metrics_disabled) {
      return "Scenario metrics disabled (--no-scenario-metrics).";
    }
    return null;
  },
});

const transactionsTable = sortableTable<FlatTransactionRow>({
  tableId: "transactions-table",
  body: requireElement("transactions-body", HTMLTableSectionElement),
  columns: [
    {
      key: "order",
      type: "num",
      format: (r) => r.scenario_index + 1 + "." + (r.transaction_index + 1),
    },
    { key: "scenario_name", type: "str", format: (r) => r.scenario_name },
    { key: "transaction_name", type: "str", format: (r) => r.transaction_name },
    { key: "run_count", type: "num", format: (r) => formatInt(r.run_count) },
    { key: "failure_count", type: "num", format: (r) => formatInt(r.failure_count) },
    {
      key: "runs_per_second",
      type: "num",
      format: (r) => formatRate(r.runs_per_second),
    },
    {
      key: "response_time_avg_ms",
      type: "num",
      format: (r) => timing(r.run_count, formatRate(r.response_time_avg_ms)),
    },
    { key: "p50", type: "num", format: (r) => timing(r.run_count, formatInt(r.p50)) },
    { key: "p95", type: "num", format: (r) => timing(r.run_count, formatInt(r.p95)) },
    { key: "p99", type: "num", format: (r) => timing(r.run_count, formatInt(r.p99)) },
  ],
  sort: { key: "order", type: "num", dir: "asc" },
  emptyText: "No transactions yet",
  disabledText: (flags) => {
    if (flags.metrics_disabled) return "Metrics disabled (--no-metrics).";
    if (flags.transaction_metrics_disabled) {
      return "Transaction metrics disabled (--no-transaction-metrics).";
    }
    return null;
  },
  filter: {
    input: optionalElement("transactions-filter", HTMLInputElement),
    matches: (r, needle) =>
      includes(r.scenario_name, needle) || includes(r.transaction_name, needle),
    noMatchText: "No matching transactions",
  },
});

const requestsTable = sortableTable<FlatRequestRow>({
  tableId: "requests-table",
  body: requireElement("requests-body", HTMLTableSectionElement),
  columns: [
    { key: "method", type: "str", format: (r) => r.method },
    { key: "name", type: "str", format: (r) => r.name },
    { key: "request_count", type: "num", format: (r) => formatInt(r.request_count) },
    { key: "failure_count", type: "num", format: (r) => formatInt(r.failure_count) },
    {
      key: "requests_per_second",
      type: "num",
      format: (r) => formatRate(r.requests_per_second),
    },
    {
      key: "response_time_avg_ms",
      type: "num",
      format: (r) => formatRate(r.response_time_avg_ms),
      title: (r) => r.measured.response_time_avg_ms,
    },
    {
      key: "p50",
      type: "num",
      format: (r) => formatInt(r.p50),
      title: (r) => r.measured.p50,
    },
    {
      key: "p95",
      type: "num",
      format: (r) => formatInt(r.p95),
      title: (r) => r.measured.p95,
    },
    {
      key: "p99",
      type: "num",
      format: (r) => formatInt(r.p99),
      title: (r) => r.measured.p99,
    },
  ],
  sort: { key: "request_count", type: "num", dir: "desc" },
  emptyText: "No requests yet",
  disabledText: (flags) =>
    flags.metrics_disabled
      ? "Metrics disabled (--no-metrics). Charts and request tables are empty."
      : null,
  filter: {
    input: optionalElement("requests-filter", HTMLInputElement),
    matches: (r, needle) => includes(r.method, needle) || includes(r.name, needle),
    noMatchText: "No matching requests",
  },
});

const errorsTable = sortableTable<ErrorRow>({
  tableId: "errors-table",
  body: requireElement("errors-body", HTMLTableSectionElement),
  columns: [
    { key: "method", type: "str", format: (r) => r.method },
    { key: "name", type: "str", format: (r) => r.name },
    { key: "error", type: "str", format: (r) => r.error },
    { key: "occurrences", type: "num", format: (r) => formatInt(r.occurrences) },
  ],
  sort: { key: "occurrences", type: "num", dir: "desc" },
  emptyText: "No errors",
  disabledText: () => null,
});

/** Stores a snapshot's rows and renders every table. */
export function setTableData(snap: DashboardSnapshot): void {
  scenariosTable.setRows(snap.scenarios.map(flattenScenario), snap.flags);
  transactionsTable.setRows(
    snap.transactions.map(flattenTransaction),
    snap.flags
  );
  requestsTable.setRows(snap.requests.map(flattenRequest), snap.flags);
  errorsTable.setRows(snap.errors.slice(), snap.flags);
}

/** Wires the sortable headers and the filters. */
export function initTables(): void {
  scenariosTable.init();
  transactionsTable.init();
  requestsTable.init();
  errorsTable.init();
}
