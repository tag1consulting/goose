use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub(crate) struct GraphData {
    /// Whether to include startup data into the generated graphs.
    include_startup: bool,
    /// Tracks when the load test first started with an optional system timestamp.
    starting: Option<DateTime<Utc>>,
    /// Tracks when all [`GooseUser`](../goose/struct.GooseUser.html) threads fully
    /// started with an optional system timestamp.
    started: Option<DateTime<Utc>>,
    /// Tracks when the load test first began stopping with an optional system timestamp.
    stopping: Option<DateTime<Utc>>,
    /// Tracks when the load test stopped with an optional system timestamp.
    stopped: Option<DateTime<Utc>>,
    /// Counts requests per second. Each element of the vector represents one second.
    requests_per_second: Vec<u32>,
    /// Counts errors per second. Each element of the vector represents one second.
    errors_per_second: Vec<u32>,
    /// Maintains average response time per second. Each element of the vector represents one second.
    average_response_time_per_second: Vec<MovingAverage>,
    /// Number of tasks at the end of each second of the test. Each element of the vector
    /// represents one second.
    tasks_per_second: Vec<usize>,
    /// Number of users at the end of each second of the test. Each element of the vector
    /// represents one second.
    users_per_second: Vec<usize>,
}

impl GraphData {
    /// Create a new GraphData object.
    pub(crate) fn new(include_startup: bool) -> Self {
        trace!("new graph");
        GraphData {
            include_startup,
            starting: None,
            started: None,
            stopping: None,
            stopped: None,
            requests_per_second: Vec::new(),
            errors_per_second: Vec::new(),
            average_response_time_per_second: Vec::new(),
            tasks_per_second: Vec::new(),
            users_per_second: Vec::new(),
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
    pub(crate) fn record_requests_per_second(&mut self, second: usize) {
        expand_per_second_metric_array(&mut self.requests_per_second, second, 0);
        self.requests_per_second[second] += 1;

        debug!(
            "incremented second {} for requests per second counter: {}",
            second, self.requests_per_second[second]
        );
    }

    /// Record errors per second metric.
    pub(crate) fn record_errors_per_second(&mut self, second: usize) {
        expand_per_second_metric_array(&mut self.errors_per_second, second, 0);
        self.errors_per_second[second] += 1;

        debug!(
            "incremented second {} for errors per second counter: {}",
            second, self.errors_per_second[second]
        );
    }

    /// Record average response time per second metric.
    pub(crate) fn record_average_response_time_per_second(
        &mut self,
        second: usize,
        response_time: u64,
    ) {
        expand_per_second_metric_array(
            &mut self.average_response_time_per_second,
            second,
            MovingAverage::new(),
        );
        self.average_response_time_per_second[second].add_item(response_time as f32);

        debug!(
            "updated second {} for average response time per second: {}",
            second, self.average_response_time_per_second[second].average
        );
    }

    /// Record tasks per second metric.
    pub(crate) fn record_tasks_per_second(&mut self, second: usize) {
        expand_per_second_metric_array(&mut self.tasks_per_second, second, 0);
        self.tasks_per_second[second] += 1;

        debug!(
            "incremented second {} for tasks per second counter: {}",
            second, self.tasks_per_second[second]
        );
    }

    /// Records number of users for a current second.
    ///
    /// This is called from [`GooseAttack::sync_metrics()`] and the data
    /// collected is used to display users graph on the HTML report.
    pub(crate) fn record_users_per_second(&mut self, users: usize) {
        if let Some(starting) = self.starting {
            let second = (Utc::now().timestamp() - starting.timestamp()) as usize;

            let last_user_count = match self.users_per_second.last() {
                Some(last) => *last,
                None => 0,
            };
            expand_per_second_metric_array(&mut self.users_per_second, second, last_user_count);
            self.users_per_second[second] = users;
        }
    }

    /// Generate active users graph.
    pub(crate) fn get_active_users_graph(&self) -> Graph<usize> {
        Graph::new(
            "graph-active-users",
            "Active users #",
            self.add_timestamp_to_html_graph_data(&self.users_per_second),
            self.starting,
            self.started,
            self.stopping,
            self.stopped,
        )
    }

    /// Generate requests per second graph.
    pub(crate) fn get_requests_per_second_graph(&self) -> Graph<u32> {
        Graph::new(
            "graph-rps",
            "Requests #",
            self.add_timestamp_to_html_graph_data(&self.requests_per_second),
            self.starting,
            self.started,
            self.stopping,
            self.stopped,
        )
    }

    /// Generate average response time graph.
    pub(crate) fn get_average_response_time_graph(&self) -> Graph<u32> {
        let response_times = self
            .average_response_time_per_second
            .iter()
            .map(|moving_average| moving_average.average as u32)
            .collect::<Vec<_>>();

        Graph::new(
            "graph-avg-response-time",
            "Response time [ms]",
            self.add_timestamp_to_html_graph_data(&response_times),
            self.starting,
            self.started,
            self.stopping,
            self.stopped,
        )
    }

    /// Generate active tasks graph.
    pub(crate) fn get_tasks_per_second_graph(&self) -> Graph<usize> {
        Graph::new(
            "graph-tps",
            "Tasks #",
            self.add_timestamp_to_html_graph_data(&self.tasks_per_second),
            self.starting,
            self.started,
            self.stopping,
            self.stopped,
        )
    }

    /// Generate errors per second graph.
    pub(crate) fn get_errors_per_second_graph(&self) -> Graph<u32> {
        Graph::new(
            "graph-eps",
            "Errors #",
            self.add_timestamp_to_html_graph_data(&self.errors_per_second),
            self.starting,
            self.started,
            self.stopping,
            self.stopped,
        )
    }

    /// Adds timestamps to the graph data series to ensure correct time display on x axis.
    ///
    /// Will take a vector of (generally numerical) values and convert them into tuples where
    /// the second element will be the data point and the first element will be formatted time
    /// it belongs to.
    fn add_timestamp_to_html_graph_data<T: Copy>(&self, data: &[T]) -> Vec<(String, T)> {
        // TODO properly handle the date Option timestamps.
        data.iter()
            .enumerate()
            .filter(|(second, _)| {
                if self.include_startup {
                    true
                } else {
                    *second as i64 + self.starting.unwrap().timestamp()
                        >= self.started.unwrap().timestamp()
                }
            })
            .map(|(second, &count)| {
                (
                    Local
                        .timestamp(second as i64 + self.starting.unwrap().timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    count,
                )
            })
            .collect::<Vec<_>>()
    }
}

/// Expands vectors that collect per-second data for HTML report graphs with a
/// default value.
///
/// We need to do that since we don't know for how long the load test will run
/// and we can't initialize these vectors at the beginning. It is also
/// better to do it as we go to save memory.
fn expand_per_second_metric_array<T: Clone>(data: &mut Vec<T>, second: usize, initial: T) {
    // Each element in per second metric vectors (self.requests_per_second,
    // self.errors_per_second, ...) is counted for a given second since the start
    // of the test. Since we don't know how long the test will at the beginning
    // we need to push new elements (second counters) as the test is running.
    if data.len() <= second {
        for _ in 0..(second - data.len() + 1) {
            data.push(initial.clone());
        }
    };
}

/// Defines the HTML graph data.
#[derive(Debug)]
// TODO why does this need to be pub (instead pub(crate)) in order for stuff on
// report.rs to not complain?
pub struct Graph<'a, T: Serialize> {
    html_id: &'a str,
    y_axis_label: &'a str,
    data: Vec<(String, T)>,
    starting: Option<DateTime<Utc>>,
    started: Option<DateTime<Utc>>,
    stopping: Option<DateTime<Utc>>,
    stopped: Option<DateTime<Utc>>,
}

impl<'a, T: Serialize> Graph<'a, T> {
    /// Creates a new Graph object.
    fn new(
        html_id: &'a str,
        y_axis_label: &'a str,
        data: Vec<(String, T)>,
        starting: Option<DateTime<Utc>>,
        started: Option<DateTime<Utc>>,
        stopping: Option<DateTime<Utc>>,
        stopped: Option<DateTime<Utc>>,
    ) -> Graph<'a, T> {
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

        let starting_area = if self.starting.is_some() && self.started.is_some() {
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
                starting = self.starting.unwrap().format(datetime_format),
                started = self.started.unwrap().format(datetime_format),
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
                stopping = self.stopping.unwrap().format(datetime_format),
                stopped = self.stopped.unwrap().format(datetime_format),
            )
        } else {
            "".to_string()
        };

        format!(
            r#"<div class="graph">
                <div id="{html_id}" style="width: 1000px; height:500px; background: white;"></div>

                <script type="text/javascript">
                    var chartDom = document.getElementById('{html_id}');
                    var myChart = echarts.init(chartDom);

                    myChart.setOption({{
                        color: ['#2c664f'],
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
                                data: {values},
                            }}
                        ]
                    }});
                </script>
            </div>"#,
            html_id = self.html_id,
            values = json!(self.data),
            starting_area = starting_area,
            stopping_area = stopping_area,
            y_axis_label = self.y_axis_label,
        )
    }
}

/// Data structure to maintain moving averages.
///
/// It will maintain the current average and the number of data items that
/// were used to compute it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MovingAverage {
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
    fn test_record_requests_per_second() {
        // Should be initialized with empty requests per second vector.
        let mut graph = GraphData::new(false);
        assert_eq!(graph.requests_per_second.len(), 0);

        graph.record_requests_per_second(0);
        graph.record_requests_per_second(0);
        graph.record_requests_per_second(0);
        graph.record_requests_per_second(1);
        graph.record_requests_per_second(2);
        graph.record_requests_per_second(2);
        graph.record_requests_per_second(2);
        graph.record_requests_per_second(2);
        graph.record_requests_per_second(2);
        assert_eq!(graph.requests_per_second.len(), 3);
        assert_eq!(graph.requests_per_second[0], 3);
        assert_eq!(graph.requests_per_second[1], 1);
        assert_eq!(graph.requests_per_second[2], 5);

        graph.record_requests_per_second(100);
        graph.record_requests_per_second(100);
        graph.record_requests_per_second(100);
        graph.record_requests_per_second(0);
        graph.record_requests_per_second(1);
        graph.record_requests_per_second(2);
        graph.record_requests_per_second(5);
        assert_eq!(graph.requests_per_second.len(), 101);
        assert_eq!(graph.requests_per_second[0], 4);
        assert_eq!(graph.requests_per_second[1], 2);
        assert_eq!(graph.requests_per_second[2], 6);
        assert_eq!(graph.requests_per_second[3], 0);
        assert_eq!(graph.requests_per_second[4], 0);
        assert_eq!(graph.requests_per_second[5], 1);
        assert_eq!(graph.requests_per_second[100], 3);
        for second in 6..100 {
            assert_eq!(graph.requests_per_second[second], 0);
        }
    }

    #[test]
    fn test_record_errors_per_second() {
        // Should be initialized with empty errors per second vector.
        let mut graph = GraphData::new(false);
        assert_eq!(graph.errors_per_second.len(), 0);

        graph.record_errors_per_second(0);
        graph.record_errors_per_second(0);
        graph.record_errors_per_second(0);
        graph.record_errors_per_second(1);
        graph.record_errors_per_second(2);
        graph.record_errors_per_second(2);
        graph.record_errors_per_second(2);
        graph.record_errors_per_second(2);
        graph.record_errors_per_second(2);
        assert_eq!(graph.errors_per_second.len(), 3);
        assert_eq!(graph.errors_per_second[0], 3);
        assert_eq!(graph.errors_per_second[1], 1);
        assert_eq!(graph.errors_per_second[2], 5);

        graph.record_errors_per_second(100);
        graph.record_errors_per_second(100);
        graph.record_errors_per_second(100);
        graph.record_errors_per_second(0);
        graph.record_errors_per_second(1);
        graph.record_errors_per_second(2);
        graph.record_errors_per_second(5);
        assert_eq!(graph.errors_per_second.len(), 101);
        assert_eq!(graph.errors_per_second[0], 4);
        assert_eq!(graph.errors_per_second[1], 2);
        assert_eq!(graph.errors_per_second[2], 6);
        assert_eq!(graph.errors_per_second[3], 0);
        assert_eq!(graph.errors_per_second[4], 0);
        assert_eq!(graph.errors_per_second[5], 1);
        assert_eq!(graph.errors_per_second[100], 3);
        for second in 6..100 {
            assert_eq!(graph.errors_per_second[second], 0);
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_record_average_response_time_per_second() {
        // Should be initialized with empty errors per second vector.
        let mut graph = GraphData::new(false);
        assert_eq!(graph.average_response_time_per_second.len(), 0);

        graph.record_average_response_time_per_second(0, 5);
        graph.record_average_response_time_per_second(0, 4);
        graph.record_average_response_time_per_second(0, 3);
        graph.record_average_response_time_per_second(1, 1);
        graph.record_average_response_time_per_second(2, 4);
        graph.record_average_response_time_per_second(2, 8);
        graph.record_average_response_time_per_second(2, 12);
        graph.record_average_response_time_per_second(2, 4);
        graph.record_average_response_time_per_second(2, 4);
        assert_eq!(graph.average_response_time_per_second.len(), 3);
        assert_eq!(graph.average_response_time_per_second[0].average, 4.);
        assert_eq!(graph.average_response_time_per_second[1].average, 1.);
        assert_eq!(graph.average_response_time_per_second[2].average, 6.4);

        graph.record_average_response_time_per_second(100, 5);
        graph.record_average_response_time_per_second(100, 9);
        graph.record_average_response_time_per_second(100, 7);
        graph.record_average_response_time_per_second(0, 2);
        graph.record_average_response_time_per_second(1, 2);
        graph.record_average_response_time_per_second(2, 5);
        graph.record_average_response_time_per_second(5, 2);
        assert_eq!(graph.average_response_time_per_second.len(), 101);
        assert_eq!(graph.average_response_time_per_second[0].average, 3.5);
        assert_eq!(graph.average_response_time_per_second[1].average, 1.5);
        assert_eq!(graph.average_response_time_per_second[2].average, 6.166667);
        assert_eq!(graph.average_response_time_per_second[3].average, 0.);
        assert_eq!(graph.average_response_time_per_second[4].average, 0.);
        assert_eq!(graph.average_response_time_per_second[5].average, 2.);
        assert_eq!(graph.average_response_time_per_second[100].average, 7.);
        for second in 6..100 {
            assert_eq!(graph.average_response_time_per_second[second].average, 0.);
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
        let mut graph = GraphData::new(false);
        let data = vec![123, 234, 345, 456, 567];

        graph.starting = Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 23));
        graph.started = Some(Utc.ymd(2021, 12, 14).and_hms(15, 12, 25));
        graph.include_startup = false;
        assert_eq!(
            graph.add_timestamp_to_html_graph_data(&data),
            vec![
                ("2021-12-14 15:12:25".to_string(), 345),
                ("2021-12-14 15:12:26".to_string(), 456),
                ("2021-12-14 15:12:27".to_string(), 567)
            ]
        );

        graph.include_startup = true;
        assert_eq!(
            graph.add_timestamp_to_html_graph_data(&data),
            vec![
                ("2021-12-14 15:12:23".to_string(), 123),
                ("2021-12-14 15:12:24".to_string(), 234),
                ("2021-12-14 15:12:25".to_string(), 345),
                ("2021-12-14 15:12:26".to_string(), 456),
                ("2021-12-14 15:12:27".to_string(), 567)
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
                        color: ['#2c664f'],
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

        let data = vec![
            ("2021-11-21 21:20:32".to_string(), 123),
            ("2021-11-21 21:20:33".to_string(), 111),
            ("2021-11-21 21:20:34".to_string(), 99),
            ("2021-11-21 21:20:35".to_string(), 134),
        ];

        let mut expected = expected_prefix.to_owned();
        expected.push_str(r#"                                    data: [
                                        
                                        
                                    ]
                                },
                                data: [["2021-11-21 21:20:32",123],["2021-11-21 21:20:33",111],["2021-11-21 21:20:34",99],["2021-11-21 21:20:35",134]],
                            }
                        ]
                    });
                </script>
            </div>"#
        );
        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                data.clone(),
                None,
                None,
                None,
                None
            )
            .get_markup(),
            expected
        );

        let mut expected = expected_prefix.to_owned();
        expected.push_str(r#"                                    data: [
                                        [
                    {
                        name: 'Starting',
                        xAxis: '2021-11-21 21:20:32'
                    },
                    {
                        xAxis: '2021-11-21 21:20:34'
                    }
                ],
                                        
                                    ]
                                },
                                data: [["2021-11-21 21:20:32",123],["2021-11-21 21:20:33",111],["2021-11-21 21:20:34",99],["2021-11-21 21:20:35",134]],
                            }
                        ]
                    });
                </script>
            </div>"#
        );
        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                data.clone(),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34)),
                None,
                None
            )
            .get_markup(),
            expected
        );

        let mut expected = expected_prefix.to_owned();
        expected.push_str(r#"                                    data: [
                                        
                                        [
                    {
                        name: 'Stopping',
                        xAxis: '2021-11-21 21:20:32'
                    },
                    {
                        xAxis: '2021-11-21 21:20:34'
                    }
                ],
                                    ]
                                },
                                data: [["2021-11-21 21:20:32",123],["2021-11-21 21:20:33",111],["2021-11-21 21:20:34",99],["2021-11-21 21:20:35",134]],
                            }
                        ]
                    });
                </script>
            </div>"#
        );
        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                data.clone(),
                None,
                None,
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34))
            )
            .get_markup(),
            expected
        );

        let mut expected = expected_prefix;
        expected.push_str(r#"                                    data: [
                                        [
                    {
                        name: 'Starting',
                        xAxis: '2021-11-21 21:20:32'
                    },
                    {
                        xAxis: '2021-11-21 21:20:34'
                    }
                ],
                                        [
                    {
                        name: 'Stopping',
                        xAxis: '2021-11-21 21:20:36'
                    },
                    {
                        xAxis: '2021-11-21 21:20:38'
                    }
                ],
                                    ]
                                },
                                data: [["2021-11-21 21:20:32",123],["2021-11-21 21:20:33",111],["2021-11-21 21:20:34",99],["2021-11-21 21:20:35",134]],
                            }
                        ]
                    });
                </script>
            </div>"#
        );
        assert_eq!(
            Graph::new(
                "graph-rps",
                "Requests #",
                data,
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 32)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 34)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 36)),
                Some(Utc.ymd(2021, 11, 21).and_hms(21, 20, 38))
            )
            .get_markup(),
            expected
        );
    }
}
