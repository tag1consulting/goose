//! Enhanced Coordinated Omission metrics tracking.

use crate::metrics::GooseCoordinatedOmissionMitigation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Enhanced metrics for tracking Coordinated Omission events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatedOmissionMetrics {
    /// Total number of actual requests made.
    pub actual_requests: u64,
    /// Total number of synthetic requests generated due to CO mitigation.
    pub synthetic_requests: u64,
    /// Percentage of synthetic requests vs total requests.
    pub synthetic_percentage: f32,
    /// List of CO events detected during the test.
    pub co_events: Vec<CoEvent>,
    /// Severity distribution of CO events.
    pub severity_histogram: HashMap<CoSeverity, usize>,
    /// Current mitigation strategy being used.
    pub mitigation_strategy: GooseCoordinatedOmissionMitigation,
    /// Timestamp when metrics collection started (seconds since UNIX epoch).
    pub started_secs: u64,
}

/// Represents a single Coordinated Omission event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoEvent {
    /// When the CO event was detected (seconds since UNIX epoch).
    pub timestamp_secs: u64,
    /// The expected cadence at the time of detection.
    pub expected_cadence: Duration,
    /// The actual duration that triggered CO detection.
    pub actual_duration: Duration,
    /// Number of synthetic requests injected for this event.
    pub synthetic_injected: u32,
    /// The user/thread that experienced the CO event.
    pub user_id: usize,
    /// The scenario that was running when CO occurred.
    pub scenario_name: String,
    /// Severity level of this CO event.
    pub severity: CoSeverity,
}

/// Severity levels for Coordinated Omission events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CoSeverity {
    /// Minor: actual duration is 2-5x expected cadence
    Minor,
    /// Moderate: actual duration is 5-10x expected cadence
    Moderate,
    /// Severe: actual duration is 10-20x expected cadence
    Severe,
    /// Critical: actual duration is >20x expected cadence
    Critical,
}

impl CoordinatedOmissionMetrics {
    /// Create a new CoordinatedOmissionMetrics instance.
    pub fn new(mitigation_strategy: GooseCoordinatedOmissionMitigation) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        CoordinatedOmissionMetrics {
            actual_requests: 0,
            synthetic_requests: 0,
            synthetic_percentage: 0.0,
            co_events: Vec::new(),
            severity_histogram: HashMap::new(),
            mitigation_strategy,
            started_secs: now,
        }
    }

    /// Record an actual request.
    pub fn record_actual_request(&mut self) {
        self.actual_requests += 1;
        self.update_synthetic_percentage();
    }

    /// Record synthetic requests generated for CO mitigation.
    pub fn record_synthetic_requests(&mut self, count: u32) {
        self.synthetic_requests += count as u64;
        self.update_synthetic_percentage();
    }

    /// Record a new CO event.
    pub fn record_co_event(
        &mut self,
        expected_cadence: Duration,
        actual_duration: Duration,
        synthetic_injected: u32,
        user_id: usize,
        scenario_name: String,
    ) {
        let severity = Self::calculate_severity(expected_cadence, actual_duration);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        let event = CoEvent {
            timestamp_secs: now,
            expected_cadence,
            actual_duration,
            synthetic_injected,
            user_id,
            scenario_name,
            severity,
        };

        // Update severity histogram
        *self.severity_histogram.entry(severity).or_insert(0) += 1;

        // Add event to list
        self.co_events.push(event);

        // Record the synthetic requests
        self.record_synthetic_requests(synthetic_injected);
    }

    /// Calculate the severity of a CO event based on the duration ratio.
    fn calculate_severity(expected: Duration, actual: Duration) -> CoSeverity {
        let ratio = actual.as_millis() as f64 / expected.as_millis() as f64;

        match ratio {
            r if r < 2.0 => CoSeverity::Minor, // Shouldn't happen, but handle edge case
            r if r < 5.0 => CoSeverity::Minor,
            r if r < 10.0 => CoSeverity::Moderate,
            r if r < 20.0 => CoSeverity::Severe,
            _ => CoSeverity::Critical,
        }
    }

    /// Update the synthetic percentage calculation.
    fn update_synthetic_percentage(&mut self) {
        let total = self.actual_requests + self.synthetic_requests;
        if total > 0 {
            self.synthetic_percentage = (self.synthetic_requests as f32 / total as f32) * 100.0;
        } else {
            self.synthetic_percentage = 0.0;
        }
    }

    /// Get a summary of CO metrics for reporting.
    pub fn get_summary(&self) -> CoMetricsSummary {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        let duration_secs = now.saturating_sub(self.started_secs);

        // Calculate individual severity counts
        let minor_count = *self
            .severity_histogram
            .get(&CoSeverity::Minor)
            .unwrap_or(&0);
        let moderate_count = *self
            .severity_histogram
            .get(&CoSeverity::Moderate)
            .unwrap_or(&0);
        let severe_count = *self
            .severity_histogram
            .get(&CoSeverity::Severe)
            .unwrap_or(&0);
        let critical_count = *self
            .severity_histogram
            .get(&CoSeverity::Critical)
            .unwrap_or(&0);

        // Calculate per-user events
        let mut user_events: HashMap<usize, (usize, HashMap<CoSeverity, usize>)> = HashMap::new();
        for event in &self.co_events {
            let (count, severity_map) = user_events
                .entry(event.user_id)
                .or_insert((0, HashMap::new()));
            *count += 1;
            *severity_map.entry(event.severity).or_insert(0) += 1;
        }

        let per_user_events: Vec<(usize, usize, String)> = user_events
            .into_iter()
            .map(|(user_id, (count, severity_map))| {
                let severity_breakdown = format!(
                    "Minor: {}, Moderate: {}, Severe: {}, Critical: {}",
                    severity_map.get(&CoSeverity::Minor).unwrap_or(&0),
                    severity_map.get(&CoSeverity::Moderate).unwrap_or(&0),
                    severity_map.get(&CoSeverity::Severe).unwrap_or(&0),
                    severity_map.get(&CoSeverity::Critical).unwrap_or(&0)
                );
                (user_id, count, severity_breakdown)
            })
            .collect();

        // Calculate per-scenario events
        let mut scenario_events: HashMap<String, (usize, u32)> = HashMap::new();
        for event in &self.co_events {
            let (count, synthetic) = scenario_events
                .entry(event.scenario_name.clone())
                .or_insert((0, 0));
            *count += 1;
            *synthetic += event.synthetic_injected;
        }

        let per_scenario_events: Vec<(String, usize, usize)> = scenario_events
            .into_iter()
            .map(|(scenario, (count, synthetic))| (scenario, count, synthetic as usize))
            .collect();

        CoMetricsSummary {
            total_co_events: self.co_events.len(),
            actual_requests: self.actual_requests,
            synthetic_requests: self.synthetic_requests,
            synthetic_percentage: self.synthetic_percentage,
            severity_breakdown: self.severity_histogram.clone(),
            duration_secs,
            events_per_minute: self.calculate_events_per_minute(),
            minor_count,
            moderate_count,
            severe_count,
            critical_count,
            per_user_events,
            per_scenario_events,
        }
    }

    /// Calculate CO events per minute rate.
    fn calculate_events_per_minute(&self) -> f64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        let duration_secs = now.saturating_sub(self.started_secs);
        let duration_minutes = duration_secs as f64 / 60.0;

        if duration_minutes > 0.0 {
            self.co_events.len() as f64 / duration_minutes
        } else {
            0.0
        }
    }

    /// Get events filtered by severity.
    pub fn get_events_by_severity(&self, severity: CoSeverity) -> Vec<&CoEvent> {
        self.co_events
            .iter()
            .filter(|event| event.severity == severity)
            .collect()
    }

    /// Check if synthetic data percentage exceeds threshold.
    pub fn is_synthetic_threshold_exceeded(&self, threshold: f32) -> bool {
        self.synthetic_percentage > threshold
    }
}

/// Summary structure for CO metrics reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoMetricsSummary {
    pub total_co_events: usize,
    pub actual_requests: u64,
    pub synthetic_requests: u64,
    pub synthetic_percentage: f32,
    pub severity_breakdown: HashMap<CoSeverity, usize>,
    pub duration_secs: u64,
    pub events_per_minute: f64,
    // Individual severity counts for easier access
    pub minor_count: usize,
    pub moderate_count: usize,
    pub severe_count: usize,
    pub critical_count: usize,
    // Per-user and per-scenario breakdowns
    pub per_user_events: Vec<(usize, usize, String)>, // (user_id, count, severity_breakdown)
    pub per_scenario_events: Vec<(String, usize, usize)>, // (scenario, count, synthetic_requests)
}

impl std::fmt::Display for CoMetricsSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\n === COORDINATED OMISSION METRICS ===")?;
        writeln!(f, " Duration: {} seconds", self.duration_secs)?;
        writeln!(f, " Total CO Events: {}", self.total_co_events)?;
        writeln!(f, " Events per minute: {:.2}", self.events_per_minute)?;
        writeln!(f, "\n Request Breakdown:")?;
        writeln!(f, "   Actual requests: {}", self.actual_requests)?;
        writeln!(
            f,
            "   Synthetic requests: {} ({:.1}%)",
            self.synthetic_requests, self.synthetic_percentage
        )?;

        if !self.severity_breakdown.is_empty() {
            writeln!(f, "\n Severity Distribution:")?;
            for (severity, count) in &self.severity_breakdown {
                writeln!(f, "   {severity:?}: {count}")?;
            }
        }

        Ok(())
    }
}

/// Display implementation for CoordinatedOmissionMetrics
impl std::fmt::Display for CoordinatedOmissionMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let summary = self.get_summary();
        write!(f, "{summary}")
    }
}

/// Trait for different cadence calculation strategies.
pub trait CadenceCalculator: Send + Sync {
    /// Get the name of this calculator strategy.
    fn name(&self) -> &str;

    /// Calculate the baseline cadence from measurements.
    fn calculate_baseline(&mut self, measurements: &[Duration]) -> Duration;

    /// Determine if synthetic data should be injected based on elapsed time.
    fn should_inject_synthetic(&self, elapsed: Duration, baseline: Duration) -> bool;

    /// Describe the approach used by this calculator.
    fn describe_approach(&self) -> &str;
}

/// Minimum cadence calculator (most sensitive to CO).
pub struct MinimumCadence {
    warmup_iterations: u32,
    measurements_seen: u32,
}

impl MinimumCadence {
    pub fn new(warmup_iterations: u32) -> Self {
        MinimumCadence {
            warmup_iterations,
            measurements_seen: 0,
        }
    }
}

impl CadenceCalculator for MinimumCadence {
    fn name(&self) -> &str {
        "minimum"
    }

    fn calculate_baseline(&mut self, measurements: &[Duration]) -> Duration {
        self.measurements_seen += measurements.len() as u32;

        if self.measurements_seen < self.warmup_iterations {
            // During warmup, return a very high duration to avoid false positives
            Duration::from_secs(3600)
        } else {
            measurements
                .iter()
                .min()
                .copied()
                .unwrap_or(Duration::from_secs(1))
        }
    }

    fn should_inject_synthetic(&self, elapsed: Duration, baseline: Duration) -> bool {
        elapsed > baseline * 2
    }

    fn describe_approach(&self) -> &str {
        "Uses minimum response time as baseline. Most sensitive to CO events."
    }
}

/// Average cadence calculator (balanced approach).
pub struct AverageCadence {
    warmup_iterations: u32,
    deviation_threshold: f64,
    measurements_seen: u32,
}

impl AverageCadence {
    pub fn new(warmup_iterations: u32, deviation_threshold: f64) -> Self {
        AverageCadence {
            warmup_iterations,
            deviation_threshold,
            measurements_seen: 0,
        }
    }
}

impl CadenceCalculator for AverageCadence {
    fn name(&self) -> &str {
        "average"
    }

    fn calculate_baseline(&mut self, measurements: &[Duration]) -> Duration {
        self.measurements_seen += measurements.len() as u32;

        if self.measurements_seen < self.warmup_iterations {
            // During warmup, return a very high duration to avoid false positives
            Duration::from_secs(3600)
        } else if measurements.is_empty() {
            Duration::from_secs(1)
        } else {
            let sum: Duration = measurements.iter().sum();
            sum / measurements.len() as u32
        }
    }

    fn should_inject_synthetic(&self, elapsed: Duration, baseline: Duration) -> bool {
        let ratio = elapsed.as_millis() as f64 / baseline.as_millis() as f64;
        ratio > self.deviation_threshold
    }

    fn describe_approach(&self) -> &str {
        "Uses average response time as baseline. Balanced approach for most scenarios."
    }
}

/// Maximum cadence calculator (least sensitive to CO).
pub struct MaximumCadence {
    warmup_iterations: u32,
    measurements_seen: u32,
}

impl MaximumCadence {
    pub fn new(warmup_iterations: u32) -> Self {
        MaximumCadence {
            warmup_iterations,
            measurements_seen: 0,
        }
    }
}

impl CadenceCalculator for MaximumCadence {
    fn name(&self) -> &str {
        "maximum"
    }

    fn calculate_baseline(&mut self, measurements: &[Duration]) -> Duration {
        self.measurements_seen += measurements.len() as u32;

        if self.measurements_seen < self.warmup_iterations {
            // During warmup, return a very high duration to avoid false positives
            Duration::from_secs(3600)
        } else {
            measurements
                .iter()
                .max()
                .copied()
                .unwrap_or(Duration::from_secs(1))
        }
    }

    fn should_inject_synthetic(&self, elapsed: Duration, baseline: Duration) -> bool {
        elapsed > baseline * 2
    }

    fn describe_approach(&self) -> &str {
        "Uses maximum response time as baseline. Least sensitive to CO events."
    }
}

/// Percentile-based cadence calculator.
pub struct PercentileCadence {
    percentile: f64,
    warmup_iterations: u32,
    measurements_seen: u32,
}

impl PercentileCadence {
    pub fn new(percentile: f64, warmup_iterations: u32) -> Self {
        PercentileCadence {
            percentile,
            warmup_iterations,
            measurements_seen: 0,
        }
    }
}

impl CadenceCalculator for PercentileCadence {
    fn name(&self) -> &str {
        "percentile"
    }

    fn calculate_baseline(&mut self, measurements: &[Duration]) -> Duration {
        self.measurements_seen += measurements.len() as u32;

        if self.measurements_seen < self.warmup_iterations {
            Duration::from_secs(3600)
        } else if measurements.is_empty() {
            Duration::from_secs(1)
        } else {
            let mut sorted: Vec<Duration> = measurements.to_vec();
            sorted.sort();

            let index = ((sorted.len() as f64 - 1.0) * self.percentile) as usize;
            sorted[index]
        }
    }

    fn should_inject_synthetic(&self, elapsed: Duration, baseline: Duration) -> bool {
        elapsed > baseline * 2
    }

    fn describe_approach(&self) -> &str {
        "Uses configurable percentile of response times as baseline."
    }
}

/// Factory function to create cadence calculator based on configuration.
pub fn create_cadence_calculator(
    mitigation: &GooseCoordinatedOmissionMitigation,
    warmup_iterations: u32,
) -> Box<dyn CadenceCalculator> {
    match mitigation {
        GooseCoordinatedOmissionMitigation::Average => {
            Box::new(AverageCadence::new(warmup_iterations, 2.0))
        }
        GooseCoordinatedOmissionMitigation::Minimum => {
            Box::new(MinimumCadence::new(warmup_iterations))
        }
        GooseCoordinatedOmissionMitigation::Maximum => {
            Box::new(MaximumCadence::new(warmup_iterations))
        }
        GooseCoordinatedOmissionMitigation::Disabled => {
            // Return a dummy calculator that never triggers
            Box::new(DisabledCadence {})
        }
    }
}

/// Disabled cadence calculator (never triggers CO mitigation).
struct DisabledCadence;

impl CadenceCalculator for DisabledCadence {
    fn name(&self) -> &str {
        "disabled"
    }

    fn calculate_baseline(&mut self, _measurements: &[Duration]) -> Duration {
        Duration::from_secs(u64::MAX)
    }

    fn should_inject_synthetic(&self, _elapsed: Duration, _baseline: Duration) -> bool {
        false
    }

    fn describe_approach(&self) -> &str {
        "Coordinated Omission mitigation is disabled."
    }
}
