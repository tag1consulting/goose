//! What a Gaggle manager's dashboard shows of its workers: the snapshot's
//! `gaggle` section.
//!
//! Built only inside the manager's dashboard snapshot build, so only while
//! someone watches, and only from what the manager already keeps: the worker
//! health model ([`super::health`]) for every figure, flag, event and
//! sentence, the merge's per worker graph rows for the RPS lines by worker,
//! and step 3's split rules ([`super::control`]) for the control previews.
//! Nothing here judges a worker or words a sentence of its own: it reads,
//! sorts, counts and caps.

use super::control::live_throttle_shares;
use super::health::{self, GaggleHealth, WorkerHealth, WorkerState};
use super::merge::GaggleAccumulator;
use crate::metrics::dashboard_snapshot::{
    GaggleByWorker, GaggleEvents, GaggleGrid, GagglePreviews, GaggleRows, GaggleSnapshot,
    GaggleSplit, GaggleSummary, GaggleUsersChange, SeriesWindow, BY_WORKER_LINES,
    MAX_GAGGLE_EVENTS, MAX_WORKER_ROWS,
};
use crate::runs::{
    check_sentences, named, GaggleEvent, GaggleEventKind, GaggleRecord, GaggleWorkerFlag,
};
use std::collections::BTreeMap;
use std::time::Instant;

/// The most bytes of an event's text the section copies: a worker's words
/// can reach it, at up to 4 bytes a character.
const EVENT_TEXT_BYTES: usize = 160;
/// The most bytes of event texts in all; older events past it are left out.
const EVENT_TEXTS_BYTES: usize = 12 * 1024;
/// The most bytes of a lost worker's reason the section copies.
const REASON_BYTES: usize = 120;
/// The most bytes of reasons in all; a reason past it reads as `REASON_LEFT_OUT`.
const REASONS_BYTES: usize = 6 * 1024;
/// What a reason past `REASONS_BYTES` reads as.
const REASON_LEFT_OUT: &str = "its reason is in the run's report";
/// The most bytes of one summary sentence.
const SENTENCE_BYTES: usize = 1_500;

/// `text`'s length as JSON writes it, without the quotes around it: `"`
/// and `\\` take two bytes each. Control characters, which take more, were
/// already replaced by the health model.
fn json_len(text: &str) -> usize {
    text.len() + text.bytes().filter(|b| *b == b'"' || *b == b'\\').count()
}

/// `text` cut to at most `max` bytes as JSON writes it, at a character
/// boundary: the section copies the health model's words, never words them
/// again, and keeps them within its budget.
fn clip(text: &str, max: usize) -> String {
    if json_len(text) <= max {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .scan(0usize, |width, (at, c)| {
            *width += c.len_utf8() + usize::from(c == '"' || c == '\\');
            Some((at + c.len_utf8(), *width))
        })
        .take_while(|&(_, width)| width <= max)
        .map(|(end, _)| end)
        .last()
        .unwrap_or(0);
    text[..end].to_string()
}

/// What the section is built from, beside the health model and the merge.
pub(crate) struct SectionInput<'a> {
    /// `--expect-workers`.
    pub(crate) expected: usize,
    /// The snapshot's series window, which the RPS lines by worker follow.
    pub(crate) series: &'a SeriesWindow,
    /// The run's throttle, when it has one.
    pub(crate) throttle: Option<u64>,
    pub(crate) now: Instant,
}

/// One worker as the dashboard sees it now.
struct Live<'a> {
    worker: &'a WorkerHealth,
    figures: Option<health::Figures>,
    /// Its live flags, most severe first.
    flags: Vec<GaggleWorkerFlag>,
}

impl<'a> Live<'a> {
    fn of(worker: &'a WorkerHealth) -> Self {
        let live = &worker.live;
        let lost = worker.state == WorkerState::Lost;
        let mut flags = Vec::new();
        for (flag, on) in [
            (GaggleWorkerFlag::Lost, lost),
            (GaggleWorkerFlag::Excluded, live.excluded),
            (GaggleWorkerFlag::Saturated, live.saturated),
            (GaggleWorkerFlag::Lagging, live.lagging),
            (
                GaggleWorkerFlag::Outlier,
                live.rate_outlier.is_some() || live.p95_outlier.is_some(),
            ),
            (GaggleWorkerFlag::Refused, live.refused),
            (GaggleWorkerFlag::Missing, live.missing),
        ] {
            if on {
                flags.push(flag);
            }
        }
        // A worker not running has no live figures: what its window holds
        // is from before it stopped.
        let figures = worker.is_running().then(|| worker.window_figures());
        Live {
            worker,
            figures,
            flags,
        }
    }

    /// How urgent it is: its most severe flag, then the rest.
    fn severity(&self) -> usize {
        self.flags.first().map_or(usize::MAX, |flag| *flag as usize)
    }

    fn rps(&self) -> f64 {
        self.figures
            .and_then(|figures| figures.requests_per_second)
            .unwrap_or(0.0)
    }
}

/// A flag as the snapshot names it.
fn flag_name(flag: GaggleWorkerFlag) -> &'static str {
    match flag {
        GaggleWorkerFlag::Lost => "lost",
        GaggleWorkerFlag::Excluded => "excluded",
        GaggleWorkerFlag::Saturated => "saturated",
        GaggleWorkerFlag::Lagging => "lagging",
        GaggleWorkerFlag::Outlier => "outlier",
        GaggleWorkerFlag::Refused => "refused",
        GaggleWorkerFlag::Missing => "missing",
    }
}

/// A state as the grid codes it.
fn state_code(state: WorkerState) -> u8 {
    match state {
        WorkerState::Joining => 0,
        WorkerState::Ready => 1,
        WorkerState::Running => 2,
        WorkerState::Stopping => 3,
        WorkerState::Finished => 4,
        WorkerState::UsersExited => 5,
        WorkerState::Lost => 6,
    }
}

/// A worker's most severe live flag but its loss, which its state says, as
/// the grid codes it.
fn flag_code(flags: &[GaggleWorkerFlag]) -> u8 {
    flags
        .iter()
        .find_map(|flag| match flag {
            GaggleWorkerFlag::Lost => None,
            GaggleWorkerFlag::Excluded => Some(1),
            GaggleWorkerFlag::Saturated => Some(2),
            GaggleWorkerFlag::Lagging => Some(3),
            GaggleWorkerFlag::Outlier => Some(4),
            GaggleWorkerFlag::Refused | GaggleWorkerFlag::Missing => Some(5),
        })
        .unwrap_or(0)
}

/// `value` to one decimal.
fn tenths(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// `values` as groups of equal values, largest first.
fn groups(values: impl Iterator<Item = u64>) -> Vec<GaggleSplit> {
    let mut counts: BTreeMap<u64, u64> = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .rev()
        .map(|(value, workers)| GaggleSplit { value, workers })
        .collect()
}

/// The number at the start of `text`, written as `format_number` writes
/// it, `1,200`, up to the first character that is neither a digit nor a
/// comma.
fn leading_number(text: &str) -> Option<u64> {
    let digits: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The users before and after of a users change's event, `users 600 to
/// 900, acknowledged by ...`, as step 3 words it.
fn users_change(text: &str) -> Option<(u64, u64)> {
    let rest = text.strip_prefix("users ")?;
    let from = leading_number(rest)?;
    let (_, after) = rest.split_once(" to ")?;
    Some((from, leading_number(after)?))
}

/// The chart marker's few words for `event`: `users 900`, `throttle
/// 3,000`, `load-31 lost`.
fn label(event: &GaggleEvent) -> String {
    let worker = event.worker.as_deref().unwrap_or("worker");
    let number = crate::metrics::format_number;
    match event.kind {
        GaggleEventKind::RunStarted => "run started".to_string(),
        GaggleEventKind::Start => "start".to_string(),
        GaggleEventKind::Stop => "stop".to_string(),
        GaggleEventKind::UsersChanged => match users_change(&event.text) {
            Some((_, to)) => format!("users {}", number(to as usize)),
            None => "users".to_string(),
        },
        GaggleEventKind::ThrottleChanged => match event
            .text
            .strip_prefix("throttle set to ")
            .and_then(leading_number)
        {
            Some(throttle) => format!("throttle {}", number(throttle as usize)),
            None => "throttle off".to_string(),
        },
        GaggleEventKind::HostChanged => "host".to_string(),
        GaggleEventKind::ThrottleSharesRescaled => "throttle rescaled".to_string(),
        GaggleEventKind::UsersRebalanced => format!("{worker} users moved"),
        GaggleEventKind::WorkerLost => format!("{worker} lost"),
        GaggleEventKind::WorkerExcluded => format!("{worker} left out"),
        GaggleEventKind::DeltasRefused => format!("{worker} refused"),
        GaggleEventKind::WorkerSaturated => format!("{worker} saturated"),
        GaggleEventKind::WorkerLagging => format!("{worker} lagging"),
        GaggleEventKind::WorkerRecovered => format!("{worker} recovered"),
    }
}

/// An event kind as the snapshot names it: its name in the run record.
fn kind_name(kind: GaggleEventKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// `count` and `noun`, plural past 1: `1 worker`, `2 workers`.
fn count(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        _ => format!("{} {noun}s", crate::metrics::format_number(count)),
    }
}

/// The attention banner's sentence: each worker that needs attention now,
/// in step 4's words, saturated first, then lagging, left out and lost,
/// each by name; `None` when none does.
fn attention(live: &[Live<'_>], healthy: usize, now: Instant) -> Option<String> {
    let mut clauses: Vec<(u8, &str, String)> = Vec::new();
    for view in live {
        let worker = view.worker;
        let name = worker.name.as_str();
        let clause = if worker.state == WorkerState::Lost {
            let (second, why) = worker
                .lost
                .as_ref()
                .map_or((0, ""), |(_, second, why)| (*second, why.as_str()));
            (
                3,
                health::lost_at_text(name, second, &clip(why, REASON_BYTES)),
            )
        } else if worker.live.saturated {
            (
                0,
                health::saturated_text(name, worker.cpu_percent, worker.saturated_for()),
            )
        } else if worker.live.lagging {
            (
                1,
                health::lagging_text(name, worker.delta_gap(now).unwrap_or_default()),
            )
        } else if let Some(why) = &worker.merge.excluded {
            (2, health::excluded_text(name, &clip(why, REASON_BYTES)))
        } else {
            continue;
        };
        clauses.push((clause.0, name, clause.1));
    }
    if clauses.is_empty() {
        return None;
    }
    clauses.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let need = match clauses.len() {
        1 => "1 worker needs attention".to_string(),
        n => format!("{} need attention", count(n, "worker")),
    };
    let texts: Vec<String> = clauses.into_iter().map(|(_, _, text)| text).collect();
    Some(format!(
        "{need}: {}. {} healthy.",
        named(&texts),
        count(healthy, "worker")
    ))
}

/// The snapshot's `gaggle` section, from `health` and `accumulator`.
pub(crate) fn section(
    health: &GaggleHealth,
    accumulator: &GaggleAccumulator,
    input: SectionInput<'_>,
) -> GaggleSnapshot {
    let now = input.now;
    let live: Vec<Live<'_>> = health.workers().map(Live::of).collect();

    // Counts, over every worker.
    let has = |view: &Live<'_>, flag: GaggleWorkerFlag| view.flags.contains(&flag);
    let tally = |test: &dyn Fn(&Live<'_>) -> bool| live.iter().filter(|view| test(view)).count();
    let running = tally(&|view| view.worker.is_running());
    let healthy = tally(&|view| {
        view.worker.is_running()
            && !has(view, GaggleWorkerFlag::Saturated)
            && !has(view, GaggleWorkerFlag::Lagging)
            && !has(view, GaggleWorkerFlag::Excluded)
    });
    let records = health.records(now);
    let record = GaggleRecord {
        degraded: records.iter().any(|worker| worker.degraded()),
        workers: records,
        ..GaggleRecord::default()
    };
    let (events, dropped) = health.events();
    let events: Vec<&GaggleEvent> = events.collect();
    let last_users_change = events
        .iter()
        .rev()
        .filter(|event| event.kind == GaggleEventKind::UsersChanged)
        .find_map(|event| {
            users_change(&event.text).map(|(from, to)| GaggleUsersChange {
                second: event.second,
                from,
                to,
            })
        });
    let summary = GaggleSummary {
        expected: input.expected as u64,
        running: running as u64,
        stopped: tally(&|view| {
            matches!(
                view.worker.state,
                WorkerState::Stopping | WorkerState::Finished | WorkerState::UsersExited
            )
        }) as u64,
        lost: tally(&|view| has(view, GaggleWorkerFlag::Lost)) as u64,
        excluded: tally(&|view| has(view, GaggleWorkerFlag::Excluded)) as u64,
        saturated: tally(&|view| has(view, GaggleWorkerFlag::Saturated)) as u64,
        lagging: tally(&|view| has(view, GaggleWorkerFlag::Lagging)) as u64,
        outliers: tally(&|view| has(view, GaggleWorkerFlag::Outlier)) as u64,
        healthy: healthy as u64,
        attention: attention(&live, healthy, now).map(|text| clip(&text, SENTENCE_BYTES)),
        degraded: record
            .degraded_sentences_named()
            .iter()
            .map(|text| clip(text, SENTENCE_BYTES))
            .collect(),
        checks: check_sentences(&record.workers)
            .iter()
            .map(|text| clip(text, SENTENCE_BYTES))
            .collect(),
        last_users_change,
    };

    // The rows: most urgent first, at most MAX_WORKER_ROWS.
    let mut order: Vec<usize> = (0..live.len()).collect();
    order.sort_by(|&a, &b| {
        (live[a].severity(), &live[a].worker.name).cmp(&(live[b].severity(), &live[b].worker.name))
    });
    let rows: Vec<usize> = order.into_iter().take(MAX_WORKER_ROWS).collect();
    let column = |value: &dyn Fn(&Live<'_>) -> Option<f64>| -> Vec<Option<f64>> {
        rows.iter().map(|&at| value(&live[at])).collect()
    };
    let figure = |view: &Live<'_>| view.figures.unwrap_or_default();
    let mut reasons: Vec<String> = Vec::new();
    let mut reasons_bytes = 0;
    let mut workers = GaggleRows {
        workers: rows.iter().map(|&at| at as u64).collect(),
        cores: rows.iter().map(|&at| live[at].worker.cores).collect(),
        fail_percent: column(&|view| figure(view).fail_percent.map(tenths)),
        avg_ms: column(&|view| figure(view).average_ms.map(tenths)),
        requests_per_user_per_second: column(&|view| {
            figure(view)
                .requests_per_user_per_second
                .map(|rate| (rate * 100.0).round() / 100.0)
        }),
        last_delta_secs: column(&|view| {
            view.worker
                .delta_gap(now)
                .map(|gap| tenths(gap.as_secs_f64()))
        }),
        flags: rows
            .iter()
            .map(|&at| {
                live[at]
                    .flags
                    .iter()
                    .map(|flag| flag_name(*flag).to_string())
                    .collect()
            })
            .collect(),
        saturated_secs: rows
            .iter()
            .map(|&at| {
                let worker = live[at].worker;
                worker
                    .live
                    .saturated
                    .then(|| worker.saturated_for().as_secs())
            })
            .collect(),
        p95_gap_percent: rows
            .iter()
            .map(|&at| live[at].worker.live.p95_outlier)
            .collect(),
        lost_reason: rows
            .iter()
            .map(|&at| {
                live[at].worker.lost.as_ref().map(|(_, _, why)| {
                    let mut why = clip(why, REASON_BYTES);
                    if !reasons.contains(&why) && reasons_bytes + json_len(&why) > REASONS_BYTES {
                        why = REASON_LEFT_OUT.to_string();
                    }
                    match reasons.iter().position(|reason| *reason == why) {
                        Some(index) => index as u32,
                        None => {
                            reasons_bytes += json_len(&why);
                            reasons.push(why);
                            (reasons.len() - 1) as u32
                        }
                    }
                })
            })
            .collect(),
        lost_second: rows
            .iter()
            .map(|&at| live[at].worker.lost.as_ref().map(|(_, second, _)| *second))
            .collect(),
        reasons: Vec::new(),
    };
    workers.reasons = reasons;

    // The grid: every worker, in id order.
    let busiest = live.iter().map(Live::rps).fold(0.0, f64::max);
    let grid = GaggleGrid {
        names: live.iter().map(|view| view.worker.name.clone()).collect(),
        states: live
            .iter()
            .map(|view| state_code(view.worker.state))
            .collect(),
        flags: live.iter().map(|view| flag_code(&view.flags)).collect(),
        load: live
            .iter()
            .map(|view| match busiest > 0.0 {
                true => (view.rps() * 100.0 / busiest).round().clamp(0.0, 100.0) as u8,
                false => 0,
            })
            .collect(),
        users: live.iter().map(|view| view.worker.users as u64).collect(),
        cpu: live
            .iter()
            .map(|view| {
                view.worker
                    .cpu_percent
                    .filter(|_| view.figures.is_some())
                    .map_or(-1, |cpu| cpu.round().clamp(0.0, 100.0) as i32)
            })
            .collect(),
        rps: live
            .iter()
            .map(|view| {
                view.figures
                    .and_then(|f| f.requests_per_second)
                    .map_or(-1, |rps| rps.round().max(0.0) as i64)
            })
            .collect(),
        p95: live
            .iter()
            .map(|view| {
                view.figures
                    .and_then(|f| f.p95_ms)
                    .map_or(-1, |p95| p95.round().max(0.0) as i64)
            })
            .collect(),
        gap: live
            .iter()
            .map(|view| view.worker.live.rate_outlier.unwrap_or(0))
            .collect(),
    };

    // The RPS lines of the busiest running workers, over the series window.
    let mut busy: Vec<&Live<'_>> = live.iter().filter(|view| view.rps() > 0.0).collect();
    busy.sort_by(|a, b| {
        b.rps()
            .total_cmp(&a.rps())
            .then_with(|| a.worker.name.cmp(&b.worker.name))
    });
    busy.truncate(BY_WORKER_LINES);
    let series = input.series;
    let len = series.rps.len();
    let lines: Vec<Vec<u64>> = busy
        .iter()
        .map(|view| accumulator.requests_by_second(view.worker.id, series.start_second, len))
        .collect();
    let by_worker = GaggleByWorker {
        workers: busy.iter().map(|view| view.worker.name.clone()).collect(),
        rps: lines,
        // The others the rest's line adds up: every worker not lost.
        others_workers: live
            .iter()
            .filter(|view| view.worker.state != WorkerState::Lost)
            .count()
            .saturating_sub(busy.len()) as u64,
    };

    // The events, newest first, as many as fit EVENT_TEXTS_BYTES.
    let mut bytes = 0;
    let newest: Vec<&GaggleEvent> = events
        .iter()
        .rev()
        .take(MAX_GAGGLE_EVENTS)
        .take_while(|event| {
            bytes += json_len(&clip(&event.text, EVENT_TEXT_BYTES));
            bytes <= EVENT_TEXTS_BYTES
        })
        .copied()
        .collect();
    let shown = GaggleEvents {
        seconds: newest.iter().map(|event| event.second).collect(),
        kinds: newest.iter().map(|event| kind_name(event.kind)).collect(),
        labels: newest
            .iter()
            .enumerate()
            .map(|(at, event)| {
                // Newest first: only the first event of its second is labelled.
                let first = at == 0 || newest[at - 1].second != event.second;
                (first && event.second >= series.start_second).then(|| label(event))
            })
            .collect(),
        texts: newest
            .iter()
            .map(|event| clip(&event.text, EVENT_TEXT_BYTES))
            .collect(),
    };
    let events_truncated = dropped > 0 || events.len() > newest.len();

    // How a change splits now, by step 3's rules, over the workers it goes
    // to.
    let counts: Vec<usize> = live
        .iter()
        .filter(|view| view.worker.is_running())
        .map(|view| view.worker.users)
        .collect();
    let previews = GagglePreviews {
        workers: counts.len() as u64,
        users: groups(counts.iter().map(|&users| users as u64)),
        throttle: input
            .throttle
            .filter(|_| !counts.is_empty())
            .map(|throttle| {
                groups(
                    live_throttle_shares(throttle as usize, &counts)
                        .into_iter()
                        .map(|share| share as u64),
                )
            }),
    };

    GaggleSnapshot {
        expected_workers: input.expected as u64,
        summary,
        workers_truncated: live.len() > workers.workers.len(),
        workers,
        grid,
        by_worker,
        events: shown,
        events_truncated,
        previews,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gaggle::control::UserBook;
    use crate::gaggle::delta::{DeltaKind, DeltaLimits, GaggleDelta, RequestPart, Timing};
    use crate::gaggle::health::Fold;
    use crate::goose::GooseMethod;
    use crate::graph::GraphSettings;
    use crate::metrics::buckets;
    use crate::metrics::GooseMetrics;
    use std::time::Duration;

    fn secs(n: f64) -> Duration {
        Duration::from_secs_f64(n)
    }

    /// `n` successful requests, each taking `ms` milliseconds.
    fn fold(n: u64, ms: u64) -> Fold {
        let us = ms * 1_000;
        Fold {
            success: n as i64,
            fail: 0,
            times: Timing {
                buckets: if n > 0 {
                    vec![(buckets::bucket_floor(us), n)]
                } else {
                    Vec::new()
                },
                minimum_us: us,
                maximum_us: us,
                total_us: us * n,
                count: n,
            },
        }
    }

    /// A model of `workers` workers named `load-<id>`, each with `cores`
    /// and `users`, running since `at`.
    fn running(workers: u32, cores: Option<u32>, users: usize, at: Instant) -> GaggleHealth {
        let mut health = GaggleHealth::default();
        for id in 0..workers {
            health.join(id, &format!("load-{id:02}"), cores);
            health.set_state(id, WorkerState::Ready, at);
            health.set_users(id, users, users);
        }
        health.start(at);
        health
    }

    /// Ten seconds of `per_second` requests of `ms` each, for worker `id`,
    /// ending at `at`.
    fn window(health: &mut GaggleHealth, id: u32, per_second: u64, ms: u64, at: Instant) {
        for second in 0..10 {
            let when = at - secs(9.0 - second as f64);
            health.applied(id, Some((fold(per_second, ms), 1)), None, when);
        }
    }

    /// Worker `id`'s CPU at `percent` of its `cores` every second from
    /// `from` to `to` seconds past `at`.
    fn burn(health: &mut GaggleHealth, id: u32, cores: u64, percent: u64, at: Instant, to: u64) {
        for second in 0..=to {
            let micros = second * 1_000_000 * cores * percent / 100;
            health.cpu(id, Some(micros), None, at + secs(second as f64));
        }
    }

    fn accumulator() -> GaggleAccumulator {
        let settings = GraphSettings {
            records: true,
            latency: false,
            synthetic: false,
            columns: false,
        };
        GaggleAccumulator::new(GooseMetrics::default(), settings, DeltaLimits::default())
    }

    fn series(rps: Vec<f64>) -> SeriesWindow {
        SeriesWindow {
            start_second: 0,
            fps: vec![0.0; rps.len()],
            users: vec![0; rps.len()],
            avg_latency_ms: vec![0.0; rps.len()],
            rps,
            heatmap: None,
            provisional_from: None,
        }
    }

    fn build(
        health: &GaggleHealth,
        accumulator: &GaggleAccumulator,
        expected: usize,
        series: &SeriesWindow,
        throttle: Option<u64>,
        now: Instant,
    ) -> GaggleSnapshot {
        section(
            health,
            accumulator,
            SectionInput {
                expected,
                series,
                throttle,
                now,
            },
        )
    }

    /// Worker `name`'s index in the grid.
    fn index(section: &GaggleSnapshot, name: &str) -> usize {
        section
            .grid
            .names
            .iter()
            .position(|named| named == name)
            .unwrap_or_else(|| panic!("no worker {}", name))
    }

    /// Worker `name`'s row index.
    fn row(section: &GaggleSnapshot, name: &str) -> usize {
        let at = index(section, name) as u64;
        section
            .workers
            .workers
            .iter()
            .position(|&worker| worker == at)
            .unwrap_or_else(|| panic!("no row {}", name))
    }

    /// The rows' worker names, in order.
    fn names(section: &GaggleSnapshot) -> Vec<&str> {
        section
            .workers
            .workers
            .iter()
            .map(|&worker| section.grid.names[worker as usize].as_str())
            .collect()
    }

    /// A worker fed a known state shows it, every field read from the
    /// health model: saturated at 97% of 4 cores, 41 requests a second at
    /// 212 ms for 19 users, lost with its reason and second.
    #[test]
    fn rows_read_the_health_model() {
        let at = Instant::now();
        let mut health = running(4, Some(4), 19, at);
        burn(&mut health, 0, 4, 97, at, 6);
        let now = at + secs(6.0);
        window(&mut health, 0, 41, 212, now);
        for id in 1..4 {
            burn(&mut health, id, 4, 30, at, 6);
            window(&mut health, id, 63, 18, now);
        }
        health.set_second(185);
        assert!(health.lose(3, "connection reset", now));
        health.tick(now);
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 4, &empty, None, now);

        let rows = &section.workers;
        let saturated = row(&section, "load-00");
        assert_eq!(rows.cores[saturated], Some(4));
        assert_eq!(rows.fail_percent[saturated], Some(0.0));
        assert!(rows.avg_ms[saturated].is_some());
        assert_eq!(rows.requests_per_user_per_second[saturated], Some(2.16));
        assert_eq!(rows.flags[saturated], ["saturated"]);
        assert_eq!(rows.saturated_secs[saturated], Some(6));
        assert_eq!(rows.last_delta_secs[saturated], Some(0.0));
        assert_eq!(rows.lost_reason[saturated], None);

        let lost = row(&section, "load-03");
        assert_eq!(rows.flags[lost], ["lost"]);
        let reason = rows.lost_reason[lost].unwrap() as usize;
        assert_eq!(rows.reasons[reason], "connection reset");
        assert_eq!(rows.lost_second[lost], Some(185));
        assert_eq!(rows.avg_ms[lost], None);
        assert_eq!(rows.last_delta_secs[lost], None);

        // The most urgent first: lost, then saturated, then the rest.
        assert_eq!(
            names(&section),
            ["load-03", "load-00", "load-01", "load-02"]
        );
        assert!(!section.workers_truncated);

        let summary = &section.summary;
        assert_eq!(
            (
                summary.expected,
                summary.running,
                summary.lost,
                summary.saturated
            ),
            (4, 3, 1, 1)
        );
        assert_eq!(summary.healthy, 2);
        assert_eq!(
            summary.attention.as_deref(),
            Some("2 workers need attention: load-00 saturated (CPU 97% for 6 s), load-03 lost at 03:05 (connection reset). 2 workers healthy.")
        );
        // The reports' sentences, as they would open now.
        assert_eq!(summary.degraded.len(), 1, "{:?}", summary.degraded);
        assert!(summary.degraded[0].starts_with("Degraded: 1 of the 4 Gaggle workers were lost"));
        assert!(
            summary.checks[0].starts_with("Check: load-00 was CPU saturated for 6 s in total"),
            "{:?}",
            summary.checks
        );

        // The grid, in id order.
        let grid = &section.grid;
        assert_eq!(grid.names, ["load-00", "load-01", "load-02", "load-03"]);
        assert_eq!(grid.states, [2, 2, 2, 6]);
        assert_eq!(grid.flags, [2, 0, 0, 0]);
        assert_eq!(grid.load, [65, 100, 100, 0]);
        // The lost worker sent CPU before it was lost; none is shown now.
        assert_eq!(grid.cpu, [97, 30, 30, -1]);
        assert_eq!(grid.rps, [41, 63, 63, -1]);
        assert_eq!(grid.users, [19, 19, 19, 19]);
        let p95 = grid.p95[0];
        assert!((200..=225).contains(&p95), "{}", p95);
        assert_eq!(grid.p95[3], -1);
    }

    /// A worker that sends no CPU time shows none, in its row and the grid.
    #[test]
    fn cpu_absent() {
        let at = Instant::now();
        let mut health = running(2, None, 5, at);
        let now = at + secs(5.0);
        window(&mut health, 0, 10, 5, now);
        health.cpu(0, None, None, now);
        health.tick(now);
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 2, &empty, None, now);
        assert_eq!(section.workers.cores[row(&section, "load-00")], None);
        assert_eq!(section.grid.cpu, [-1, -1]);
        assert_eq!(section.grid.p95[0], 5);
    }

    /// Rows are the most urgent 100, by severity then name; the summary and
    /// grid count every worker.
    #[test]
    fn rows_sort_by_severity_and_cap_at_100() {
        let at = Instant::now();
        let mut health = running(150, Some(1), 2, at);
        let now = at + secs(8.0);
        for id in 0..150 {
            window(&mut health, id, 5, 10, now);
        }
        burn(&mut health, 120, 1, 99, at, 8);
        health.lose(140, "gone", now);
        health.lose(7, "gone", now);
        // Lagging: no delta for 5 seconds.
        health.applied(60, None, None, now - secs(5.0));
        let later = now + secs(0.5);
        health.applied(60, None, None, now - secs(5.0));
        health.tick(later);
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 150, &empty, None, later);
        assert_eq!(section.workers.workers.len(), MAX_WORKER_ROWS);
        assert_eq!(section.workers.flags.len(), MAX_WORKER_ROWS);
        assert!(section.workers_truncated);
        // Lost, saturated, lagging, then the rest, each by name.
        assert_eq!(
            names(&section)[..5],
            ["load-07", "load-140", "load-120", "load-60", "load-00"]
        );
        assert_eq!(section.summary.lost, 2);
        assert_eq!(section.summary.saturated, 1);
        assert_eq!(section.grid.names.len(), 150);
        // Every worker past the rows is in the grid, with its hover figures.
        let last = index(&section, "load-149");
        assert_eq!(section.grid.rps[last], 5);
    }

    /// The grid holds every one of 1,024 workers, one entry each in every
    /// array.
    #[test]
    fn grid_holds_1024_workers() {
        let at = Instant::now();
        let mut health = running(1_024, Some(2), 3, at);
        let now = at + secs(2.0);
        for id in 0..1_024 {
            window(&mut health, id, 1 + u64::from(id % 7), 10, now);
        }
        health.tick(now);
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 1_024, &empty, None, now);
        let grid = &section.grid;
        for len in [
            grid.names.len(),
            grid.states.len(),
            grid.flags.len(),
            grid.load.len(),
            grid.users.len(),
            grid.cpu.len(),
            grid.rps.len(),
            grid.p95.len(),
            grid.gap.len(),
        ] {
            assert_eq!(len, 1_024);
        }
        assert_eq!(grid.load.iter().copied().max(), Some(100));
        assert_eq!(section.summary.running, 1_024);
    }

    /// Events: newest first, at most 200, with a chart label each.
    #[test]
    fn events_newest_first_capped_at_200() {
        let mut health = GaggleHealth::default();
        health.join(0, "load-31", None);
        for second in 0..250u64 {
            health.set_second(second);
            health.event(
                GaggleEventKind::UsersChanged,
                None,
                format!(
                    "users {} to {}, acknowledged by 48 of 48 in 0.4 s",
                    second * 10,
                    1_200 + second
                ),
            );
        }
        health.set_second(250);
        health.lose(0, "connection reset", Instant::now());
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 1, &empty, None, Instant::now());
        let events = &section.events;
        assert_eq!(events.seconds.len(), MAX_GAGGLE_EVENTS);
        assert_eq!(events.texts.len(), MAX_GAGGLE_EVENTS);
        assert!(section.events_truncated);
        assert_eq!(events.seconds[0], 250);
        assert_eq!(events.kinds[0], "worker_lost");
        assert_eq!(events.labels[0].as_deref(), Some("load-31 lost"));
        assert_eq!(events.texts[0], "load-31 lost: connection reset");
        assert_eq!(events.labels[1].as_deref(), Some("users 1,449"));
        assert_eq!(events.kinds[1], "users_changed");
        assert!(events.seconds.windows(2).all(|pair| pair[0] >= pair[1]));
        assert_eq!(
            section.summary.last_users_change,
            Some(GaggleUsersChange {
                second: 249,
                from: 2_490,
                to: 1_449
            })
        );
    }

    /// Chart labels, from the event kinds and step 3's words.
    #[test]
    fn labels() {
        let event = |kind, worker: Option<&str>, text: &str| GaggleEvent {
            second: 0,
            time: String::new(),
            kind,
            worker: worker.map(str::to_string),
            text: text.to_string(),
        };
        let cases = [
            (
                event(
                    GaggleEventKind::ThrottleChanged,
                    None,
                    "throttle set to 3,000 req/s, acknowledged by 48 of 48 in 0.2 s",
                ),
                "throttle 3,000",
            ),
            (
                event(
                    GaggleEventKind::ThrottleChanged,
                    None,
                    "throttle removed, acknowledged by 2 of 2 in 0.1 s",
                ),
                "throttle off",
            ),
            (
                event(
                    GaggleEventKind::UsersChanged,
                    None,
                    "users 600 to 900, acknowledged by 48 of 48 in 0.4 s",
                ),
                "users 900",
            ),
            (
                event(
                    GaggleEventKind::WorkerSaturated,
                    Some("load-17"),
                    "load-17 saturated (CPU 97% for 5 s)",
                ),
                "load-17 saturated",
            ),
            (
                event(
                    GaggleEventKind::RunStarted,
                    None,
                    "48 of 48 workers joined, run started",
                ),
                "run started",
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(label(&event), expected);
        }
        assert_eq!(
            users_change("users 1,200 to 12,000, acknowledged"),
            Some((1_200, 12_000))
        );
        assert_eq!(users_change("users 6 to 8"), Some((6, 8)));
        assert_eq!(users_change("host set to x"), None);
    }

    /// The busiest workers' RPS lines come from the merge's per worker rows,
    /// over the series window; the rest are added up as "others".
    #[test]
    fn by_worker_lines_and_others() {
        let at = Instant::now();
        let mut health = running(10, Some(1), 1, at);
        let now = at + secs(10.0);
        let mut accumulator = accumulator();
        for id in 0..10u32 {
            // Worker n makes n + 1 requests a second; worker 0 the fewest.
            window(&mut health, id, u64::from(id) + 1, 10, now);
            let mut delta = GaggleDelta::new(id, 1, 0, DeltaKind::Delta);
            delta.now = 5;
            let mut request = RequestPart::new("/a", GooseMethod::Get, "");
            request.success = 5 * (i64::from(id) + 1);
            delta.metrics.requests.insert("GET /a".to_string(), request);
            let rows = (0..5).map(|second| (second, u64::from(id) + 1)).collect();
            delta.graph.requests.insert("GET /a".to_string(), rows);
            accumulator.apply(id, delta).unwrap();
        }
        // A lost worker is not among the others the rest's line adds up.
        health.lose(0, "gone", now);
        health.tick(now);
        // The merged series counts every worker: 55 a second.
        let merged = series(vec![55.0; 6]);
        let section = build(&health, &accumulator, 10, &merged, None, now);
        let lines = &section.by_worker;
        assert_eq!(
            lines.workers,
            [
                "load-09", "load-08", "load-07", "load-06", "load-05", "load-04", "load-03",
                "load-02"
            ]
        );
        assert_eq!(lines.rps[0], [10, 10, 10, 10, 10, 0]);
        assert_eq!(lines.rps[7], [3, 3, 3, 3, 3, 0]);
        // The others, worker 1 (worker 0 is lost), are the merged series
        // less these.
        assert_eq!(lines.others_workers, 1);
    }

    /// The split previews are step 3's: 900 users dealt over 47 workers run
    /// 20 on 7 and 19 on 40; a throttle of 3,000 over 47 workers with as
    /// many users each is 64 on 39 and 63 on 8, and over the 900 users,
    /// by each worker's share of them, 67 on 7, 64 on 11 and 63 on 29.
    #[test]
    fn split_previews() {
        let mut book = UserBook::new(vec![Vec::new(); 47], 0);
        book.set_users(900, &[true; 47], vec![0; 900]);
        let at = Instant::now();
        let mut health = running(48, Some(4), 0, at);
        for id in 0..47u32 {
            let users = book.users(id as usize).len();
            health.set_users(id, users, users);
        }
        // The 48th is lost: no change goes to it.
        health.lose(47, "connection reset", at);
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 48, &empty, Some(3_000), at);
        let split = |pairs: &[(u64, u64)]| -> Vec<GaggleSplit> {
            pairs
                .iter()
                .map(|&(value, workers)| GaggleSplit { value, workers })
                .collect()
        };
        let previews = &section.previews;
        assert_eq!(previews.workers, 47);
        assert_eq!(previews.users, split(&[(20, 7), (19, 40)]));
        assert_eq!(
            previews.throttle,
            Some(split(&[(67, 7), (64, 11), (63, 29)]))
        );
        // With as many users on each worker, the throttle splits evenly.
        let equal = running(47, Some(4), 19, at);
        let section = build(&equal, &accumulator(), 47, &empty, Some(3_000), at);
        assert_eq!(section.previews.throttle, Some(split(&[(64, 39), (63, 8)])));
        assert_eq!(section.previews.users, split(&[(19, 47)]));
        // Without a throttle, none.
        let section = build(&health, &accumulator(), 48, &empty, None, at);
        assert_eq!(section.previews.throttle, None);
    }

    /// 1,024 workers, each with 50 request names, flagged as a troubled
    /// run is, with 200 events: the section the snapshot gains, as JSON.
    fn large(at: Instant) -> (GaggleHealth, GaggleAccumulator, SeriesWindow) {
        let mut health = running(1_024, Some(4), 20, at);
        let mut accumulator = accumulator();
        let now = at + secs(40.0);
        let names: Vec<String> = (0..50).map(|n| format!("GET /page/{n}")).collect();
        for id in 0..1_024u32 {
            // Ten one second samples, each of 50 names with their own times.
            for second in 0..10u64 {
                let parts: Vec<RequestPart> = (0..50u64)
                    .map(|n| {
                        let mut part =
                            RequestPart::new(&format!("/page/{n}"), GooseMethod::Get, "");
                        let count = 1 + (u64::from(id) + n) % 3;
                        for k in 0..count {
                            let us = 2_000 + 1_000 * ((n * 7 + k * 13 + u64::from(id)) % 300);
                            part.raw.buckets.push((buckets::bucket_floor(us), 1));
                            part.raw.count += 1;
                            part.raw.total_us += us;
                        }
                        part.raw.buckets.sort_unstable();
                        part.raw.buckets.dedup_by(|b, a| {
                            let same = a.0 == b.0;
                            if same {
                                a.1 += b.1;
                            }
                            same
                        });
                        part.success = count as i64;
                        part
                    })
                    .collect();
                let when = now - secs(9.0 - second as f64);
                health.applied(id, Some((Fold::of(parts.iter()), 1)), None, when);
            }
            // Graph rows for its 50 names: the busiest over the whole
            // window, the rest over its last 10 seconds.
            let span = if id >= 1_016 { 300 } else { 10 };
            let mut delta = GaggleDelta::new(id, 1, 0, DeltaKind::Delta);
            delta.now = 2_400;
            for name in &names {
                let mut request = RequestPart::new(&name[4..], GooseMethod::Get, "");
                request.success = span as i64;
                delta.metrics.requests.insert(name.clone(), request);
                let rows = (2_400 - span..2_400).map(|second| (second, 1)).collect();
                delta.graph.requests.insert(name.clone(), rows);
            }
            accumulator.apply(id, delta).unwrap();
            let percent = if id % 50 == 0 { 97 } else { 40 };
            burn(&mut health, id, 4, percent, at + secs(30.0), 10);
        }
        for id in (0..1_024).step_by(97) {
            health.set_second(185);
            health.lose(id, "connection reset by peer", now);
        }
        // A run 40 minutes in: its events spread over it, its window the
        // last 5 minutes.
        for n in 0..200u64 {
            health.set_second(n * 12);
            health.event(
                GaggleEventKind::UsersChanged,
                None,
                format!(
                    "users {} to {}, acknowledged by 1,024 of 1,024 in 0.4 s",
                    20_000 + n,
                    20_480 + n
                ),
            );
        }
        health.tick(now);
        let mut series = series(vec![1_024.0 * 100.0; 300]);
        series.start_second = 2_100;
        (health, accumulator, series)
    }

    /// The section for 1,024 workers adds at most 64 KiB to the snapshot.
    #[test]
    fn section_budget_at_1024_workers() {
        let at = Instant::now();
        let (health, accumulator, series) = large(at);
        let now = at + secs(40.0);
        let started = Instant::now();
        let section = build(&health, &accumulator, 1_024, &series, Some(100_000), now);
        let built = started.elapsed();
        let bytes = serde_json::to_vec(&section).unwrap();
        println!(
            "gaggle section at 1,024 workers: {} bytes, built in {:?} (debug build)",
            bytes.len(),
            built
        );
        assert!(section.summary.saturated > 0 && section.summary.lost > 0);
        assert_eq!(section.events.texts.len(), MAX_GAGGLE_EVENTS);
        assert_eq!(section.by_worker.workers.len(), BY_WORKER_LINES);
        for (part, len) in [
            (
                "summary",
                serde_json::to_vec(&section.summary).unwrap().len(),
            ),
            (
                "workers",
                serde_json::to_vec(&section.workers).unwrap().len(),
            ),
            ("grid", serde_json::to_vec(&section.grid).unwrap().len()),
            (
                "by_worker",
                serde_json::to_vec(&section.by_worker).unwrap().len(),
            ),
            (
                "names",
                serde_json::to_vec(&section.grid.names).unwrap().len(),
            ),
            ("events", serde_json::to_vec(&section.events).unwrap().len()),
            (
                "previews",
                serde_json::to_vec(&section.previews).unwrap().len(),
            ),
        ] {
            println!("  {part}: {len} bytes");
        }
        assert!(bytes.len() <= 64 * 1024, "{} bytes", bytes.len());
    }

    /// The section's build time at 1,024 workers, median and p99 of 200
    /// builds and serializations: run in release with `--ignored`.
    #[test]
    #[ignore]
    fn section_build_time_at_1024_workers() {
        let at = Instant::now();
        let (health, accumulator, series) = large(at);
        let now = at + secs(40.0);
        let mut times: Vec<Duration> = (0..200)
            .map(|_| {
                let started = Instant::now();
                let section = build(&health, &accumulator, 1_024, &series, Some(100_000), now);
                let bytes = serde_json::to_vec(&section).unwrap();
                std::hint::black_box(bytes);
                started.elapsed()
            })
            .collect();
        times.sort();
        println!(
            "gaggle section build and serialize at 1,024 workers, 200 builds: median {:?}, p99 {:?}, max {:?}",
            times[100],
            times[197],
            times[199]
        );
        if !cfg!(debug_assertions) {
            assert!(
                times[197] <= Duration::from_millis(5),
                "p99 {:?} over 5 ms",
                times[197]
            );
        }
    }

    /// A lost worker's reason, which a worker's own words can reach, is
    /// kept bounded: a reason of 1 MiB leaves the section small.
    #[test]
    fn a_long_lost_reason_is_bounded() {
        let at = Instant::now();
        let mut health = running(2, Some(2), 4, at);
        health.lose(1, &"x\u{7}".repeat(1 << 19), at + secs(1.0));
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 2, &empty, None, at + secs(1.0));
        let bytes = serde_json::to_vec(&section).unwrap().len();
        assert!(bytes < 4 * 1024, "{} bytes", bytes);
        let reason = &section.workers.reasons[section.workers.lost_reason[0].unwrap() as usize];
        // Kept to 500 characters by the model, and cut to REASON_BYTES here.
        assert_eq!(reason.len(), REASON_BYTES);
        assert!(!reason.contains('\u{7}'));
    }

    /// Every one of 1,024 workers lost, each with a reason of its own as long
    /// as a worker's words may be: the section still holds at most 64 KiB,
    /// with names as long as `load-1023`. Names of the longest, 64
    /// characters, take 68 KB alone at 1,024 workers, so the budget holds for
    /// names up to about 10 characters there.
    #[test]
    fn every_worker_lost_stays_in_budget() {
        let at = Instant::now();
        let mut health = running(1_024, Some(4), 20, at);
        // Each with a reason of its own, as long as a worker's words may be,
        // in characters of 2 bytes each.
        for id in 0..1_024 {
            let why = format!("{id} {}", "\u{e9}".repeat(500));
            health.lose(id, &why, at + secs(10.0));
        }
        let empty = series(vec![0.0; 300]);
        let section = build(
            &health,
            &accumulator(),
            1_024,
            &empty,
            Some(3_000),
            at + secs(10.0),
        );
        let bytes = serde_json::to_vec(&section).unwrap().len();
        println!("every worker lost, distinct reasons: {} bytes", bytes);
        let part = |v: serde_json::Value| v.to_string().len();
        let json = serde_json::to_value(&section).unwrap();
        assert!(
            bytes <= 64 * 1024,
            "{} bytes: summary {}, rows {}, grid {}, events {}",
            bytes,
            part(json["summary"].clone()),
            part(json["workers"].clone()),
            part(json["grid"].clone()),
            part(json["events"].clone())
        );
        let degraded = &section.summary.degraded;
        assert_eq!(degraded.len(), 1, "{:?}", degraded);
        assert!(degraded[0].len() <= SENTENCE_BYTES, "{}", degraded[0].len());
        assert_eq!(section.by_worker.others_workers, 0);
        // The reasons and event texts are cut to their budgets, the events
        // past it left out, and said to be.
        let reasons: usize = section.workers.reasons.iter().map(String::len).sum();
        assert!(
            reasons <= REASONS_BYTES + REASON_LEFT_OUT.len(),
            "{}",
            reasons
        );
        assert!(section.workers.reasons.iter().any(|r| r == REASON_LEFT_OUT));
        let texts: usize = section.events.texts.iter().map(String::len).sum();
        assert!(texts <= EVENT_TEXTS_BYTES, "{}", texts);
        assert!(section.events.texts.len() < MAX_GAGGLE_EVENTS);
        assert!(section.events_truncated);
        assert_eq!(section.events.labels.iter().flatten().count(), 1);
    }

    /// Reasons made of characters JSON escapes, `"` and `\\`, are held to
    /// the same budgets as written: the section stays within 64 KiB.
    #[test]
    fn escaped_reasons_stay_in_budget() {
        let at = Instant::now();
        let mut health = running(1_024, Some(4), 20, at);
        for id in 0..1_024 {
            let why = format!("{id} {}", "\"\\".repeat(250));
            health.lose(id, &why, at + secs(10.0));
        }
        let empty = series(vec![0.0; 300]);
        let section = build(
            &health,
            &accumulator(),
            1_024,
            &empty,
            None,
            at + secs(10.0),
        );
        let bytes = serde_json::to_vec(&section).unwrap().len();
        println!("every worker lost, escaped reasons: {} bytes", bytes);
        assert!(bytes <= 64 * 1024, "{} bytes", bytes);
        for reason in &section.workers.reasons {
            assert!(json_len(reason) <= REASON_BYTES.max(REASON_LEFT_OUT.len()));
        }
        let texts: usize = section.events.texts.iter().map(|t| json_len(t)).sum();
        assert!(texts <= EVENT_TEXTS_BYTES, "{}", texts);
        assert_eq!(clip("ab\"c", 3), "ab");
        assert_eq!(clip("\u{e9}\u{e9}", 3), "\u{e9}");
    }

    /// The section holds worker names, never an address, port or token (a
    /// name is by default the worker's HOSTNAME): every string in it is a name, a state, a flag, a kind, a
    /// label or a health sentence.
    #[test]
    fn names_only() {
        let at = Instant::now();
        let mut health = running(3, Some(2), 4, at);
        health.lose(2, "connection reset", at + secs(3.0));
        let empty = series(Vec::new());
        let section = build(&health, &accumulator(), 3, &empty, None, at + secs(3.0));
        let json = serde_json::to_string(&section).unwrap();
        assert!(
            !json.contains("127.0.0.1") && !json.contains("localhost"),
            "{}",
            json
        );
        // No field is named for an address, a port or a token.
        let keys = |value: &serde_json::Value| -> Vec<String> {
            fn walk(value: &serde_json::Value, out: &mut Vec<String>) {
                match value {
                    serde_json::Value::Object(map) => {
                        for (key, value) in map {
                            out.push(key.clone());
                            walk(value, out);
                        }
                    }
                    serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
                    _ => {}
                }
            }
            let mut out = Vec::new();
            walk(value, &mut out);
            out
        };
        for key in keys(&serde_json::to_value(&section).unwrap()) {
            for word in ["addr", "port", "token", "host"] {
                assert!(!key.contains(word), "{} in {}", key, json);
            }
        }
    }

    /// 1,000 random cases of step 3's split rules, as the dashboard's split
    /// previews read them (`split-fixture.json`, checked by
    /// `split.test.mjs`): a book of users dealt over 1 to 24 workers, as a
    /// run deals them, changed to a random number of users, and a random
    /// throttle split over random users. Written with
    /// `GOOSE_UPDATE_SPLIT_FIXTURE=1`, otherwise compared.
    #[test]
    fn split_fixture_matches_the_rules() {
        let mut seed: u64 = 0x5eed_6a66_1e05;
        let mut below = |n: u64| -> u64 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        let mut cases = Vec::new();
        for _ in 0..1_000 {
            let workers = 1 + below(24) as usize;
            let live = vec![true; workers];
            let mut book = UserBook::new(vec![Vec::new(); workers], 0);
            // A run's users, then a change or two, as a run deals them.
            for _ in 0..=below(3) {
                book.set_users(below(400) as usize, &live, vec![0; 400]);
            }
            let counts: Vec<usize> = (0..workers).map(|w| book.users(w).len()).collect();
            let users = below(500) as usize;
            book.set_users(users, &live, vec![0; 500]);
            let after: Vec<usize> = (0..workers).map(|w| book.users(w).len()).collect();
            // The throttle over any users, some workers with none.
            let loads: Vec<usize> = (0..workers)
                .map(|_| match below(5) {
                    0 => 0,
                    _ => below(60) as usize,
                })
                .collect();
            let throttle = 1 + below(20_000) as usize;
            let shares = live_throttle_shares(throttle, &loads);
            cases.push(serde_json::json!({
                "counts": counts,
                "users": users,
                "users_after": after,
                "loads": loads,
                "throttle": throttle,
                "shares": shares,
            }));
        }
        let mut text = String::from("[\n");
        for (at, case) in cases.iter().enumerate() {
            text.push_str(&case.to_string());
            text.push_str(if at + 1 < cases.len() { ",\n" } else { "\n" });
        }
        text.push_str("]\n");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/dashboard/static/split-fixture.json");
        if std::env::var_os("GOOSE_UPDATE_SPLIT_FIXTURE").is_some() {
            assert!(std::env::var_os("CI").is_none(), "not in CI");
            std::fs::write(&path, &text).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(&path).expect("read split-fixture.json");
        assert!(
            committed == text,
            "split-fixture.json no longer matches step 3's rules: regenerate it with \
             `GOOSE_UPDATE_SPLIT_FIXTURE=1 cargo test --features gaggle --lib split_fixture`"
        );
    }
}
