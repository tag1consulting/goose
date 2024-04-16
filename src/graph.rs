//! Optional graph data collected during the load tests.
//!
//! If the HTML report is enabled the graph data will be collected and stored in
//! the [`GraphData`] structure during the load test. When the report is written
//! this data is converted into [`Graph`] structures and HTML markup is generated
//! based on them.

use crate::test_plan::{TestPlanHistory, TestPlanStepAction};
use chrono::prelude::*;
use itertools::Itertools;
use serde::Serialize;
use serde_json::json;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Write;
use std::marker::PhantomData;

#[derive(Clone)]
struct ItemsPerSecond(HashMap<String, TimeSeries<u32, u32>>);

impl ItemsPerSecond {
    fn new() -> ItemsPerSecond {
        ItemsPerSecond(Default::default())
    }

    #[inline(always)]
    fn initialize_or_increment(&mut self, key: &str, second: usize, value: u32) -> u32 {
        if !self.contains_key(key) {
            self.insert(key, TimeSeries::new());
        }
        let data = self.0.get_mut(key).unwrap();
        data.increase_value(second, value);
        data.get(second)
    }

    #[inline(always)]
    fn contains_key(&mut self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    #[inline(always)]
    fn insert(&mut self, key: &str, time_series: TimeSeries<u32, u32>) {
        self.0.insert(key.to_string(), time_series);
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn get(&self, key: &str) -> Option<TimeSeries<u32, u32>> {
        self.0.get(key).cloned()
    }

    #[inline(always)]
    fn get_map(&self) -> HashMap<String, TimeSeries<u32, u32>> {
        self.0.clone()
    }
}

/// Used to collect graph data during a load test.
pub(crate) struct GraphData {
    /// Counts requests per second for each request type.
    requests_per_second: ItemsPerSecond,
    /// Counts errors per second.
    errors_per_second: ItemsPerSecond,
    /// Maintains average response time per second.
    average_response_time_per_second: HashMap<String, TimeSeries<MovingAverage, f32>>,
    /// Number of transactions at the end of each second of the test.
    transactions_per_second: TimeSeries<usize, usize>,
    /// Number of scenarios at the end of each second of the test.
    scenarios_per_second: TimeSeries<usize, usize>,
    /// Number of users at the end of each second of the test.
    users_per_second: TimeSeries<usize, usize>,
}

impl GraphData {
    /// Create a new GraphData object.
    pub(crate) fn new() -> Self {
        trace!("new graph");
        GraphData {
            requests_per_second: ItemsPerSecond::new(),
            errors_per_second: ItemsPerSecond::new(),
            average_response_time_per_second: HashMap::new(),
            transactions_per_second: TimeSeries::new(),
            scenarios_per_second: TimeSeries::new(),
            users_per_second: TimeSeries::new(),
        }
    }

    /// Record requests per second metric.
    pub(crate) fn record_requests_per_second(&mut self, key: &str, second: usize) {
        let value = self
            .requests_per_second
            .initialize_or_increment(key, second, 1);
        debug!(
            "incremented second {} for requests per second counter: {}",
            second, value
        );
    }

    /// Record errors per second metric.
    pub(crate) fn record_errors_per_second(&mut self, key: &str, second: usize) {
        let value = self
            .errors_per_second
            .initialize_or_increment(key, second, 1);

        debug!(
            "incremented second {} for errors per second counter: {}",
            second, value
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
        data.increase_value(second, response_time as f32);

        debug!(
            "updated second {} for average response time per second: {}",
            second,
            data.get(second).average
        );
    }

    /// Record transactions per second metric.
    pub(crate) fn record_transactions_per_second(&mut self, second: usize) {
        self.transactions_per_second.increase_value(second, 1);

        debug!(
            "incremented second {} for transactions per second counter: {}",
            second,
            self.transactions_per_second.get(second)
        );
    }

    /// Record scenarios per second metric.
    pub(crate) fn record_scenarios_per_second(&mut self, second: usize) {
        self.scenarios_per_second.increase_value(second, 1);

        debug!(
            "incremented second {} for scenarios per second counter: {}",
            second,
            self.scenarios_per_second.get(second)
        );
    }

    /// Records number of users for a current second.
    pub(crate) fn record_users_per_second(&mut self, users: usize, second: usize) {
        self.users_per_second.set_and_maintain_last(second, users);
    }

    /// Generate active users graph.
    pub(crate) fn get_active_users_graph(&self, granular_data: bool) -> Graph<usize, usize> {
        self.create_graph_from_single_data(
            "graph-active-users",
            "Active users #",
            granular_data,
            self.users_per_second.clone(),
        )
    }

    /// Generate requests per second graph.
    pub(crate) fn get_requests_per_second_graph(&self, granular_data: bool) -> Graph<u32, u32> {
        self.create_graph_from_data(
            "graph-rps",
            "Requests #",
            granular_data,
            self.requests_per_second.get_map(),
        )
    }

    /// Generate average response time graph.
    pub(crate) fn get_average_response_time_graph(
        &self,
        granular_data: bool,
    ) -> Graph<MovingAverage, f32> {
        self.create_graph_from_data(
            "graph-avg-response-time",
            "Response time [ms]",
            granular_data,
            self.average_response_time_per_second.clone(),
        )
    }

    /// Generate active transactions graph.
    pub(crate) fn get_transactions_per_second_graph(
        &self,
        granular_data: bool,
    ) -> Graph<usize, usize> {
        self.create_graph_from_single_data(
            "graph-tps",
            "Transactions #",
            granular_data,
            self.transactions_per_second.clone(),
        )
    }

    /// Generate active scenarios graph.
    pub(crate) fn get_scenarios_per_second_graph(
        &self,
        granular_data: bool,
    ) -> Graph<usize, usize> {
        self.create_graph_from_single_data(
            "graph-sps",
            "Scenarios #",
            granular_data,
            self.scenarios_per_second.clone(),
        )
    }

    /// Generate errors per second graph.
    pub(crate) fn get_errors_per_second_graph(&self, granular_data: bool) -> Graph<u32, u32> {
        self.create_graph_from_data(
            "graph-eps",
            "Errors #",
            granular_data,
            self.errors_per_second.get_map(),
        )
    }

    /// Creates a Graph from granular data.
    fn create_graph_from_data<
        'a,
        T: Clone + TimeSeriesValue<T, U>,
        U: Serialize + Copy + PartialEq + PartialOrd,
    >(
        &self,
        html_id: &'a str,
        y_axis_label: &'a str,
        granular_data: bool,
        data: HashMap<String, TimeSeries<T, U>>,
    ) -> Graph<'a, T, U> {
        Graph::new(html_id, y_axis_label, granular_data, data)
    }

    /// Creates a Graph from single (just total numbers, not granular) data.
    fn create_graph_from_single_data<
        'a,
        T: Clone + TimeSeriesValue<T, U>,
        U: Serialize + Copy + PartialEq + PartialOrd,
    >(
        &self,
        html_id: &'a str,
        y_axis_label: &'a str,
        granular_data: bool,
        data: TimeSeries<T, U>,
    ) -> Graph<'a, T, U> {
        let mut hash_map_data = HashMap::new();
        hash_map_data.insert("Total".to_string(), data);

        Graph::new(html_id, y_axis_label, granular_data, hash_map_data)
    }
}

/// Defines the HTML graph data.
#[derive(Debug)]
pub(crate) struct Graph<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy> {
    /// HTML ID of the graph's main wrapper.
    html_id: &'a str,
    /// Label of the y axis.
    y_axis_label: &'a str,
    /// Indicates whether the granular data should be displayed on graphs.
    granular_data: bool,
    /// Graph data.
    data: HashMap<String, TimeSeries<T, U>>,
}

impl<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy + PartialEq + PartialOrd>
    Graph<'a, T, U>
{
    /// Creates a new Graph object.
    #[allow(clippy::too_many_arguments)]
    fn new(
        html_id: &'a str,
        y_axis_label: &'a str,
        granular_data: bool,
        data: HashMap<String, TimeSeries<T, U>>,
    ) -> Graph<'a, T, U> {
        Graph {
            html_id,
            y_axis_label,
            granular_data,
            data,
        }
    }

    /// Helper function to build HTML charts powered by the
    /// [ECharts](https://echarts.apache.org) library.
    pub(crate) fn get_markup(
        self,
        history: &[TestPlanHistory],
        test_started_time: DateTime<Utc>,
    ) -> String {
        let mut steps = String::new();
        for step in history.windows(2) {
            let started = Local
                .timestamp_opt(step[0].timestamp.timestamp(), 0)
                // @TODO: Error handling
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let stopped = Local
                .timestamp_opt(step[1].timestamp.timestamp(), 0)
                // @TODO: Error handling
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            match &step[0].action {
                // For increasing show the current number of users to the new number of users.
                TestPlanStepAction::Increasing => {
                    let _ = write!(
                        steps,
                        r#"[
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{started}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ color: 'rgba(44, 102, 79, 0.05)' }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{stopped}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],"#,
                        started = started,
                        stopped = stopped,
                    );
                }
                // For decreasing show the new number of users from the current number of users.
                TestPlanStepAction::Decreasing | TestPlanStepAction::Canceling => {
                    let _ = write!(
                        steps,
                        r#"[
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{started}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ color: 'rgba(179, 65, 65, 0.05)' }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{stopped}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],"#,
                        started = started,
                        stopped = stopped,
                    );
                }
                _ => {}
            }
        }

        let mut total_values: TimeSeries<T, U> = TimeSeries::new();
        if self.data.is_empty() {
            "<!-- no data, no legend -->".to_string()
        } else {
            let (legend, main_label, main_values, other_values) = if self.data.len() > 1 {
                // If we are dealing with a metric with granular data we need to calculate totals.
                for (_, single_data) in self.data.iter() {
                    total_values.add_time_series(single_data);
                }

                // We will have multiple lines. We need to prepare the legend section on the graph
                // and create data series for all of them.
                let mut legend = vec!["Total"];

                let mut other_values = String::new();
                if self.granular_data {
                    // In order for this to sort correctly we need to flip label and time series since tuples
                    // are sorted lexicographically so we want time series to be the first element of the tuple.
                    for (sub_data, label) in self
                        .data
                        .iter()
                        .map(|(label, sub_data)| (sub_data, label))
                        .sorted()
                        .rev()
                    {
                        legend.push(label);

                        let formatted_line = format!(
                            r#"{{
                                name: '{label}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: {values},
                            }},
                        "#,
                            label = label,
                            values = json!(self.add_timestamp_to_html_graph_data(
                                &sub_data.get_graph_data(),
                                test_started_time
                            ))
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
                    ("".to_string(), "Total", &total_values, "".to_string())
                }
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
                                name: '{main_label}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    data: [
                                        {steps}
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
                main_values = json!(self.add_timestamp_to_html_graph_data(
                    &main_values.get_graph_data(),
                    test_started_time
                )),
                y_axis_label = self.y_axis_label,
                main_label = main_label,
                legend = legend,
                other_values = other_values,
                steps = steps,
            )
        }
    }

    /// Adds timestamps to the graph data series to ensure correct time display on x axis.
    ///
    /// Will take a vector of (generally numerical) values and convert them into tuples where
    /// the second element will be the data point and the first element will be formatted time
    /// it belongs to.
    fn add_timestamp_to_html_graph_data(
        &self,
        data: &[Option<U>],
        started: DateTime<Utc>,
    ) -> Vec<(String, U)> {
        data.iter()
            .enumerate()
            .map(|(second, value)| {
                (
                    Local
                        .timestamp_opt(second as i64 + started.timestamp(), 0)
                        // @TODO: Error handling
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    *value,
                )
            })
            .filter_map(|(time, data_option)| data_option.map(|data| (time, data)))
            .collect::<Vec<_>>()
    }
}

/// Data structure to represent time series data.
#[derive(Debug, Clone)]
struct TimeSeries<T: TimeSeriesValue<T, U>, U> {
    /// Time series data.
    ///
    /// Each element of the vector represents value for one second in the time series.
    data: Vec<T>,
    /// Total value of the time series (sum of all elements).
    total: T,
    /// Phantom data indicates to the compiler that the "U" generic data type has zero size.
    phantom: PhantomData<U>,
}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq + PartialOrd> Ord for TimeSeries<T, U> {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_total = self.total();
        let other_total = other.total();

        if self_total > other_total {
            Ordering::Greater
        } else if self_total < other_total {
            Ordering::Less
        } else {
            Ordering::Equal
        }
    }
}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq + PartialOrd> PartialOrd for TimeSeries<T, U> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq> Eq for TimeSeries<T, U> {}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq> PartialEq for TimeSeries<T, U> {
    fn eq(&self, other: &Self) -> bool {
        self.total() == other.total()
    }
}

impl<T: Clone + TimeSeriesValue<T, U>, U> TimeSeries<T, U> {
    /// Creates a new TimeSeries object.
    fn new() -> TimeSeries<T, U> {
        TimeSeries {
            data: Vec::new(),
            phantom: PhantomData,
            total: T::initial_value(),
        }
    }

    /// Increases the the value for a given second.
    fn increase_value(&mut self, second: usize, value: U) {
        self.expand(second, T::initial_value());
        self.data[second].increase_value(&value);
        self.total.increase_value(&value);
    }

    /// Adds another time series.
    fn add_time_series(&mut self, other: &TimeSeries<T, U>) {
        for (second, other_item) in other.data.iter().enumerate() {
            self.expand(second, T::initial_value());
            self.data.get_mut(second).unwrap().merge(other_item);
            self.total.merge(other_item);
        }
    }

    /// Sets a value for a given second and maintains last recorded value if
    /// there is a gap in the time series.
    fn set_and_maintain_last(&mut self, second: usize, value: U) {
        self.expand(second, self.last());
        self.total.increase_value(&value);
        self.data[second].set_value(value);
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
    fn get_graph_data(&self) -> Vec<Option<U>> {
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
                self.total.merge(&initial);
            }
        };
    }

    /// Gets time series total value (sum of all values).
    fn total(&self) -> U {
        self.total.get_total_value()
    }
}

/// Defines a single value in a TimeSeries.
pub trait TimeSeriesValue<T, U> {
    /// Initial ("zero") value.
    fn initial_value() -> T;
    /// Adds the given value to the current value.
    fn increase_value(&mut self, value: &U);
    /// Sets the value (and drops existing one if present).
    fn set_value(&mut self, value: U);
    /// Merges (adds) another TimeSeriesValue.
    fn merge(&mut self, other: &T);
    /// Gets representation of the value suitable for HTML graphs (generally a scalar).
    fn get_graph_value(&self) -> Option<U>;
    fn get_total_value(&self) -> U;
}

impl TimeSeriesValue<usize, usize> for usize {
    fn initial_value() -> usize {
        0
    }
    fn increase_value(&mut self, value: &usize) {
        *self += *value;
    }
    fn set_value(&mut self, value: usize) {
        *self = value;
    }
    fn merge(&mut self, other: &usize) {
        *self += *other;
    }
    fn get_graph_value(&self) -> Option<usize> {
        match *self == 0 {
            true => None,
            false => Some(*self),
        }
    }
    fn get_total_value(&self) -> usize {
        *self
    }
}

impl TimeSeriesValue<u32, u32> for u32 {
    fn initial_value() -> u32 {
        0
    }
    fn increase_value(&mut self, value: &u32) {
        *self += *value;
    }
    fn set_value(&mut self, value: u32) {
        *self = value;
    }
    fn merge(&mut self, other: &u32) {
        *self += *other;
    }
    fn get_graph_value(&self) -> Option<u32> {
        match *self == 0 {
            true => None,
            false => Some(*self),
        }
    }
    fn get_total_value(&self) -> u32 {
        *self
    }
}

impl TimeSeriesValue<MovingAverage, f32> for MovingAverage {
    fn initial_value() -> MovingAverage {
        MovingAverage::new()
    }
    fn increase_value(&mut self, value: &f32) {
        self.add_item(value);
    }
    fn set_value(&mut self, _: f32) {
        panic!("TimeSeriesValue::set() is not supported for MovingAverage");
    }
    fn merge(&mut self, other: &MovingAverage) {
        let total_count = self.count + other.count;
        if total_count == 0 {
            self.average = 0.;
        } else {
            self.average = self.average * (self.count as f32 / total_count as f32)
                + other.average * (other.count as f32 / total_count as f32);
        };
        self.count = total_count;
    }
    fn get_graph_value(&self) -> Option<f32> {
        match self.average == 0f32 {
            true => None,
            false => Some(self.average),
        }
    }
    fn get_total_value(&self) -> f32 {
        self.average
    }
}

/// Data structure to maintain moving averages.
///
/// It will maintain the current average and the number of data items that
/// were used to compute it.
#[derive(Debug, Clone, Copy)]
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
    fn add_item(&mut self, item: &f32) {
        self.count += 1;
        self.average += (*item - self.average) / self.count as f32;
    }
}

impl Default for MovingAverage {
    /// Creates an empty moving average.
    fn default() -> MovingAverage {
        MovingAverage::new()
    }
}

impl Ord for MovingAverage {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.average > other.average {
            Ordering::Greater
        } else if self.average < other.average {
            Ordering::Less
        } else {
            Ordering::Equal
        }
    }
}

impl PartialOrd for MovingAverage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for MovingAverage {
    fn eq(&self, other: &Self) -> bool {
        self.average == other.average
    }
}

impl Eq for MovingAverage {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_graph_setters() {
        let mut graph = GraphData::new();
        graph.requests_per_second.insert(
            "GET /",
            TimeSeries {
                data: vec![123, 234, 345, 456, 567],
                phantom: PhantomData,
                total: 0,
            },
        );
        graph.users_per_second = TimeSeries {
            data: vec![345, 456, 567, 123, 234],
            phantom: PhantomData,
            total: 0,
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
                total: MovingAverage {
                    count: 0,
                    average: 0.,
                },
            },
        );
        graph.transactions_per_second = TimeSeries {
            data: vec![345, 123, 234, 456, 567],
            phantom: PhantomData,
            total: 0,
        };

        graph.scenarios_per_second = TimeSeries {
            data: vec![345, 123, 234, 456, 567],
            phantom: PhantomData,
            total: 0,
        };

        graph.errors_per_second.insert(
            "GET /",
            TimeSeries {
                data: vec![567, 123, 234, 345, 456],
                phantom: PhantomData,
                total: 0,
            },
        );

        let rps_graph = graph.get_requests_per_second_graph(true);
        let expected_time_series: TimeSeries<u32, u32> = TimeSeries {
            data: vec![123, 234, 345, 456, 567],
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            rps_graph.data.get("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(rps_graph.html_id, "graph-rps");
        assert_eq!(rps_graph.y_axis_label, "Requests #");

        let users_graph = graph.get_active_users_graph(true);
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            data: vec![345, 456, 567, 123, 234],
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            users_graph.data.get("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(users_graph.html_id, "graph-active-users");
        assert_eq!(users_graph.y_axis_label, "Active users #");

        let avg_rt_graph = graph.get_average_response_time_graph(true);
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
            total: MovingAverage {
                count: 0,
                average: 0.,
            },
        };
        assert_eq!(
            avg_rt_graph.data.get("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(avg_rt_graph.html_id, "graph-avg-response-time");
        assert_eq!(avg_rt_graph.y_axis_label, "Response time [ms]");

        let transactions_graph = graph.get_transactions_per_second_graph(true);
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            data: vec![345, 123, 234, 456, 567],
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            transactions_graph.data.get("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(transactions_graph.html_id, "graph-tps");
        assert_eq!(transactions_graph.y_axis_label, "Transactions #");

        let scenarios_graph = graph.get_scenarios_per_second_graph(true);
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            data: vec![345, 123, 234, 456, 567],
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            scenarios_graph.data.get("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(scenarios_graph.html_id, "graph-sps");
        assert_eq!(scenarios_graph.y_axis_label, "Scenarios #");

        let errors_graph = graph.get_errors_per_second_graph(true);
        let expected_time_series: TimeSeries<u32, u32> = TimeSeries {
            data: vec![567, 123, 234, 345, 456],
            phantom: PhantomData,
            total: 0,
        };

        assert_eq!(
            errors_graph.data.get("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(errors_graph.html_id, "graph-eps");
        assert_eq!(errors_graph.y_axis_label, "Errors #");
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
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 9);

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
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 16);

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
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 18);
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().total(),
            3
        );

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
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 20);
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().total(),
            6
        );
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
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().total(), 9);

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
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().total(), 16);
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
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .total(),
            5.0000005
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
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .total(),
            4.8125005
        );
    }

    #[test]
    fn test_record_transactions_per_second() {
        // Should be initialized with empty transactions per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.transactions_per_second.data.len(), 0);

        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(1);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        assert_eq!(graph.transactions_per_second.data.len(), 3);
        assert_eq!(graph.transactions_per_second.data[0], 3);
        assert_eq!(graph.transactions_per_second.data[1], 1);
        assert_eq!(graph.transactions_per_second.data[2], 5);
        assert_eq!(graph.transactions_per_second.total(), 9);

        graph.record_transactions_per_second(100);
        graph.record_transactions_per_second(100);
        graph.record_transactions_per_second(100);
        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(1);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(5);
        assert_eq!(graph.transactions_per_second.data.len(), 101);
        assert_eq!(graph.transactions_per_second.data[0], 4);
        assert_eq!(graph.transactions_per_second.data[1], 2);
        assert_eq!(graph.transactions_per_second.data[2], 6);
        assert_eq!(graph.transactions_per_second.data[3], 0);
        assert_eq!(graph.transactions_per_second.data[4], 0);
        assert_eq!(graph.transactions_per_second.data[5], 1);
        assert_eq!(graph.transactions_per_second.data[100], 3);
        for second in 6..100 {
            assert_eq!(graph.transactions_per_second.data[second], 0);
        }
        assert_eq!(graph.transactions_per_second.total(), 16);
    }

    #[test]
    fn test_record_users_per_second() {
        // Should be initialized with empty transactions per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.users_per_second.data.len(), 0);

        graph.record_users_per_second(1, 0);
        graph.record_users_per_second(1, 1);
        graph.record_users_per_second(2, 2);
        assert_eq!(graph.users_per_second.data.len(), 3);
        assert_eq!(graph.users_per_second.data[0], 1);
        assert_eq!(graph.users_per_second.data[1], 1);
        assert_eq!(graph.users_per_second.data[2], 2);
        assert_eq!(graph.users_per_second.total(), 6);

        graph.record_users_per_second(5, 5);
        graph.record_users_per_second(10, 37);
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
        assert_eq!(graph.users_per_second.total(), 187);
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

        moving_average.add_item(&1.23);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 1,
                average: 1.23
            }
        );

        moving_average.add_item(&2.34);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 2,
                average: 1.785
            }
        );

        moving_average.add_item(&89.23);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 3,
                average: 30.933332
            }
        );

        moving_average.add_item(&12.34);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 4,
                average: 26.285
            }
        );
    }

    #[test]
    fn test_moving_average_cmp() {
        assert!(
            MovingAverage {
                count: 0,
                average: 0.
            } < MovingAverage {
                count: 0,
                average: 0.1,
            }
        );

        assert!(
            MovingAverage {
                count: 0,
                average: 1.1,
            } > MovingAverage {
                count: 0,
                average: 0.1,
            }
        );

        assert_eq!(
            MovingAverage {
                count: 1,
                average: 1.1,
            },
            MovingAverage {
                count: 2,
                average: 1.1,
            }
        );

        assert!(
            MovingAverage {
                count: 0,
                average: 1.1,
            } != MovingAverage {
                count: 0,
                average: 1.0,
            }
        );
    }

    #[test]
    fn test_add_timestamp_to_html_graph_data() {
        let data = vec![Some(123), Some(234), Some(345), Some(456), Some(567)];
        let graph: Graph<usize, usize> = Graph::new("html_id", "Label", true, HashMap::new());

        assert_eq!(
            graph.add_timestamp_to_html_graph_data(
                &data,
                Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 23).unwrap()
            ),
            vec![
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 23)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    123
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 24)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    234
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 25)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    345
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 26)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    456
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 27)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    567
                )
            ]
        );
    }

    fn expected_graph_html_prefix(
        html_id: &str,
        y_axis_label: &str,
        main_series_label: &str,
    ) -> String {
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
                                name: '{main_series_label}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
"#,
            html_id = html_id,
            y_axis_label = y_axis_label,
            main_series_label = main_series_label
        )
    }

    #[test]
    fn test_graph_markup() {
        let expected_prefix = expected_graph_html_prefix("graph-rps", "Requests #", "GET /");

        let data: TimeSeries<usize, usize> = TimeSeries {
            data: vec![123, 111, 99, 134],
            phantom: PhantomData,
            total: 0,
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
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M")
        ).as_str();

        assert_eq!(
            Graph::new("graph-rps", "Requests #", true, graph.clone(),).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        // It should make no difference if we disable granular graphs, since we only have one
        // request.
        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, graph.clone(),).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        let mut expected = expected_prefix;
        expected += format!(
            r#"                                    data: [
                                        [
                            {{
                                xAxis: '{increasing}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{increasing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{increasing}',
                                itemStyle: {{ color: 'rgba(44, 102, 79, 0.05)' }},
                            }},
                            {{
                                xAxis: '{decreasing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{decreasing}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{decreasing}'
                            }}
                        ],[
                            {{
                                xAxis: '{decreasing}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{decreasing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{decreasing}',
                                itemStyle: {{ color: 'rgba(179, 65, 65, 0.05)' }},
                            }},
                            {{
                                xAxis: '{cancelling}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{cancelling}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{cancelling}'
                            }}
                        ],[
                            {{
                                xAxis: '{cancelling}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{cancelling}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{cancelling}',
                                itemStyle: {{ color: 'rgba(179, 65, 65, 0.05)' }},
                            }},
                            {{
                                xAxis: '{finishing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{finishing}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{finishing}'
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
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
            increasing = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
            decreasing = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 33).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
            cancelling = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 34).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
            finishing = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 35).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
        ).as_str();

        let steps = vec![
            TestPlanHistory {
                action: TestPlanStepAction::Increasing,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap(),
                users: 123,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Decreasing,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 33).unwrap(),
                users: 123,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Canceling,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 34).unwrap(),
                users: 123,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Finished,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 35).unwrap(),
                users: 123,
            },
        ];

        assert_eq!(
            Graph::new("graph-rps", "Requests #", true, graph.clone(),).get_markup(
                &steps,
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        // It should make no difference if we disable granular graphs, since we only have one
        // request.
        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, graph.clone(),).get_markup(
                &steps,
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        let user_data: TimeSeries<usize, usize> = TimeSeries {
            data: vec![23, 12, 44, 22],
            phantom: PhantomData,
            total: 0,
        };
        graph.insert("GET /user".to_string(), user_data);

        let markup = Graph::new("graph-rps", "Requests #", true, graph.clone()).get_markup(
            &Vec::new(),
            Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap(),
        );
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
                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",146],["{data_series_prefix}:33",123],["{data_series_prefix}:34",143],["{data_series_prefix}:35",156]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
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
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
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
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        // Now same graph with granular data disabled.
        let expected_nongranular_prefix =
            expected_graph_html_prefix("graph-rps", "Requests #", "Total");
        let mut expected = expected_nongranular_prefix.to_owned();
        expected += format!(
            r#"                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",146],["{data_series_prefix}:33",123],["{data_series_prefix}:34",143],["{data_series_prefix}:35",156]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M")
        ).as_str();

        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, graph.clone(),).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        let more_data: TimeSeries<usize, usize> = TimeSeries {
            data: vec![1, 1, 1, 1],
            phantom: PhantomData,
            total: 0,
        };
        graph.insert("GET /one".to_string(), more_data.clone());
        graph.insert("GET /two".to_string(), more_data.clone());
        graph.insert("GET /three".to_string(), more_data);

        let markup = Graph::new("graph-rps", "Requests #", true, graph.clone()).get_markup(
            &Vec::new(),
            Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap(),
        );
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
                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",149],["{data_series_prefix}:33",126],["{data_series_prefix}:34",146],["{data_series_prefix}:35",159]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
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
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
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
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
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
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
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
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
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
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        // Now same graph with granular data disabled.
        let mut expected = expected_nongranular_prefix;
        expected += format!(
            r#"                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",149],["{data_series_prefix}:33",126],["{data_series_prefix}:34",146],["{data_series_prefix}:35",159]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M")
        ).as_str();

        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, graph.clone(),).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );
    }
}
