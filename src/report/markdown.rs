use crate::{
    metrics::{format_number, GooseErrorMetricAggregate, ReportData},
    report::{
        common::OrEmpty, RequestMetric, ResponseMetric, ScenarioMetric, StatusCodeMetric,
        TransactionMetric,
    },
    test_plan::TestPlanStepAction,
    GooseError,
};
use chrono::{Local, TimeZone};
use std::io::Write;

struct Markdown<'m, 'w, W: Write> {
    w: &'w mut W,
    data: ReportData<'m>,
}

pub(crate) fn write_markdown_report<W: Write>(
    w: &mut W,
    data: ReportData,
) -> Result<(), GooseError> {
    Markdown { w, data }.write()
}

impl<W: Write> Markdown<'_, '_, W> {
    pub fn write(mut self) -> Result<(), GooseError> {
        self.write_header()?;
        self.write_plan_overview()?;
        self.write_request_metrics()?;
        self.write_response_metrics()?;
        self.write_status_code_metrics()?;
        self.write_transaction_metrics()?;
        self.write_scenario_metrics()?;
        self.write_error_metrics()?;
        self.write_coordinated_omission_metrics()?;

        Ok(())
    }

    fn write_header(&mut self) -> Result<(), GooseError> {
        writeln!(
            self.w,
            r#"
# Goose Attack Report
"#
        )?;

        Ok(())
    }

    fn write_plan_overview(&mut self) -> Result<(), GooseError> {
        write!(
            self.w,
            r#"
## Plan Overview

| Action | Started | Stopped | Elapsed | Users |
| ------ | ------- | ------- | ------- | ----: |
"#
        )?;

        for step in self.data.raw_metrics.history.windows(2) {
            let (seconds, minutes, hours) = self
                .data
                .raw_metrics
                .get_seconds_minutes_hours(&step[0].timestamp, &step[1].timestamp);
            let started = Local
                .timestamp_opt(step[0].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%y-%m-%d %H:%M:%S");
            let stopped = Local
                .timestamp_opt(step[1].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%y-%m-%d %H:%M:%S");

            let users = match &step[0].action {
                // For maintaining just show the current number of users.
                TestPlanStepAction::Maintaining => {
                    format!("{}", step[0].users)
                }
                // For increasing show the current number of users to the new number of users.
                TestPlanStepAction::Increasing => {
                    format!("{} &rarr; {}", step[0].users, step[1].users)
                }
                // For decreasing show the new number of users from the current number of users.
                TestPlanStepAction::Decreasing | TestPlanStepAction::Canceling => {
                    format!("{} &larr; {}", step[1].users, step[0].users,)
                }
                TestPlanStepAction::Finished => {
                    unreachable!("there shouldn't be a step after finished");
                }
            };

            writeln!(
                self.w,
                r#"| {action:?} | {started} | {stopped} | {hours:02}:{minutes:02}:{seconds:02} | {users} |"#,
                action = step[0].action,
            )?;
        }

        Ok(())
    }

    fn write_request_metrics(&mut self) -> Result<(), GooseError> {
        write!(
            self.w,
            r#"
## Request Metrics

| Method | Name | # Requests | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ------ | ---- | ---------: | ------: | -----------: | -------: | -------: | --: | ---------: |
"#
        )?;

        for RequestMetric {
            method,
            name,
            number_of_requests,
            number_of_failures,
            response_time_average,
            response_time_minimum,
            response_time_maximum,
            requests_per_second,
            failures_per_second,
        } in &self.data.raw_request_metrics
        {
            writeln!(
                self.w,
                r#"| {method} | {name} | {number_of_requests} | {number_of_failures } | {response_time_average:.2 } | {response_time_minimum} | {response_time_maximum} | {requests_per_second:.2} | {failures_per_second:.2} |"#,
            )?;
        }

        Ok(())
    }

    fn write_response_metrics(&mut self) -> Result<(), GooseError> {
        write!(
            self.w,
            r#"
## Response Time Metrics

| Method | Name | 50%ile (ms) | 60%ile (ms) | 70%ile (ms) | 80%ile (ms) | 90%ile (ms) | 95%ile (ms) | 99%ile (ms) | 100%ile (ms) |
| ------ | ---- | ----------: | ----------: | ----------: | ----------: | ----------: | ----------: | ----------: | -----------: |
"#
        )?;

        for ResponseMetric {
            method,
            name,
            percentile_50,
            percentile_60,
            percentile_70,
            percentile_80,
            percentile_90,
            percentile_95,
            percentile_99,
            percentile_100,
            is_breakdown: _,
        } in &self.data.raw_response_metrics
        {
            writeln!(
                self.w,
                r#"| {method} | {name} | {percentile_50} | {percentile_60 } | {percentile_70 } | {percentile_80} | {percentile_90} | {percentile_95} | {percentile_99} | {percentile_100} |"#,
                percentile_50 = format_number(*percentile_50),
                percentile_60 = format_number(*percentile_60),
                percentile_70 = format_number(*percentile_70),
                percentile_80 = format_number(*percentile_80),
                percentile_90 = format_number(*percentile_90),
                percentile_95 = format_number(*percentile_95),
                percentile_99 = format_number(*percentile_99),
                percentile_100 = format_number(*percentile_100),
            )?;
        }

        Ok(())
    }

    fn write_status_code_metrics(&mut self) -> Result<(), GooseError> {
        let Some(status_code_metrics) = &self.data.status_code_metrics else {
            return Ok(());
        };

        write!(
            self.w,
            r#"
## Status Code Metrics

| Method | Name | Status Codes |
| ------ | ---- | ------------ |
"#
        )?;

        for StatusCodeMetric {
            method,
            name,
            status_codes,
        } in status_code_metrics
        {
            writeln!(self.w, r#"| {method} | {name} | {status_codes} |"#)?;
        }

        Ok(())
    }

    fn write_transaction_metrics(&mut self) -> Result<(), GooseError> {
        let Some(transaction_metrics) = &self.data.transaction_metrics else {
            return Ok(());
        };

        write!(
            self.w,
            r#"
## Transaction Metrics

| Transaction | # Times Run | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ----------- | ----------: | ------: | -----------: | -------: | -------: | --: | ---------: |
"#
        )?;

        for TransactionMetric {
            is_scenario,
            transaction,
            name,
            number_of_requests,
            number_of_failures,
            response_time_average,
            response_time_minimum,
            response_time_maximum,
            requests_per_second,
            failures_per_second,
        } in transaction_metrics
        {
            match is_scenario {
                true => writeln!(self.w, r#"| **{name}** |"#)?,
                false => writeln!(
                    self.w,
                    r#"| {transaction} {name} | {number_of_requests} | {number_of_failures} | {response_time_average:.2} | {response_time_minimum} | {response_time_maximum} | {requests_per_second:.2} | {failures_per_second:.2} |"#,
                    response_time_average = OrEmpty(*response_time_average),
                    requests_per_second = OrEmpty(*requests_per_second),
                    failures_per_second = OrEmpty(*failures_per_second),
                )?,
            }
        }

        Ok(())
    }

    fn write_scenario_metrics(&mut self) -> Result<(), GooseError> {
        let Some(scenario_metrics) = &self.data.scenario_metrics else {
            return Ok(());
        };

        write!(
            self.w,
            r#"
## Scenario Metrics

| Transaction | # Users | # Times Run | Average (ms) | Min (ms) | Max (ms) | Scenarios/s | Iterations |
| ----------- | ------: | ----------: | -----------: | -------: | -------: | ----------: | ---------: |
"#
        )?;

        for ScenarioMetric {
            name,
            users,
            count,
            response_time_average,
            response_time_minimum,
            response_time_maximum,
            count_per_second,
            iterations,
        } in scenario_metrics
        {
            writeln!(
                self.w,
                r#"| {name} | {users} | {count} | {response_time_average:.2} | {response_time_minimum}  | {response_time_maximum} | {count_per_second:.2} | {iterations:.2} |"#
            )?;
        }

        Ok(())
    }

    fn write_error_metrics(&mut self) -> Result<(), GooseError> {
        let Some(errors) = &self.data.errors else {
            return Ok(());
        };

        write!(
            self.w,
            r#"
## Error Metrics

| Method | Name |  #  | Error |
| ------ | ---- | --: | ----- |
"#
        )?;

        for GooseErrorMetricAggregate {
            method,
            name,
            error,
            occurrences,
        } in errors
        {
            writeln!(self.w, r#"| {method} | {name} | {occurrences} | {error} |"#)?;
        }

        Ok(())
    }

    fn write_coordinated_omission_metrics(&mut self) -> Result<(), GooseError> {
        let Some(co_metrics) = &self.data.coordinated_omission_metrics else {
            return Ok(());
        };

        write!(
            self.w,
            r#"
## Coordinated Omission Metrics

"#
        )?;

        // Overall metrics
        writeln!(self.w, "**Overall Metrics:**")?;
        writeln!(self.w, "- Total CO Events: {}", co_metrics.total_co_events)?;
        writeln!(
            self.w,
            "- Synthetic Requests Generated: {}",
            format_number(co_metrics.synthetic_requests as usize)
        )?;
        writeln!(
            self.w,
            "- Actual Requests: {}",
            format_number(co_metrics.actual_requests as usize)
        )?;
        writeln!(
            self.w,
            "- Synthetic Percentage: {:.1}%",
            co_metrics.synthetic_percentage
        )?;
        writeln!(self.w)?;

        // Severity breakdown
        writeln!(self.w, "**Severity Breakdown:**")?;
        writeln!(self.w, "- Minor (2-5x): {}", co_metrics.minor_count)?;
        writeln!(self.w, "- Moderate (5-10x): {}", co_metrics.moderate_count)?;
        writeln!(self.w, "- Severe (10-20x): {}", co_metrics.severe_count)?;
        writeln!(self.w, "- Critical (>20x): {}", co_metrics.critical_count)?;
        writeln!(self.w)?;

        // Per-user breakdown if available
        if !co_metrics.per_user_events.is_empty() {
            writeln!(self.w, "**Per-User CO Events:**")?;
            writeln!(self.w)?;
            writeln!(self.w, "| User | CO Events | Severity Breakdown |")?;
            writeln!(self.w, "| ---: | --------: | ------------------ |")?;

            for (user_id, count, severity) in &co_metrics.per_user_events {
                writeln!(self.w, "| {user_id} | {count} | {severity} |")?;
            }
            writeln!(self.w)?;
        }

        // Per-scenario breakdown if available
        if !co_metrics.per_scenario_events.is_empty() {
            writeln!(self.w, "**Per-Scenario CO Events:**")?;
            writeln!(self.w)?;
            writeln!(self.w, "| Scenario | CO Events | Synthetic Requests |")?;
            writeln!(self.w, "| -------- | --------: | -----------------: |")?;

            for (scenario, count, synthetic) in &co_metrics.per_scenario_events {
                writeln!(
                    self.w,
                    "| {} | {} | {} |",
                    scenario,
                    count,
                    format_number(*synthetic)
                )?;
            }
        }

        Ok(())
    }
}
