// The "Last 5 minutes / Whole run" toggle over the time charts. On "Whole
// run" the page asks `GET /api/v1/whole-run` for the line charts and the
// response time heatmap over the whole run, at most once a second and never
// twice at once, and draws them in the same charts and with the same
// heatmap drawing as the live window. On "Last 5 minutes" it asks for
// nothing more, and the charts draw the snapshots again. The choice is kept
// for the tab's session only.

import { updateChartData } from "./chart-data";
import { updateCharts, wholeRunSeries } from "./charts";
import { fetchWholeRun } from "./connection";
import { requireElement } from "./dom";
import type { WholeRun } from "./heatmap";
import {
  chosenRequest,
  setHeatmapWholeRun,
  setWholeRunHeatmap,
  showsAverage,
} from "./heatmap-card";

/** sessionStorage key of the choice: "whole", or nothing for the live window. */
export const RANGE_STORAGE_KEY = "goose-dashboard-chart-range";

/** How often the whole run view is asked for while shown. */
const WHOLE_RUN_INTERVAL_MS = 1000;

const recentButton = requireElement("range-recent", HTMLButtonElement);
const wholeButton = requireElement("range-whole", HTMLButtonElement);

let whole = false;
let timer: ReturnType<typeof setInterval> | null = null;
let inFlight = false;
/** Draws the live window again from the last snapshot, given by main. */
let redrawRecent: () => void = () => {};

/** Whether the charts show the whole run. */
export function showsWholeRun(): boolean {
  return whole;
}

function storedChoice(): boolean {
  try {
    return window.sessionStorage.getItem(RANGE_STORAGE_KEY) === "whole";
  } catch {
    return false;
  }
}

function storeChoice(): void {
  try {
    if (whole) {
      window.sessionStorage.setItem(RANGE_STORAGE_KEY, "whole");
    } else {
      window.sessionStorage.removeItem(RANGE_STORAGE_KEY);
    }
  } catch {
    // Storage blocked: the choice lasts until the page is left.
  }
}

function setButtons(): void {
  recentButton.setAttribute("aria-pressed", whole ? "false" : "true");
  wholeButton.setAttribute("aria-pressed", whole ? "true" : "false");
}

/** Draws a whole run view in every time chart. */
function show(view: WholeRun): void {
  const series = wholeRunSeries(view.lines);
  const width = view.lines.width;
  const average = showsAverage();
  setWholeRunHeatmap(view.heatmap, view.lines.end_second);
  updateChartData(series, average, 0, width, true);
  updateCharts(series, average, width, true);
}

/** Asks for the whole run view and draws it, unless one is still on its way. */
function poll(): void {
  if (!whole || inFlight) return;
  inFlight = true;
  const request = chosenRequest();
  void fetchWholeRun(request).then((data) => {
    inFlight = false;
    // Drawn only if the page still shows the whole run of the same choice.
    if (!whole || data === null || chosenRequest() !== request) return;
    show(data as WholeRun);
  });
}

function choose(next: boolean): void {
  if (next === whole) return;
  whole = next;
  storeChoice();
  setButtons();
  if (timer !== null) {
    clearInterval(timer);
    timer = null;
  }
  setHeatmapWholeRun(whole, poll);
  if (whole) {
    timer = setInterval(poll, WHOLE_RUN_INTERVAL_MS);
    poll();
  } else {
    redrawRecent();
  }
}

/**
 * Wires the toggle, with `redraw` drawing the live window from the last
 * snapshot, and restores the tab's last choice.
 */
export function initRangeToggle(redraw: () => void): void {
  redrawRecent = redraw;
  recentButton.addEventListener("click", () => choose(false));
  wholeButton.addEventListener("click", () => choose(true));
  setButtons();
  if (storedChoice()) choose(true);
}
