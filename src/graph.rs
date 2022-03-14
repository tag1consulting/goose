//! Optional graph data collected during the load tests.
//!
//! If the HTML report is enabled the graph data will be collected and stored in
//! the [`GraphData`] structure during the load test. When the report is written
//! this data is converted into [`Graph`] structures and HTML markup is generated
//! based on them.

use chrono::prelude::*;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::marker::PhantomData;

/// Used to collect graph data during a load test.
pub(crate) struct GraphData {
    /// Tracks when the load test first started with an optional system timestamp.
    starting: Option<DateTime<Utc>>,
    /// Tracks when all [`GooseUser`](../goose/struct.GooseUser.html) threads fully
    /// started with an optional system timestamp.
    started: Option<DateTime<Utc>>,
    /// Tracks when the load test first began stopping with an optional system timestamp.
    stopping: Option<DateTime<Utc>>,
    /// Tracks when the load test stopped with an optional system timestamp.
    stopped: Option<DateTime<Utc>>,
    /// Counts requests per second for each request type.
    requests_per_second: HashMap<String, TimeSeries<u32, u32>>,
    /// Counts errors per second.
    errors_per_second: HashMap<String, TimeSeries<u32, u32>>,
    /// Maintains average response time per second.
    average_response_time_per_second: HashMap<String, TimeSeries<MovingAverage, f32>>,
    /// Number of tasks at the end of each second of the test.
    tasks_per_second: TimeSeries<usize, usize>,
    /// Number of users at the end of each second of the test.
    users_per_second: TimeSeries<usize, usize>,
}

impl GraphData {
    /// Create a new GraphData object.
    pub(crate) fn new() -> Self {
        trace!("new graph");
        GraphData {
            starting: None,
            started: None,
            stopping: None,
            stopped: None,
            requests_per_second: HashMap::new(),
            errors_per_second: HashMap::new(),
            average_response_time_per_second: HashMap::new(),
            tasks_per_second: TimeSeries::new(),
            users_per_second: TimeSeries::new(),
        }
    }

    /// Sets starting time.
    pub(crate) fn set_starting(&mut self, starting: DateTime<Utc>) {
        self.starting = Some(starting);
    }

    /// Sets started time.
    pub(crate) fn set_started(&mut self, started: DateTime<Utc>) {
        self.started = Some(started);
    }

    /// Sets stopping time.
    pub(crate) fn set_stopping(&mut self, stopping: DateTime<Utc>) {
        self.stopping = Some(stopping);
    }

    /// Sets stopped time.
    pub(crate) fn set_stopped(&mut self, stopped: DateTime<Utc>) {
        self.stopped = Some(stopped);
    }

    /// Record requests per second metric.
    pub(crate) fn record_requests_per_second(&mut self, key: &str, second: usize) {
        if !self.requests_per_second.contains_key(key) {
            self.requests_per_second
                .insert(key.to_string(), TimeSeries::new());
        }
        let data = self.requests_per_second.get_mut(key).unwrap();
        data.increase(second, 1);

        debug!(
            "incremented second {} for requests per second counter: {}",
            second,
            data.get(second)
        );
    }

    /// Record errors per second metric.
    pub(crate) fn record_errors_per_second(&mut self, key: &str, second: usize) {
        if !self.errors_per_second.contains_key(key) {
            self.errors_per_second
                .insert(key.to_string(), TimeSeries::new());
        }
        let data = self.errors_per_second.get_mut(key).unwrap();
        data.increase(second, 1);

        debug!(
            "incremented second {} for errors per second counter: {}",
            second,
            data.get(second)
        );
    }

    /// Record average response time per second metric.
    pub(crate) fn record_average_response_time_per_second(
        &mut self,
        key: String,
        second: usize,
        response_time: u64,
    ) {
        if !self.average_response_time_per_second.contains_key(&key) {
            self.average_response_time_per_second
                .insert(key.clone(), TimeSeries::new());
        }
        let data = self.average_response_time_per_second.get_mut(&key).unwrap();
        data.increase(second, response_time as f32);

        debug!(
            "updated second {} for average response time per second: {}",
            second,
            data.get(second).average
        );
    }

    /// Record tasks per second metric.
    pub(crate) fn record_tasks_per_second(&mut self, second: usize) {
        self.tasks_per_second.increase(second, 1);

        debug!(
            "incremented second {} for tasks per second counter: {}",
            second,
            self.tasks_per_second.get(second)
        );
    }

    /// Records number of users for a current second.
    pub(crate) fn record_users_per_second(&mut self, users: usize, now: DateTime<Utc>) {
        if let Some(starting) = self.starting {
            let second = (now.timestamp() - starting.timestamp()) as usize;
            self.users_per_second.set_and_maintain_last(second, users);
        }
    }

    /// Generate active users graph.
    pub(crate) fn get_active_users_graph(&self) -> Graph<usize, usize> {
        self.create_graph_from_single_data(
            "graph-active-users",
            "Active users #",
            self.users_per_second.clone(),
        )
    }

    /// Generate requests per second graph.
    pub(crate) fn get_requests_per_second_graph(&self) -> Graph<u32, u32> {
        self.create_graph_from_data("graph-rps", "Requests #", self.requests_per_second.clone())
    }

    /// Generate average response time graph.
    pub(crate) fn get_average_response_time_graph(&self) -> Graph<MovingAverage, f32> {
        self.create_graph_from_data(
            "graph-avg-response-time",
            "Response time [ms]",
            self.average_response_time_per_second.clone(),
        )
    }

    /// Generate active tasks graph.
    pub(crate) fn get_tasks_per_second_graph(&self) -> Graph<usize, usize> {
        self.create_graph_from_single_data("graph-tps", "Tasks #", self.tasks_per_second.clone())
    }

    /// Generate errors per second graph.
    pub(crate) fn get_errors_per_second_graph(&self) -> Graph<u32, u32> {
        self.create_graph_from_data("graph-eps", "Errors #", self.errors_per_second.clone())
    }

    /// Creates a Graph from granular data.
    fn create_graph_from_data<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy>(
        &self,
        html_id: &'a str,
        y_axis_label: &'a str,
        data: HashMap<String, TimeSeries<T, U>>,
    ) -> Graph<'a, T, U> {
        Graph::new(
            html_id,
            y_axis_label,
            data,
            self.starting.unwrap(),
            if self.started.is_none() && self.stopping.is_some() {
                self.stopping
            } else {
                self.started
            },
            self.stopping,
            self.stopped,
        )
    }

    /// Creates a Graph from single (just total numbers, not granular) data.
    fn create_graph_from_single_data<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy>(
        &self,
        html_id: &'a str,
        y_axis_label: &'a str,
        data: TimeSeries<T, U>,
    ) -> Graph<'a, T, U> {
        let mut hash_map_data = HashMap::new();
        hash_map_data.insert("Total".to_string(), data);

        Graph::new(
            html_id,
            y_axis_label,
            hash_map_data,
            self.starting.unwrap(),
            if self.started.is_none() && self.stopping.is_some() {
                self.stopping
            } else {
                self.started
            },
            self.stopping,
            self.stopped,
        )
    }
}

/// Defines the HTML graph data.
#[derive(Debug)]
pub(crate) struct Graph<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy> {
    /// HTML ID of the graph's main wrapper.
    html_id: &'a str,
    /// Label of the y axis.
    y_axis_label: &'a str,
    /// Graph data.
    data: HashMap<String, TimeSeries<T, U>>,
    /// Time when the test startup phase began.
    starting: DateTime<Utc>,
    /// Time when the test was started (startup phase completed).
    started: Option<DateTime<Utc>>,
    /// Time when the test stopping phase began.
    stopping: Option<DateTime<Utc>>,
    /// Time when the test was completely stopped.
    stopped: Option<DateTime<Utc>>,
}

impl<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy> Graph<'a, T, U> {
    /// Creates a new Graph object.
    fn new(
        html_id: &'a str,
        y_axis_label: &'a str,
        data: HashMap<String, TimeSeries<T, U>>,
        starting: DateTime<Utc>,
        started: Option<DateTime<Utc>>,
        stopping: Option<DateTime<Utc>>,
        stopped: Option<DateTime<Utc>>,
    ) -> Graph<'a, T, U> {
        Graph {
            html_id,
            y_axis_label,
            data,
            starting,
            started,
            stopping,
            stopped,
        }
    }

    /// Helper function to build HTML charts powered by the
    /// [ECharts](https://echarts.apache.org) library.
    pub(crate) fn get_markup(self) -> String {
        let datetime_format = "%Y-%m-%d %H:%M:%S";

        let starting_area = if self.started.is_some() {
            format!(
                r#"[
                                            {{
                                                name: 'Starting',
                                                xAxis: '{starting}'
                                            }},
                                            {{
                                                xAxis: '{started}'
                                            }}
                                        ],"#,
                starting = Local
                    .timestamp(self.starting.timestamp(), 0)
                    .format(datetime_format),
                started = Local
                    .timestamp(self.started.unwrap().timestamp(), 0)
                    .format(datetime_format),
            )
        } else {
            "".to_string()
        };

        let stopping_area = if self.stopping.is_some() && self.stopped.is_some() {
            format!(
                r#"[
                                            {{
                                                name: 'Stopping',
                                                xAxis: '{stopping}'
                                            }},
                                            {{
                                                xAxis: '{stopped}'
                                            }}
                                        ],"#,
                stopping = Local
                    .timestamp(self.stopping.unwrap().timestamp(), 0)
                    .format(datetime_format),
                stopped = Local
                    .timestamp(self.stopped.unwrap().timestamp(), 0)
                    .format(datetime_format),
            )
        } else {
            "".to_string()
        };

        let mut total_values: TimeSeries<T, U> = TimeSeries::new();
        let (legend, main_title, main_values, other_values) = if self.data.len() > 1 {
            // If we are dealing with a metric with granular data we need to calculate totals.
            for (_, single_data) in self.data.iter() {
                total_values.add(single_data);
            }

            // We will have multiple lines. We need to prepare the legend section on the graph
            // and create data series for all of them.
            let mut legend = vec!["Total"];

            let mut other_values = String::new();
            for (title, sub_data) in self.data.iter() {
                legend.push(title);

                let formatted_line = format!(
                    r#"{{
                                name: '{title}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: {values},
                            }},
                            "#,
                    title = title,
                    values =
                        json!(self.add_timestamp_to_html_graph_data(&sub_data.get_graph_data()))
                );
                other_values += formatted_line.as_str();
            }

            (
                format!(
                    r#"legend: {{
                            type: '{legend_type}',
                            width: '75%',
                            data: {data},
                        }},"#,
                    legend_type = if self.data.len() > 4 {
                        "scroll"
                    } else {
                        "plain"
                    },
                    data = json!(legend)
                ),
                "Total",
                &total_values,
                other_values,
            )
        } else {
            // If there is only one data series in the metric we simply display it.
            (
                "".to_string(),
                self.data.keys().next().unwrap().as_str(),
                self.data.values().next().unwrap(),
                "".to_string(),
            )
        };

        format!(
            r#"<div class="graph">
                <div id="{html_id}" style="width: 1000px; height:500px; background: white;"></div>

                <script type="text/javascript">
                    var chartDom = document.getElementById('{html_id}');
                    var myChart = echarts.init(chartDom);

                    myChart.setOption({{
                        color: ['#2c664f', '#5470c6', '#91cc75', '#fac858', '#ee6666', '#73c0de', '#3ba272', '#fc8452', '#9a60b4', '#ea7ccc'],
                        tooltip: {{ trigger: 'axis' }},
                        toolbox: {{
                            feature: {{
                                dataZoom: {{ yAxisIndex: 'none' }},
                                restore: {{}},
                                saveAsImage: {{}}
                            }}
                        }},
                        dataZoom: [
                            {{
                                type: 'inside',
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                            {{
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                        ],
                        xAxis: {{ type: 'time' }},
                        yAxis: {{
                            name: '{y_axis_label}',
                            nameLocation: 'center',
                            nameRotate: 90,
                            nameGap: 45,
                            type: 'value'
                        }},
                        {legend}
                        series: [
                            {{
                                name: '{main_title}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    itemStyle: {{ color: 'rgba(6, 6, 6, 0.10)' }},
                                    data: [
                                        {starting_area}
                                        {stopping_area}
                                    ]
                                }},
                                data: {main_values},
                            }},
                            {other_values}
                        ]
                    }});
                </script>
            </div>"#,
            html_id = self.html_id,
            main_values =
                json!(self.add_timestamp_to_html_graph_data(&main_values.get_graph_data())),
            starting_area = starting_area,
            stopping_area = stopping_area,
            y_axis_label = self.y_axis_label,
            main_title = main_title,
            legend = legend,
            other_values = other_values
        )
    }

    /// Adds timestamps to the graph data series to ensure correct time display on x axis.
    ///
    /// Will take a vector of (generally numerical) values and convert them into tuples where
    /// the second element will be the data point and the first element will be formatted time
    /// it belongs to.
    fn add_timestamp_to_html_graph_data(&self, data: &[U]) -> Vec<(String, U)> {
        data.iter()
            .enumerate()
            .map(|(second, value)| {
                (
                    Local
                        .timestamp(second as i64 + self.starting.timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    *value,
                )
            })
            .collect::<Vec<_>>()
    }
}

/// Data structure to represent time series data.
#[derive(Debug, Clone, PartialEq)]
struct TimeSeries<T: TimeSeriesValue<T, U>, U> {
    /// Time series data.
    ///
    /// Each element of the vector represents value for one second in the time series.
    data: Vec<T>,
    /// Phantom data indicates to the compiler that the "U" generic data type has zero size.
    phantom: PhantomData<U>,
}

impl<T: Clone + TimeSeriesValue<T, U>, U> TimeSeries<T, U> {
    /// Creates a new TimeSeries object.
    fn new() -> TimeSeries<T, U> {
        TimeSeries {
            data: Vec::new(),
            phantom: PhantomData,
        }
    }

    /// Increases the the value for a given second.
    fn increase(&mut self, second: usize, value: U) {
        self.expand(second, T::initial_value());
        self.data[second].add(value);
    }

    /// Adds another time series.
    fn add(&mut self, other: &TimeSeries<T, U>) {
        for (second, other_item) in other.data.iter().enumerate() {
            self.expand(second, T::initial_value());
            self.data.get_mut(second).unwrap().merge(other_item);
        }
    }

    /// Sets a value for a given second and maintains last recorded value if
    /// there is a gap in the time series.
    fn set_and_maintain_last(&mut self, second: usize, value: U) {
        self.expand(second, self.last());
        self.data[second].set(value);
    }

    /// Returns a value for a given second.
    fn get(&self, second: usize) -> T {
        match self.data.get(second) {
            Some(value) => value.clone(),
            None => T::initial_value(),
        }
    }

    /// Returns the last value in the time series or initial value if the time
    /// series is empty.
    fn last(&self) -> T {
        match self.data.last() {
            Some(last) => last.clone(),
            None => T::initial_value(),
        }
    }

    /// Gets time series suitable for usage in HTML graphs (generally each value
    /// becomes some kind of a scalar).
    fn get_graph_data(&self) -> Vec<U> {
        self.data
            .iter()
            .map(|value| value.get_graph_value())
            .collect::<Vec<_>>()
    }

    /// Expands vectors that collect per-second data for HTML report graphs with a
    /// default value.
    ///
    /// We need to do that since we don't know for how long the load test will run
    /// and we can't initialize these vectors at the beginning. It is also
    /// better to do it as we go to save memory.
    fn expand(&mut self, second: usize, initial: T) {
        if self.data.len() <= second {
            for _ in 0..(second - self.data.len() + 1) {
                self.data.push(initial.clone());
            }
        };
    }
}

/// Defines a single value in a TimeSeries.
pub trait TimeSeriesValue<T, U> {
    /// Initial ("zero") value.
    fn initial_value() -> T;
    /// Adds the given value to the current value.
    fn add(&mut self, value: U);
    /// Sets the value (and drops existing one if present).
    fn set(&mut self, value: U);
    /// Merges (adds) another TimeSeriesValue.
    fn merge(&mut self, other: &T);
    /// Gets representation of the value suitable for HTML graphs (generally a scalar).
    fn get_graph_value(&self) -> U;
}

impl TimeSeriesValue<usize, usize> for usize {
    fn initial_value() -> usize {
        0
    }
    fn add(&mut self, value: usize) {
        *self += value;
    }
    fn set(&mut self, value: usize) {
        *self = value;
    }
    fn merge(&mut self, other: &usize) {
        *self += *other;
    }
    fn get_graph_value(&self) -> usize {
        *self
    }
}

impl TimeSeriesValue<u32, u32> for u32 {
    fn initial_value() -> u32 {
        0
    }
    fn add(&mut self, value: u32) {
        *self += value;
    }
    fn set(&mut self, value: u32) {
        *self = value;
    }
    fn merge(&mut self, other: &u32) {
        *self += *other;
    }
    fn get_graph_value(&self) -> u32 {
        *self
    }
}

impl TimeSeriesValue<MovingAverage, f32> for MovingAverage {
    fn initial_value() -> MovingAverage {
        MovingAverage::new()
    }
    fn add(&mut self, value: f32) {
        self.add_item(value);
    }
    fn set(&mut self, _: f32) {
        panic!("TimeSeriesValue::set() is not supported for MovingAverage");
    }
    fn merge(&mut self, other: &MovingAverage) {
        let total_count = self.count + other.count;
        self.average = self.average * (self.count as f32 / total_count as f32)
            + other.average * (other.count as f32 / total_count as f32);
        self.count = total_count;
    }
    fn get_graph_value(&self) -> f32 {
        self.average
    }
}

/// Data structure to maintain moving averages.
///
/// It will maintain the current average and the number of data items that
/// were used to compute it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MovingAverage {
    /// Number of data items that were used to compute the current average.
    count: u32,
    /// Current average.
    average: f32,
}

impl MovingAverage {
    /// Create a new MovingAverage object.
    fn new() -> Self {
        MovingAverage {
            count: 0,
            average: 0.,
        }
    }

    /// Adds a new data item and calculates the new average.
    fn add_item(&mut self, item: f32) {
        self.count += 1;
        self.average += (item as f32 - self.average) / self.count as f32;
    }
}

impl Default for MovingAverage {
    /// Creates an empty moving average.
    fn default() -> MovingAverage {
        MovingAverage::new()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_time_setters() {
        let mut graph = GraphData::new();
        assert_eq!(graph.starting, None);
        assert_eq!(graph.started, None);
        assert_eq!(graph.stopping, None);
        assert_eq!(graph.stopped, None);

        graph.set_starting(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23));
        assert_eq!(
            graph.starting,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23))
        );
        assert_eq!(graph.started, None);
        assert_eq!(graph.stopping, None);
        assert_eq!(graph.stopped, None);

        graph.set_started(Utc.ymd(2021, 12, 14).and_hms(15, 12, 24));
        assert_eq!(
            graph.starting,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23))
        );
        assert_eq!(
            graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 24))
        );
        assert_eq!(graph.stopping, None);
        assert_eq!(graph.stopped, None);

        graph.set_stopping(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25));
        assert_eq!(
            graph.starting,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23))
        );
        assert_eq!(
            graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 24))
        );
        assert_eq!(
            graph.stopping,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25))
        );
        assert_eq!(graph.stopped, None);

        graph.set_stopped(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26));
        assert_eq!(
            graph.starting,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23))
        );
        assert_eq!(
            graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 24))
        );
        assert_eq!(
            graph.stopping,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25))
        );
        assert_eq!(
            graph.stopped,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26))
        );
    }

    #[test]
    fn test_graph_setters() {
        let mut graph = GraphData::new();
        graph.requests_per_second.insert(
            "GET /".to_string(),
            TimeSeries {
                data: vec![123, 234, 345, 456, 567],
                phantom: PhantomData,
            },
        );
        graph.users_per_second = TimeSeries {
            data: vec![345, 456, 567, 123, 234],
            phantom: PhantomData,
        };
        graph.average_response_time_per_second.insert(
            "GET /".to_string(),
            TimeSeries {
                data: vec![
                    MovingAverage {
                        count: 123,
                        average: 1.23,
                    },
                    MovingAverage {
                        count: 234,
                        average: 2.34,
                    },
                    MovingAverage {
                        count: 345,
                        average: 3.45,
                    },
                    MovingAverage {
                        count: 456,
                        average: 4.56,
                    },
                    MovingAverage {
                        count: 567,
                        average: 5.67,
                    },
                ],
                phantom: PhantomData,
            },
        );
        graph.tasks_per_second = TimeSeries {
            data: vec![345, 123, 234, 456, 567],
            phantom: PhantomData,
        };

        graph.errors_per_second.insert(
            "GET /".to_string(),
            TimeSeries {
                data: vec![567, 123, 234, 345, 456],
                phantom: PhantomData,
            },
        );
        graph.starting = Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23));
        graph.started = Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25));
        graph.stopping = Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26));
        graph.stopped = Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 27));

        let rps_graph = graph.get_requests_per_second_graph();
        let expected_time_series: TimeSeries<u32, u32> = TimeSeries {
            data: vec![123, 234, 345, 456, 567],
            phantom: PhantomData,
        };
        assert_eq!(
            rps_graph.data.get("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(rps_graph.html_id, "graph-rps");
        assert_eq!(rps_graph.y_axis_label, "Requests #");
        assert_eq!(
            rps_graph.starting,
            Utc.ymd(2021, 12, 14).and_hms(15, 12, 23)
        );
        assert_eq!(
            rps_graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25))
        );
        assert_eq!(
            rps_graph.stopping,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26))
        );
        assert_eq!(
            rps_graph.stopped,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 27))
        );

        let users_graph = graph.get_active_users_graph();
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            data: vec![345, 456, 567, 123, 234],
            phantom: PhantomData,
        };
        assert_eq!(
            users_graph.data.get("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(users_graph.html_id, "graph-active-users");
        assert_eq!(users_graph.y_axis_label, "Active users #");
        assert_eq!(
            users_graph.starting,
            Utc.ymd(2021, 12, 14).and_hms(15, 12, 23)
        );
        assert_eq!(
            users_graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25))
        );
        assert_eq!(
            users_graph.stopping,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26))
        );
        assert_eq!(
            users_graph.stopped,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 27))
        );

        let avg_rt_graph = graph.get_average_response_time_graph();
        let expected_time_series: TimeSeries<MovingAverage, f32> = TimeSeries {
            data: vec![
                MovingAverage {
                    count: 123,
                    average: 1.23,
                },
                MovingAverage {
                    count: 234,
                    average: 2.34,
                },
                MovingAverage {
                    count: 345,
                    average: 3.45,
                },
                MovingAverage {
                    count: 456,
                    average: 4.56,
                },
                MovingAverage {
                    count: 567,
                    average: 5.67,
                },
            ],
            phantom: PhantomData,
        };
        assert_eq!(
            avg_rt_graph.data.get("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(avg_rt_graph.html_id, "graph-avg-response-time");
        assert_eq!(avg_rt_graph.y_axis_label, "Response time [ms]");
        assert_eq!(
            avg_rt_graph.starting,
            Utc.ymd(2021, 12, 14).and_hms(15, 12, 23)
        );
        assert_eq!(
            avg_rt_graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25))
        );
        assert_eq!(
            avg_rt_graph.stopping,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26))
        );
        assert_eq!(
            avg_rt_graph.stopped,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 27))
        );

        let tasks_graph = graph.get_tasks_per_second_graph();
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            data: vec![345, 123, 234, 456, 567],
            phantom: PhantomData,
        };
        assert_eq!(
            tasks_graph.data.get("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(tasks_graph.html_id, "graph-tps");
        assert_eq!(tasks_graph.y_axis_label, "Tasks #");
        assert_eq!(
            tasks_graph.starting,
            Utc.ymd(2021, 12, 14).and_hms(15, 12, 23)
        );
        assert_eq!(
            tasks_graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25))
        );
        assert_eq!(
            tasks_graph.stopping,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26))
        );
        assert_eq!(
            tasks_graph.stopped,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 27))
        );

        let errors_graph = graph.get_errors_per_second_graph();
        let expected_time_series: TimeSeries<u32, u32> = TimeSeries {
            data: vec![567, 123, 234, 345, 456],
            phantom: PhantomData,
        };

        assert_eq!(
            errors_graph.data.get("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(errors_graph.html_id, "graph-eps");
        assert_eq!(errors_graph.y_axis_label, "Errors #");
        assert_eq!(
            errors_graph.starting,
            Utc.ymd(2021, 12, 14).and_hms(15, 12, 23)
        );
        assert_eq!(
            errors_graph.started,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25))
        );
        assert_eq!(
            errors_graph.stopping,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26))
        );
        assert_eq!(
            errors_graph.stopped,
            Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 27))
        );
    }

    #[test]
    fn test_record_requests_per_second() {
        // Should be initialized with empty requests per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.requests_per_second.len(), 0);

        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 1);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            3
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 3);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 1);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 5);

        graph.record_requests_per_second("GET /", 100);
        graph.record_requests_per_second("GET /", 100);
        graph.record_requests_per_second("GET /", 100);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 1);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 5);
        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 4);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 6);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[5], 1);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /").unwrap().data[second],
                0
            );
        }

        graph.record_requests_per_second("GET /user", 0);
        graph.record_requests_per_second("GET /user", 1);
        graph.record_requests_per_second("GET /user", 1);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 5);
        assert_eq!(
            graph
                .requests_per_second
                .get("GET /user")
                .unwrap()
                .data
                .len(),
            2
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[0],
            1
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[1],
            2
        );

        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 4);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 7);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[5], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /").unwrap().data[second],
                0
            );
        }

        graph.record_requests_per_second("GET /user", 100);
        graph.record_requests_per_second("GET /user", 0);
        graph.record_requests_per_second("GET /user", 1);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 1);
        assert_eq!(
            graph
                .requests_per_second
                .get("GET /user")
                .unwrap()
                .data
                .len(),
            101
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[0],
            2
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[1],
            3
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[100],
            1
        );
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /user").unwrap().data[second],
                0
            );
        }

        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 5);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 3);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 7);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[5], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /").unwrap().data[second],
                0
            );
        }
    }

    #[test]
    fn test_record_errors_per_second() {
        // Should be initialized with empty errors per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.errors_per_second.len(), 0);

        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 1);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data.len(), 3);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[0], 3);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[1], 1);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[2], 5);

        graph.record_errors_per_second("GET /", 100);
        graph.record_errors_per_second("GET /", 100);
        graph.record_errors_per_second("GET /", 100);
        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 1);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 5);
        assert_eq!(
            graph.errors_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[0], 4);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[1], 2);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[2], 6);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[5], 1);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.errors_per_second.get("GET /").unwrap().data[second],
                0
            );
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_record_average_response_time_per_second() {
        // Should be initialized with empty average response time per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.average_response_time_per_second.len(), 0);

        graph.record_average_response_time_per_second("GET /".to_string(), 0, 5);
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 4);
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 3);
        graph.record_average_response_time_per_second("GET /".to_string(), 1, 1);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 4);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 8);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 12);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 4);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 4);
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data
                .len(),
            3
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[0]
                .average,
            4.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[1]
                .average,
            1.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[2]
                .average,
            6.4
        );

        graph.record_average_response_time_per_second("GET /".to_string(), 100, 5);
        graph.record_average_response_time_per_second("GET /".to_string(), 100, 9);
        graph.record_average_response_time_per_second("GET /".to_string(), 100, 7);
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 2);
        graph.record_average_response_time_per_second("GET /".to_string(), 1, 2);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 5);
        graph.record_average_response_time_per_second("GET /".to_string(), 5, 2);
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data
                .len(),
            101
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[0]
                .average,
            3.5
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[1]
                .average,
            1.5
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[2]
                .average,
            6.166667
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[3]
                .average,
            0.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[4]
                .average,
            0.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[5]
                .average,
            2.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[100]
                .average,
            7.
        );
        for second in 6..100 {
            assert_eq!(
                graph
                    .average_response_time_per_second
                    .get("GET /")
                    .unwrap()
                    .data[second]
                    .average,
                0.
            );
        }
    }

    #[test]
    fn test_record_tasks_per_second() {
        // Should be initialized with empty tasks per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.tasks_per_second.data.len(), 0);

        graph.record_tasks_per_second(0);
        graph.record_tasks_per_second(0);
        graph.record_tasks_per_second(0);
        graph.record_tasks_per_second(1);
        graph.record_tasks_per_second(2);
        graph.record_tasks_per_second(2);
        graph.record_tasks_per_second(2);
        graph.record_tasks_per_second(2);
        graph.record_tasks_per_second(2);
        assert_eq!(graph.tasks_per_second.data.len(), 3);
        assert_eq!(graph.tasks_per_second.data[0], 3);
        assert_eq!(graph.tasks_per_second.data[1], 1);
        assert_eq!(graph.tasks_per_second.data[2], 5);

        graph.record_tasks_per_second(100);
        graph.record_tasks_per_second(100);
        graph.record_tasks_per_second(100);
        graph.record_tasks_per_second(0);
        graph.record_tasks_per_second(1);
        graph.record_tasks_per_second(2);
        graph.record_tasks_per_second(5);
        assert_eq!(graph.tasks_per_second.data.len(), 101);
        assert_eq!(graph.tasks_per_second.data[0], 4);
        assert_eq!(graph.tasks_per_second.data[1], 2);
        assert_eq!(graph.tasks_per_second.data[2], 6);
        assert_eq!(graph.tasks_per_second.data[3], 0);
        assert_eq!(graph.tasks_per_second.data[4], 0);
        assert_eq!(graph.tasks_per_second.data[5], 1);
        assert_eq!(graph.tasks_per_second.data[100], 3);
        for second in 6..100 {
            assert_eq!(graph.tasks_per_second.data[second], 0);
        }
    }

    #[test]
    fn test_record_users_per_second() {
        // Should be initialized with empty tasks per second vector.
        let mut graph = GraphData::new();
        graph.starting = Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23));
        assert_eq!(graph.users_per_second.data.len(), 0);

        graph.record_users_per_second(1, Utc.ymd(2021, 12, 14).and_hms(15, 12, 23));
        graph.record_users_per_second(1, Utc.ymd(2021, 12, 14).and_hms(15, 12, 24));
        graph.record_users_per_second(2, Utc.ymd(2021, 12, 14).and_hms(15, 12, 25));
        assert_eq!(graph.users_per_second.data.len(), 3);
        assert_eq!(graph.users_per_second.data[0], 1);
        assert_eq!(graph.users_per_second.data[1], 1);
        assert_eq!(graph.users_per_second.data[2], 2);

        graph.record_users_per_second(5, Utc.ymd(2021, 12, 14).and_hms(15, 12, 28));
        graph.record_users_per_second(10, Utc.ymd(2021, 12, 14).and_hms(15, 13, 00));
        assert_eq!(graph.users_per_second.data.len(), 38);
        assert_eq!(graph.users_per_second.data[0], 1);
        assert_eq!(graph.users_per_second.data[1], 1);
        assert_eq!(graph.users_per_second.data[2], 2);
        assert_eq!(graph.users_per_second.data[3], 2);
        assert_eq!(graph.users_per_second.data[4], 2);
        assert_eq!(graph.users_per_second.data[5], 5);
        assert_eq!(graph.users_per_second.data[37], 10);
        for second in 6..36 {
            assert_eq!(graph.users_per_second.data[second], 5);
        }
    }

    #[test]
    fn test_moving_average() {
        let mut moving_average = MovingAverage::new();
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 0,
                average: 0.
            }
        );

        moving_average.add_item(1.23);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 1,
                average: 1.23
            }
        );

        moving_average.add_item(2.34);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 2,
                average: 1.785
            }
        );

        moving_average.add_item(89.23);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 3,
                average: 30.933332
            }
        );

        moving_average.add_item(12.34);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 4,
                average: 26.285
            }
        );
    }

    #[test]
    fn test_add_timestamp_to_html_graph_data() {
        let data = vec![123, 234, 345, 456, 567];
        let graph: Graph<usize, usize> = Graph::new(
            "html_id",
            "Label",
            HashMap::new(),
            Utc.ymd(2021, 12, 14).and_hms(15, 12, 23),
            None,
            None,
            None,
        );

        assert_eq!(
            graph.add_timestamp_to_html_graph_data(&data),
            vec![
                (
                    Local
                        .timestamp(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23).timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    123
                ),
                (
                    Local
                        .timestamp(Utc.ymd(2021, 12, 14).and_hms(15, 12, 24).timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    234
                ),
                (
                    Local
                        .timestamp(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25).timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    345
                ),
                (
                    Local
                        .timestamp(Utc.ymd(2021, 12, 14).and_hms(15, 12, 26).timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    456
                ),
                (
                    Local
                        .timestamp(Utc.ymd(2021, 12, 14).and_hms(15, 12, 27).timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    567
                )
            ]
        );
    }

    fn expected_graph_html_prefix(html_id: &str, y_axis_label: &str) -> String {
        format!(
            r#"<div class="graph">
                <div id="{html_id}" style="width: 1000px; height:500px; background: white;"></div>

                <script type="text/javascript">
                    var chartDom = document.getElementById('{html_id}');
                    var myChart = echarts.init(chartDom);

                    myChart.setOption({{
                        color: ['#2c664f', '#5470c6', '#91cc75', '#fac858', '#ee6666', '#73c0de', '#3ba272', '#fc8452', '#9a60b4', '#ea7ccc'],
                        tooltip: {{ trigger: 'axis' }},
                        toolbox: {{
                            feature: {{
                                dataZoom: {{ yAxisIndex: 'none' }},
                                restore: {{}},
                                saveAsImage: {{}}
                            }}
                        }},
                        dataZoom: [
                            {{
                                type: 'inside',
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                            {{
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                        ],
                        xAxis: {{ type: 'time' }},
                        yAxis: {{
                            name: '{y_axis_label}',
                            nameLocation: 'center',
                            nameRotate: 90,
                            nameGap: 45,
                            type: 'value'
                        }},
                        
                        series: [
                            {{
                                name: 'GET /',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    itemStyle: {{ color: 'rgba(6, 6, 6, 0.10)' }},
"#,
            html_id = html_id,
            y_axis_label = y_axis_label
        )
    }

    #[test]
    fn test_graph_markup() {
        let expected_prefix = expected_graph_html_prefix("graph-rps", "Requests #");

        let data: TimeSeries<usize, usize> = TimeSeries {
            data: vec![123, 111, 99, 134],
            phantom: PhantomData,
        };

        let mut graph = HashMap::new();
        graph.insert("GET /".to_string(), data);

        let mut expected = expected_prefix.to_owned();
        expected += format!(
            r#"                                    data: [
                                        
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M")
        ).as_str();

        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                graph.clone(),
                Utc.ymd(2021, 11, 21).and_hms(21, 20, 32),
                None,
                None,
                None
            )
            .get_markup(),
            expected
        );

        let mut expected = expected_prefix.to_owned();
        expected += format!(
            r#"                                    data: [
                                        [
                                            {{
                                                name: 'Starting',
                                                xAxis: '{starting}'
                                            }},
                                            {{
                                                xAxis: '{started}'
                                            }}
                                        ],
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
            starting = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
            started = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
        ).as_str();

        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                graph.clone(),
                Utc.ymd(2021, 11, 21).and_hms(21, 20, 32),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34)),
                None,
                None
            )
            .get_markup(),
            expected
        );

        let mut expected = expected_prefix.to_owned();
        expected += format!(
            r#"                                    data: [
                                        
                                        [
                                            {{
                                                name: 'Stopping',
                                                xAxis: '{stopping}'
                                            }},
                                            {{
                                                xAxis: '{stopped}'
                                            }}
                                        ],
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
            stopping = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
            stopped = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
        ).as_str();

        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                graph.clone(),
                Utc.ymd(2021, 11, 21).and_hms(21, 20, 32),
                None,
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34))
            )
            .get_markup(),
            expected
        );

        let mut expected = expected_prefix;
        expected += format!(
            r#"                                    data: [
                                        [
                                            {{
                                                name: 'Starting',
                                                xAxis: '{starting}'
                                            }},
                                            {{
                                                xAxis: '{started}'
                                            }}
                                        ],
                                        [
                                            {{
                                                name: 'Stopping',
                                                xAxis: '{stopping}'
                                            }},
                                            {{
                                                xAxis: '{stopped}'
                                            }}
                                        ],
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
            starting = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
            started = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
            stopping = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 36).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
            stopped = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 38).timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S"),
        ).as_str();

        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                graph.clone(),
                Utc.ymd(2021, 11, 21).and_hms(21, 20, 32),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 36)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 38))
            )
            .get_markup(),
            expected
        );

        let user_data: TimeSeries<usize, usize> = TimeSeries {
            data: vec![23, 12, 44, 22],
            phantom: PhantomData,
        };
        graph.insert("GET /user".to_string(), user_data);

        let markup = Graph::new(
            "graph-rps",
            "Requests #",
            graph.clone(),
            Utc.ymd(2021, 11, 21).and_hms(21, 20, 32),
            None,
            None,
            None,
        )
        .get_markup();
        let expected_legend = r#"
                        legend: {
                            type: 'plain',
                            width: '75%',
                            data: ["Total","GET /"#;
        assert!(
            markup.contains(expected_legend),
            "legend {} not found in {}",
            expected_legend,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'Total',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    itemStyle: {{ color: 'rgba(6, 6, 6, 0.10)' }},
                                    data: [
                                        
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",146],["{data_series_prefix}:33",123],["{data_series_prefix}:34",143],["{data_series_prefix}:35",156]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /user',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",23],["{data_series_prefix}:33",12],["{data_series_prefix}:34",44],["{data_series_prefix}:35",22]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let more_data: TimeSeries<usize, usize> = TimeSeries {
            data: vec![1, 1, 1, 1],
            phantom: PhantomData,
        };
        graph.insert("GET /one".to_string(), more_data.clone());
        graph.insert("GET /two".to_string(), more_data.clone());
        graph.insert("GET /three".to_string(), more_data);

        let markup = Graph::new(
            "graph-rps",
            "Requests #",
            graph.clone(),
            Utc.ymd(2021, 11, 21).and_hms(21, 20, 32),
            None,
            None,
            None,
        )
        .get_markup();
        let expected_legend = r#"
                        legend: {
                            type: 'scroll',
                            width: '75%',
                            data: ["Total","GET /"#;
        assert!(
            markup.contains(expected_legend),
            "legend {} not found in {}",
            expected_legend,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'Total',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    itemStyle: {{ color: 'rgba(6, 6, 6, 0.10)' }},
                                    data: [
                                        
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",149],["{data_series_prefix}:33",126],["{data_series_prefix}:34",146],["{data_series_prefix}:35",159]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /user',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",23],["{data_series_prefix}:33",12],["{data_series_prefix}:34",44],["{data_series_prefix}:35",22]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /one',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",1],["{data_series_prefix}:33",1],["{data_series_prefix}:34",1],["{data_series_prefix}:35",1]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /two',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",1],["{data_series_prefix}:33",1],["{data_series_prefix}:34",1],["{data_series_prefix}:35",1]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /three',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",1],["{data_series_prefix}:33",1],["{data_series_prefix}:34",1],["{data_series_prefix}:35",1]],
                            }},"#,
            data_series_prefix = Local
                .timestamp(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32).timestamp(), 0)
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );
    }
}
