/**
 * Ambient types for the vendored Chart.js UMD build (`chart.min.js`).
 * Loaded as a global `Chart` before `app.js` in index.html.
 */

interface DashboardChartDataset {
  label: string;
  data: number[];
  borderColor: string;
  backgroundColor: string;
  borderWidth: number;
  pointRadius: number;
  tension: number;
  fill?: boolean;
}

interface DashboardChartData {
  labels: string[];
  datasets: DashboardChartDataset[];
}

interface DashboardChart {
  data: DashboardChartData;
  update(mode?: string): void;
}

interface DashboardChartConfiguration {
  type: "line";
  data: DashboardChartData;
  options: object;
}

interface DashboardChartConstructor {
  new (
    item: HTMLCanvasElement | null,
    config: DashboardChartConfiguration
  ): DashboardChart;
}

/** Present when `/static/chart.min.js` loaded successfully. */
declare const Chart: DashboardChartConstructor | undefined;
