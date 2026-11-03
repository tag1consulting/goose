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
