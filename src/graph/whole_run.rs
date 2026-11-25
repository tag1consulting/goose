//! The dashboard's whole run view of its line charts: requests and failures
//! per second, users and the average response time over the whole run, in
//! at most [`WHOLE_RUN_POINTS`] points each, folded as the heatmap's whole
//! run columns are.
//!
//! A run that keeps its graph data whole (a report will be written) builds
//! the view from that data when the dashboard asks for it, and keeps no
//! second copy. A dashboard only run prunes its graph data to the live
//! window; each second it prunes is folded into [`WholeRunLines`] first, so
//! the view is that fold plus the seconds the window still holds.

use super::{GraphData, MovingAverage, TimeSeries, TimeSeriesValue};
use crate::metrics::dashboard_snapshot::WholeRunLines as WholeRunLinesBody;

/// Most points a whole run line keeps, as many as the heatmap's all
/// requests columns.
pub(crate) const WHOLE_RUN_POINTS: usize = super::ALL_REQUESTS_COLUMNS;

/// A value two adjacent points of a whole run line fold into one.
pub(crate) trait Fold: Clone + Default {
    /// Fold in `later`, the value of the same or a later span.
    fn fold(&mut self, later: &Self);
}

/// A count, such as requests: summed.
impl Fold for u64 {
    fn fold(&mut self, later: &u64) {
        *self = self.saturating_add(*later);
    }
}

/// A mean weighted by request count, as the total and the count it is the
/// mean of: two fold into the mean of all their requests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Weighted {
    total: f64,
    count: u64,
}

impl Weighted {
    /// The mean of `count` values whose mean is `mean`.
    pub(crate) fn of(mean: f64, count: u64) -> Self {
        Weighted {
            total: mean * count as f64,
            count,
        }
    }

    /// The mean, 0 without values.
    pub(crate) fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total / self.count as f64
        }
    }
}

impl Fold for Weighted {
    fn fold(&mut self, later: &Weighted) {
        self.total += later.total;
        self.count = self.count.saturating_add(later.count);
    }
}

/// A level, such as users: the users series holds the count at the end of
/// each second, so a span's level is its last second's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Level(pub(crate) Option<u64>);

impl Fold for Level {
    fn fold(&mut self, later: &Level) {
        if later.0.is_some() {
            *self = *later;
        }
    }
}

/// One series over the whole run in at most [`WHOLE_RUN_POINTS`] points,
/// point `i` holding seconds `i * width` to `(i + 1) * width - 1` from the
/// start. When a second past the last point arrives, adjacent pairs fold
/// into one and the width doubles, so memory stops growing however long the
/// run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WholeRun<T> {
    width: usize,
    points: Vec<T>,
}

impl<T: Fold> Default for WholeRun<T> {
    fn default() -> Self {
        WholeRun {
            width: 1,
            points: Vec::new(),
        }
    }
}

impl<T: Fold> WholeRun<T> {
    /// Fold `value`, the value of `second`, into its point.
    pub(crate) fn add(&mut self, second: usize, value: &T) {
        while second / self.width >= WHOLE_RUN_POINTS {
            if !self.double() {
                return;
            }
        }
        let at = second / self.width;
        if self.points.len() <= at {
            self.points.resize_with(at + 1, T::default);
        }
        self.points[at].fold(value);
    }

    /// Fold adjacent pairs into one point twice as wide; false when the
    /// width cannot double.
    fn double(&mut self) -> bool {
        let Some(width) = self.width.checked_mul(2) else {
            return false;
        };
        self.width = width;
        let mut pairs = std::mem::take(&mut self.points).into_iter();
        while let Some(mut first) = pairs.next() {
            if let Some(second) = pairs.next() {
                first.fold(&second);
            }
            self.points.push(first);
        }
        true
    }

    /// Double the width until it is at least `width`.
    fn widen_to(&mut self, width: usize) {
        while self.width < width && self.double() {}
    }

    /// Seconds in each point (unit tests).
    #[cfg(test)]
    pub(crate) fn width(&self) -> usize {
        self.width
    }

    /// The points, oldest first (unit tests).
    #[cfg(test)]
    pub(crate) fn points(&self) -> &[T] {
        &self.points
    }
}

/// The whole run lines a dashboard only run folds the seconds it prunes
/// into: requests and failures summed, the average weighted by request
/// count, users by the last level of each point, and the most users of
/// any second, which a point's last level can hide.
#[derive(Clone, Debug, Default)]
pub(crate) struct WholeRunLines {
    requests: WholeRun<u64>,
    errors: WholeRun<u64>,
    average: WholeRun<Weighted>,
    users: WholeRun<Level>,
    max_users: u64,
}

impl WholeRunLines {
    /// Fold a request count the window drops.
    pub(super) fn add_requests(&mut self, second: usize, sample: &u32) {
        self.requests.add(second, &count(sample));
    }

    /// Fold a failure count the window drops.
    pub(super) fn add_errors(&mut self, second: usize, sample: &u32) {
        self.errors.add(second, &count(sample));
    }

    /// Fold an average response time the window drops.
    pub(super) fn add_average(&mut self, second: usize, sample: &MovingAverage) {
        self.average.add(second, &weighted(sample));
    }

    /// Fold a users level the window drops.
    pub(super) fn add_users(&mut self, second: usize, users: usize) {
        self.users.add(second, &Level(Some(users as u64)));
        self.max_users = self.max_users.max(users as u64);
    }
}

/// Fold every sample `series` holds, up to second `last`, into `into`, as
/// `value` makes it.
fn fold_series<T, U, F: Fold>(
    into: &mut WholeRun<F>,
    series: &TimeSeries<T, U>,
    last: usize,
    value: impl Fn(&T) -> F,
) where
    T: Clone + TimeSeriesValue<T, U>,
{
    for (offset, sample) in series.data.iter().enumerate() {
        let second = series.origin + offset;
        if second > last {
            break;
        }
        into.add(second, &value(sample));
    }
}

/// A request count as a fold.
fn count(sample: &u32) -> u64 {
    u64::from(*sample)
}

/// An average response time as a fold, weighted by its count.
fn weighted(sample: &MovingAverage) -> Weighted {
    Weighted::of(f64::from(sample.average), u64::from(sample.count))
}

impl GraphData {
    /// After a metrics reset, which restarts the rate series: the whole
    /// run lines' requests, failures and average restart too, as the
    /// report's graphs do. Users, a level over the whole run, stay.
    pub(super) fn reset_whole_run_rates(&mut self) {
        if let Some(lines) = self.whole_run.as_mut() {
            lines.requests = WholeRun::default();
            lines.errors = WholeRun::default();
            lines.average = WholeRun::default();
        }
    }

    /// The dashboard's whole run view of its line charts, through second
    /// `last`: the seconds folded so far, and every second the graph data
    /// still holds, folded into copies. Built only when asked for; a run
    /// that keeps its graph data whole has nothing folded, and builds it
    /// all from that data.
    pub(crate) fn whole_run_lines(&self, last: usize) -> WholeRunLinesBody {
        let mut lines = self.whole_run.as_deref().cloned().unwrap_or_default();
        for series in self.requests_per_second.series.values() {
            fold_series(&mut lines.requests, &series.counts, last, count);
        }
        for series in self.errors_per_second.series.values() {
            fold_series(&mut lines.errors, &series.counts, last, count);
        }
        for series in self.average_response_time_per_second.values() {
            fold_series(&mut lines.average, series, last, weighted);
        }
        // Users are a level: each second up to `last` holds the nearest
        // level known, as the live window's users do.
        if !self.users_per_second.data.is_empty() {
            for second in self.users_per_second.origin..=last {
                let users = self.users_per_second.get_held(second) as u64;
                lines.users.add(second, &Level(Some(users)));
                lines.max_users = lines.max_users.max(users);
            }
        }
        // One width for all four, so their points line up.
        let width = [
            lines.requests.width,
            lines.errors.width,
            lines.average.width,
            lines.users.width,
        ]
        .iter()
        .copied()
        .max()
        .unwrap_or(1);
        lines.requests.widen_to(width);
        lines.errors.widen_to(width);
        lines.average.widen_to(width);
        lines.users.widen_to(width);
        let len = [
            lines.requests.points.len(),
            lines.errors.points.len(),
            lines.average.points.len(),
            lines.users.points.len(),
        ]
        .iter()
        .copied()
        .max()
        .unwrap_or(0);
        // A point's rate is over the seconds it covers: the newest point may
        // cover fewer than `width`.
        let covered = |at: usize| (last + 1).saturating_sub(at * width).clamp(1, width) as f64;
        let rate = |series: &WholeRun<u64>, at: usize| {
            series.points.get(at).copied().unwrap_or(0) as f64 / covered(at)
        };
        let mut held = 0;
        WholeRunLinesBody {
            start_second: 0,
            width: width as u64,
            end_second: last as u64,
            rps: (0..len).map(|at| rate(&lines.requests, at)).collect(),
            fps: (0..len).map(|at| rate(&lines.errors, at)).collect(),
            users: (0..len)
                .map(|at| {
                    if let Some(Level(Some(users))) = lines.users.points.get(at) {
                        held = *users;
                    }
                    held
                })
                .collect(),
            avg_latency_ms: (0..len)
                .map(|at| lines.average.points.get(at).map_or(0.0, Weighted::mean))
                .collect(),
            max_users: lines.max_users,
        }
    }

    /// The whole run lines folded so far (unit tests).
    #[cfg(test)]
    pub(crate) fn whole_run_folded_for_test(&self) -> Option<&WholeRunLines> {
        self.whole_run.as_deref()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// A run of `seconds`, its counts kept at most WHOLE_RUN_POINTS wide,
    /// adjacent pairs summed, the total never changing.
    #[test]
    fn a_fold_keeps_at_most_600_points_and_every_count() {
        for seconds in [1, 599, 600, 601, 1_200, 1_201, 86_400, 1_000_003] {
            let mut run = WholeRun::<u64>::default();
            let mut total = 0;
            for second in 0..seconds {
                let value = (second % 7) as u64;
                run.add(second, &value);
                total += value;
            }
            assert!(run.points().len() <= WHOLE_RUN_POINTS, "{}", seconds);
            assert_eq!(run.points().iter().sum::<u64>(), total, "{}", seconds);
            assert!(run.width().is_power_of_two());
            assert!(seconds <= WHOLE_RUN_POINTS * run.width(), "{}", seconds);
            assert!(seconds > WHOLE_RUN_POINTS * run.width() / 2 || run.width() == 1);
            // Each point is the sum of its seconds.
            for (at, point) in run.points().iter().enumerate() {
                let expected: u64 = (at * run.width()..((at + 1) * run.width()).min(seconds))
                    .map(|second| (second % 7) as u64)
                    .sum();
                assert_eq!(*point, expected, "{} point {}", seconds, at);
            }
        }
    }

    /// A mean folds weighted by its count, a level keeps its span's last
    /// value, and a second added out of order lands in its own point.
    #[test]
    fn means_weigh_by_count_and_levels_keep_the_last() {
        let mut average = WholeRun::<Weighted>::default();
        average.add(0, &Weighted::of(10.0, 1));
        average.add(1, &Weighted::of(40.0, 3));
        average.add(1_000, &Weighted::of(5.0, 0));
        assert_eq!(average.width(), 2);
        assert_eq!(average.points()[0].mean(), 32.5);
        assert_eq!(average.points()[0].count, 4);
        assert_eq!(average.points()[500].mean(), 0.0);

        let mut users = WholeRun::<Level>::default();
        for (second, level) in [(0, 5), (1, 9), (2, 7), (3, 2)] {
            users.add(second, &Level(Some(level)));
        }
        users.add(1_500, &Level(Some(1)));
        assert_eq!(users.width(), 4);
        assert_eq!(users.points()[0], Level(Some(2)));
        assert_eq!(users.points()[1], Level(None));
        assert_eq!(users.points()[375], Level(Some(1)));

        let mut counts = WholeRun::<u64>::default();
        counts.add(5, &2);
        counts.add(3, &1);
        counts.add(5, &4);
        assert_eq!(counts.points(), &[0, 0, 0, 1, 0, 6]);
    }

    /// The whole run lines of a dashboard only run, its graph data pruned
    /// to the window as it goes, are those of the same run kept whole:
    /// totals and points alike, at most 600 of them, users by level.
    #[test]
    fn pruned_and_whole_runs_give_the_same_lines() {
        let seconds = 4_000;
        let mut whole = GraphData::new();
        let mut pruned = GraphData::new();
        for second in 0..seconds {
            for graph in [&mut whole, &mut pruned] {
                for (key, every) in [("GET /a", 1), ("GET /b", 3)] {
                    if second % every == 0 {
                        graph.record_requests_per_second(key, second);
                        graph.record_average_response_time_per_second(
                            key.to_string(),
                            second,
                            (second % 50) as u64,
                        );
                    }
                }
                if second % 11 == 0 {
                    graph.record_errors_per_second("GET /a", second);
                }
                graph.record_users_per_second(second / 40, second);
            }
            if second % 5 == 0 {
                pruned.prune_to_window(600);
            }
        }
        assert!(pruned.max_series_len_for_test() <= 605);
        assert!(pruned.whole_run_folded_for_test().is_some());
        assert!(whole.whole_run_folded_for_test().is_none());
        let last = seconds - 1;
        let a = whole.whole_run_lines(last);
        let b = pruned.whole_run_lines(last);
        assert_eq!(a, b);
        assert!(a.rps.len() <= WHOLE_RUN_POINTS);
        assert_eq!(a.width, 8);
        assert_eq!(a.rps.len(), 500);
        let requests: f64 = a.rps.iter().map(|rate| rate * 8.0).sum();
        assert_eq!(requests.round() as usize, seconds + seconds.div_ceil(3));
        let errors: f64 = a.fps.iter().map(|rate| rate * 8.0).sum();
        assert_eq!(errors.round() as usize, seconds.div_ceil(11));
        assert_eq!(a.users[0], 0);
        assert_eq!(*a.users.last().unwrap(), (last / 40) as u64);
        assert_eq!(a.max_users, (last / 40) as u64);
        assert!(a.avg_latency_ms.iter().all(|&ms| (0.0..50.0).contains(&ms)));
    }

    /// A metrics reset restarts the folded rates, not the users.
    #[test]
    fn a_reset_restarts_the_folded_rates() {
        let mut graph = GraphData::new();
        for second in 0..2_000 {
            graph.record_requests_per_second("GET /", second);
            graph.record_users_per_second(3, second);
            graph.prune_to_window(600);
        }
        graph.reset_rate_series(2_000);
        let lines = graph.whole_run_lines(1_999);
        assert!(lines.rps.iter().all(|&rate| rate == 0.0));
        assert!(lines.users.iter().all(|&users| users == 3));
    }

    /// A peak inside a point, whose last second has fewer users, is still
    /// the run's most users, folded or not.
    #[test]
    fn the_most_users_survive_a_point_that_ends_lower() {
        for prune in [false, true] {
            let mut graph = GraphData::new();
            for second in 0..2_000 {
                let users = if second == 1_001 { 20 } else { 5 };
                graph.record_users_per_second(users, second);
                if prune {
                    graph.prune_to_window(600);
                }
            }
            let lines = graph.whole_run_lines(1_999);
            assert_eq!(lines.width, 4);
            // Second 1,001 ends its point at 5 users.
            assert!(lines.users.iter().all(|&users| users == 5), "{:?}", prune);
            assert_eq!(lines.max_users, 20, "{:?}", prune);
        }
    }
}
