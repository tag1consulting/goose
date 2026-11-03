//! The Workers and Events sections a Gaggle run's reports carry: one row a
//! worker, worst first, and what happened to the Gaggle, oldest first. Built
//! from the run record alone, so a report read back from its JSON shows them
//! as the run's own did.

use crate::metrics::format_number;
use crate::runs::{secs_text, GaggleRecord, GaggleWorkerFlag, GaggleWorkerRecord, GaggleWorkerState};
use std::fmt::Write;

/// The most worker rows the console print shows.
pub(crate) const CONSOLE_ROWS: usize = 20;

/// The Workers table's headings.
pub(crate) const HEADINGS: [&str; 12] = [
    "Worker",
    "State",
    "Users",
    "Requests",
    "req/s",
    "Fail %",
    "Avg (ms)",
    "p95 (ms)",
    "req/user/s",
    "Peak CPU",
    "Max delta gap",
    "Flags",
];

/// One worker's row, as text.
pub(crate) struct Row {
    pub(crate) cells: [String; 12],
}

/// `value` with `decimals` decimals, or a dash when absent.
fn or_dash(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(|| "-".to_string(), |value| format!("{value:.decimals$}"))
}

/// Series second `second` as `mm:ss` from the run's start, `h:mm:ss` past an
/// hour.
pub(crate) fn clock(second: u64) -> String {
    let (hours, minutes, seconds) = (second / 3600, second / 60 % 60, second % 60);
    match hours {
        0 => format!("{minutes:02}:{seconds:02}"),
        hours => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}

/// A worker's flags as words, never only as a colour: `lost 03:05`,
/// `saturated 42 s`, `lagging 6 s`, `p95 +35%`.
pub(crate) fn flags_text(worker: &GaggleWorkerRecord) -> String {
    let signed = |percent: i32| match percent >= 0 {
        true => format!("+{percent}%"),
        false => format!("{percent}%"),
    };
    let mut words = Vec::new();
    for flag in &worker.flags {
        match flag {
            GaggleWorkerFlag::Lost => words.push(match worker.lost_second {
                Some(second) => format!("lost {}", clock(second)),
                None => "lost".to_string(),
            }),
            GaggleWorkerFlag::Excluded => words.push("excluded".to_string()),
            GaggleWorkerFlag::Saturated => words.push(format!(
                "saturated {} s",
                worker.saturated_secs.unwrap_or(0)
            )),
            GaggleWorkerFlag::Lagging => words.push(match worker.longest_delta_gap_secs {
                Some(gap) => format!("lagging {} s", secs_text(gap)),
                None => "lagging".to_string(),
            }),
            GaggleWorkerFlag::Outlier => {
                if let Some(percent) = worker.rate_vs_median_percent {
                    words.push(format!("req/user/s {}", signed(percent)));
                }
                if let Some(percent) = worker.p95_vs_median_percent {
                    words.push(format!("p95 {}", signed(percent)));
                }
            }
            GaggleWorkerFlag::Refused => {
                words.push(format!("refused {}", worker.refused_deltas))
            }
            GaggleWorkerFlag::Missing => {
                if worker.missing_deltas > 0 {
                    words.push(format!("missing {}", worker.missing_deltas));
                }
                if worker.final_missing {
                    words.push("final missing".to_string());
                }
            }
        }
    }
    words.join(", ")
}

/// Where a worker sorts: its most severe flag, healthy last.
fn severity(worker: &GaggleWorkerRecord) -> usize {
    // A record of an older Goose has no flags: its lost workers still sort
    // first.
    if worker.state == GaggleWorkerState::Lost {
        return 0;
    }
    worker
        .flags
        .iter()
        .map(|flag| *flag as usize)
        .min()
        .unwrap_or(usize::MAX)
}

/// Every worker, most severe first (lost, excluded, saturated, lagging,
/// outlier, refused or missing, healthy), then by name.
pub(crate) fn sorted(record: &GaggleRecord) -> Vec<&GaggleWorkerRecord> {
    let mut workers: Vec<&GaggleWorkerRecord> = record.workers.iter().collect();
    workers.sort_by(|a, b| {
        severity(a)
            .cmp(&severity(b))
            .then_with(|| a.name.cmp(&b.name))
    });
    workers
}

/// A worker's row.
pub(crate) fn row(worker: &GaggleWorkerRecord) -> Row {
    let fail = match (worker.requests, worker.failures) {
        (Some(requests), Some(failures)) if requests > 0 => {
            Some(failures as f64 * 100.0 / requests as f64)
        }
        _ => None,
    };
    Row {
        cells: [
            worker.name.clone(),
            worker.state.label().to_string(),
            format_number(worker.users),
            worker
                .requests
                .map_or_else(|| "-".to_string(), |requests| format_number(requests as usize)),
            or_dash(worker.requests_per_second, 1),
            or_dash(fail, 1),
            or_dash(worker.average_ms, 1),
            or_dash(worker.p95_ms, 1),
            or_dash(worker.requests_per_user_per_second, 2),
            worker
                .peak_cpu_percent
                .map_or_else(|| "-".to_string(), |peak| format!("{peak}%")),
            worker
                .longest_delta_gap_secs
                .map_or_else(|| "-".to_string(), |gap| format!("{} s", secs_text(gap))),
            flags_text(worker),
        ],
    }
}

/// The line over the table: `47 of 48 workers finished, 1 lost, 1
/// saturated, 1 lagging`.
pub(crate) fn summary(record: &GaggleRecord) -> String {
    let total = record.workers.len();
    let count = |state: &[GaggleWorkerState]| {
        record
            .workers
            .iter()
            .filter(|worker| state.contains(&worker.state))
            .count()
    };
    let finished = count(&[GaggleWorkerState::Finished, GaggleWorkerState::UsersExited]);
    let running = count(&[GaggleWorkerState::Running, GaggleWorkerState::Stopping]);
    let mut parts = vec![match (finished, running) {
        (0, running) if running > 0 => format!("{running} of {total} workers running"),
        (finished, _) => format!("{finished} of {total} workers finished"),
    }];
    let flagged = |flag: GaggleWorkerFlag| {
        record
            .workers
            .iter()
            .filter(|worker| worker.flags.contains(&flag))
            .count()
    };
    let lost = count(&[GaggleWorkerState::Lost]);
    if lost > 0 {
        parts.push(format!("{lost} lost"));
    }
    for (flag, word) in [
        (GaggleWorkerFlag::Excluded, "excluded"),
        (GaggleWorkerFlag::Saturated, "saturated"),
        (GaggleWorkerFlag::Lagging, "lagging"),
        (GaggleWorkerFlag::Outlier, "outlying"),
        (GaggleWorkerFlag::Refused, "with refused deltas"),
        (GaggleWorkerFlag::Missing, "with missing deltas"),
    ] {
        let n = flagged(flag);
        if n > 0 {
            parts.push(format!("{n} {word}"));
        }
    }
    parts.join(", ")
}

/// What the Events section says of the events left out.
pub(crate) fn dropped_text(dropped: u64) -> String {
    format!(
        "{} older events are not shown.",
        format_number(dropped as usize)
    )
}

/// The HTML report's Workers and Events sections, escaped with `escape`;
/// nothing for a run of one process.
pub(crate) fn html(record: Option<&GaggleRecord>, escape: fn(&str) -> String) -> String {
    let Some(record) = record else {
        return String::new();
    };
    if record.workers.is_empty() {
        return String::new();
    }
    let mut html = String::new();
    let _ = write!(
        html,
        r#"

        <div class="workers">
        <h2>Workers</h2>
        <p>{}</p>
        <table>
            <thead>
                <tr>"#,
        escape(&summary(record))
    );
    for heading in HEADINGS {
        let _ = write!(html, "\n                    <th>{}</th>", escape(heading));
    }
    html.push_str(
        r#"
                </tr>
            </thead>
            <tbody>"#,
    );
    for worker in sorted(record) {
        html.push_str("\n                <tr>");
        for cell in row(worker).cells {
            let _ = write!(html, "<td>{}</td>", escape(&cell));
        }
        html.push_str("</tr>");
    }
    html.push_str(
        r#"
            </tbody>
        </table>
    </div>"#,
    );
    if record.events.is_empty() && record.events_dropped == 0 {
        return html;
    }
    html.push_str(
        r#"

        <div class="events">
        <h2>Events</h2>
        <table>
            <thead>
                <tr>
                    <th>Time</th>
                    <th colspan="5">Event</th>
                </tr>
            </thead>
            <tbody>"#,
    );
    for event in &record.events {
        let _ = write!(
            html,
            "\n                <tr><td>{}</td><td colspan=\"5\">{}</td></tr>",
            clock(event.second),
            escape(&event.text)
        );
    }
    html.push_str(
        r#"
            </tbody>
        </table>"#,
    );
    if record.events_dropped > 0 {
        let _ = write!(html, "\n        <p>{}</p>", escape(&dropped_text(record.events_dropped)));
    }
    html.push_str("\n    </div>");
    html
}

/// The console print's Gaggle sections: the intro sentences, at most
/// [`CONSOLE_ROWS`] workers, worst first, and the events.
pub(crate) fn console(record: &GaggleRecord) -> String {
    let mut text = String::new();
    if record.workers.is_empty() {
        return text;
    }
    let rows: Vec<Row> = sorted(record).into_iter().map(row).collect();
    let mut widths: Vec<usize> = HEADINGS.iter().map(|heading| heading.chars().count()).collect();
    for row in rows.iter().take(CONSOLE_ROWS) {
        for (width, cell) in widths.iter_mut().zip(&row.cells) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: &[String]| -> String {
        let mut line = String::from(" ");
        for (at, (cell, width)) in cells.iter().zip(&widths).enumerate() {
            if at > 0 {
                line.push_str(" | ");
            }
            // Text left, numbers right.
            match at {
                0 | 1 | 11 => line.push_str(&format!("{cell:<width$}")),
                _ => line.push_str(&format!("{cell:>width$}")),
            }
        }
        line.trim_end().to_string()
    };
    let headings: Vec<String> = HEADINGS.iter().map(|heading| heading.to_string()).collect();
    let header = line(&headings);
    let rule = format!(" {}", "-".repeat(header.chars().count().saturating_sub(1)));
    let _ = writeln!(text, "\n === GAGGLE WORKERS ===\n{rule}");
    for sentence in record.intro() {
        let _ = writeln!(text, " {sentence}");
    }
    let _ = writeln!(text, " {}\n{rule}\n{header}\n{rule}", summary(record));
    for row in rows.iter().take(CONSOLE_ROWS) {
        let _ = writeln!(text, "{}", line(&row.cells));
    }
    if rows.len() > CONSOLE_ROWS {
        let _ = writeln!(
            text,
            " and {} more workers: see the HTML or JSON report",
            rows.len() - CONSOLE_ROWS
        );
    }
    let _ = writeln!(text, "{rule}");
    if !record.events.is_empty() || record.events_dropped > 0 {
        let _ = writeln!(text, "\n === GAGGLE EVENTS ===\n{rule}");
        for event in &record.events {
            let _ = writeln!(text, " {} {}", clock(event.second), event.text);
        }
        if record.events_dropped > 0 {
            let _ = writeln!(text, " {}", dropped_text(record.events_dropped));
        }
        let _ = writeln!(text, "{rule}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{GaggleEvent, GaggleEventKind};

    fn worker(id: u32, name: &str) -> GaggleWorkerRecord {
        GaggleWorkerRecord {
            id,
            name: name.to_string(),
            users: 10,
            state: GaggleWorkerState::Finished,
            cores: Some(4),
            peak_cpu_percent: Some(40),
            saturated_secs: Some(0),
            longest_delta_gap_secs: Some(1.2),
            running_secs: Some(60.0),
            requests: Some(6_000),
            failures: Some(6),
            average_ms: Some(20.0),
            p50_ms: Some(18.0),
            p95_ms: Some(40.0),
            p99_ms: Some(60.0),
            requests_per_second: Some(100.0),
            requests_per_user_per_second: Some(10.0),
            ..GaggleWorkerRecord::default()
        }
    }

    /// A record with a lost, a saturated, a lagging and an outlier worker,
    /// and a healthy one.
    fn record() -> GaggleRecord {
        let mut lost = worker(0, "load-31");
        lost.state = GaggleWorkerState::Lost;
        lost.lost_second = Some(185);
        lost.lost_reason = Some("connection reset".to_string());
        lost.flags = vec![GaggleWorkerFlag::Lost];
        let mut saturated = worker(1, "load-17");
        saturated.peak_cpu_percent = Some(97);
        saturated.saturated_secs = Some(42);
        saturated.flags = vec![GaggleWorkerFlag::Saturated];
        let mut lagging = worker(2, "load-42");
        lagging.longest_delta_gap_secs = Some(6.0);
        lagging.flags = vec![GaggleWorkerFlag::Lagging];
        let mut outlier = worker(3, "load-09");
        outlier.p95_vs_median_percent = Some(35);
        outlier.flags = vec![GaggleWorkerFlag::Outlier];
        GaggleRecord {
            workers: vec![worker(4, "load-01"), outlier, lagging, saturated, lost],
            degraded: true,
            events: vec![GaggleEvent {
                second: 185,
                time: "2026-11-03T10:03:05Z".to_string(),
                kind: GaggleEventKind::WorkerLost,
                worker: Some("load-31".to_string()),
                text: "load-31 lost: connection reset".to_string(),
            }],
            events_dropped: 3,
            ..GaggleRecord::default()
        }
    }

    #[test]
    fn every_flag_shows_in_the_table_and_the_intro() {
        let record = record();
        let order: Vec<&str> = sorted(&record).iter().map(|w| w.name.as_str()).collect();
        assert_eq!(order, ["load-31", "load-17", "load-42", "load-09", "load-01"]);
        assert_eq!(
            summary(&record),
            "4 of 5 workers finished, 1 lost, 1 saturated, 1 lagging, 1 outlying"
        );
        let flags: Vec<String> = sorted(&record).into_iter().map(flags_text).collect();
        assert_eq!(flags, ["lost 03:05", "saturated 42 s", "lagging 6 s", "p95 +35%", ""]);
        let intro = record.intro();
        let checks: Vec<&String> = intro.iter().filter(|s| s.starts_with("Check:")).collect();
        assert_eq!(
            checks,
            [
                "Check: load-17 was CPU saturated for 42 s in total (peak 97% of 4 cores); its share of requests may be capped and its response times inflated by the generator, not the target.",
                "Check: load-42's metrics arrived late (longest gap 6 s).",
                "Check: load-09's p95 was 35% above the median of the running workers.",
            ]
        );
        // The Degraded sentence comes first, unchanged.
        assert!(intro[0].starts_with("Degraded: 1 of the 5 Gaggle workers were lost"));
        let html = html(Some(&record), crate::report::escape_html);
        for text in [
            "<td>load-31</td>",
            "lost 03:05",
            "saturated 42 s",
            "p95 +35%",
            "<td>03:05</td>",
            "3 older events",
        ] {
            assert!(html.contains(text), "{}: {}", text, html);
        }
        assert!(!html.contains("style="), "no inline style");
        // The Markdown report's sections are checked on a real run's report
        // (src/gaggle/run_tests.rs).
        let console = console(&record);
        assert!(console.contains("GAGGLE WORKERS") && console.contains("saturated 42 s"));
        assert!(console.contains("Check: load-42"), "{}", console);
    }

    #[test]
    fn the_console_shows_twenty_workers() {
        let record = GaggleRecord {
            workers: (0..25).map(|id| worker(id, &format!("load-{id:02}"))).collect(),
            ..GaggleRecord::default()
        };
        let console = console(&record);
        assert!(console.contains("load-19") && !console.contains("load-20"), "{}", console);
        assert!(console.contains(" and 5 more workers: see the HTML or JSON report"));
    }

    #[test]
    fn a_hostile_worker_name_is_escaped() {
        let name = "<img src=x onerror=alert(1)>\"'&";
        let mut hostile = worker(0, name);
        hostile.flags = vec![GaggleWorkerFlag::Lagging];
        let record = GaggleRecord {
            workers: vec![hostile],
            events: vec![GaggleEvent {
                second: 1,
                time: String::new(),
                kind: GaggleEventKind::WorkerLagging,
                worker: Some(name.to_string()),
                text: format!("{name} lagging (last delta 4 s ago)"),
            }],
            ..GaggleRecord::default()
        };
        let html = html(Some(&record), crate::report::escape_html);
        assert!(!html.contains("<img"), "{}", html);
        let escaped = "&lt;img src=x onerror=alert(1)&gt;&quot;&#39;&amp;";
        assert!(html.contains(&format!("<td>{escaped}</td>")), "{}", html);
        assert!(html.contains(&format!("<td colspan=\"5\">{escaped} lagging")), "{}", html);
    }

    #[test]
    fn a_record_without_the_new_fields_loads() {
        // As a 0.19 or step 2 manager wrote it.
        let json = r#"{"workers":[{"id":0,"name":"w","users":3,"state":"lost","lost_reason":"gone"},
            {"id":1,"name":"v","users":3,"state":"finished"}],"degraded":true}"#;
        let record: GaggleRecord = serde_json::from_str(json).unwrap();
        assert!(record.events.is_empty() && record.events_dropped == 0);
        assert!(record.workers[0].flags.is_empty() && record.workers[0].requests.is_none());
        // Its lost worker still sorts first and shows dashes for the rest.
        let order: Vec<&str> = sorted(&record).iter().map(|w| w.name.as_str()).collect();
        assert_eq!(order, ["w", "v"]);
        assert_eq!(row(&record.workers[1]).cells[3], "-");
        assert!(check_free(&record));
    }

    fn check_free(record: &GaggleRecord) -> bool {
        !record.intro().iter().any(|s| s.starts_with("Check:"))
    }

    #[test]
    fn a_run_of_one_process_has_no_workers_section() {
        assert_eq!(html(None, crate::report::escape_html), "");
    }
}
