# Metrics

When Coordinated Omission Mitigation backfills a request, Goose keeps both the "raw" metrics, the response times it measured, and the "adjusted" metrics, the measured times together with the synthetic ones. Wherever it shows both, the "raw" metrics come first, followed by the "adjusted" metrics. As the minimum response time is never changed by Coordinated Omission Mitigation, the adjusted table replaces that column with a "Std Dev" column: the standard deviation of just two numbers, the "raw" average and the "adjusted" average, which works out to their difference divided by the square root of 2.

The following example was "contrived". The [`drupal_memcache`](../example/drupal-memcache.md) example ran with five users for 20 seconds against a local test server, and 11 seconds into the run the server held every request for 3 seconds, forcing some abnormally slow responses:

```bash
cargo run --release --example drupal_memcache -- --host http://127.0.0.1:8080/ \
    -u5 -r5 -t20s --co-mitigation average --no-telnet --no-websocket \
    --request-log requests.json --request-format json \
    --report-file report.html --report-file report.json --report-file report.md
```

The example names each node and user page request by its path, so the run made requests under about 1,400 names; the tables below keep a few of them and the static assets, and `...` marks the rows left out. In the "PER REQUEST METRICS" Goose shows first the "raw" metrics, followed by the "adjusted" metrics:

```bash
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 GET /                    |          21 |       5.48 |       3,017 |         19
 GET /node/103            |         6.1 |        6.1 |         6.1 |        6.1
 GET /node/1377           |          25 |         25 |          25 |         25
 GET /node/1461           |          11 |         11 |          11 |         11
 ...
 GET /user/143            |       3,010 |      3,010 |       3,010 |      3,010
 ...
 GET static asset         |        7.09 |       1.14 |       3,005 |       5.22
 ...
 POST /comment/reply/1609 |       3,078 |      3,078 |       3,078 |      3,078
 ...
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |          16 |       1.14 |       3,078 |       8.26
 ------------------------------------------------------------------------------
 Adjusted for Coordinated Omission:
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |    Std Dev |         Max |     Median
 ------------------------------------------------------------------------------
 GET /                    |         244 |        157 |       3,017 |         22
 GET /node/103            |       1,350 |        950 |       2,920 |      1,270
 GET /node/1377           |       1,380 |        958 |       2,737 |        922
 GET /node/1461           |       1,347 |        945 |       2,916 |      1,270
 ...
 GET /user/143            |           - |          - |           - |          -
 ...
 GET static asset         |         232 |        159 |       3,005 |       5.86
 ...
 POST /comment/reply/1609 |           - |          - |           - |          -
 ...
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |         239 |        158 |       3,078 |         10
```

`GET /node/103` was requested once, by a user just after the stall, and took 6.1 ms, but the user would have requested it while the server was stalled, so its adjusted times run up to 2,920 ms. A request with no synthetic times, such as the two that were themselves held up by the stall, shows `-` in the adjusted table. The adjusted aggregate covers every request, using the measured times of those without synthetic ones.

The `# reqs` and `req/s` columns of the same section count the synthetic requests too. `GET /node/103` shows 6 requests: the one it measured, four synthetic ones, and the measured request counted a second time because it carried the backfill.

**Note**: It is beyond the scope of Goose to test for statistically significant changes in the right-tail, or other locations, of the distribution of response times. Goose produces the raw data you need to conduct these tests. For detailed statistical analysis, consider using tools like the Kolmogorov-Smirnov or Anderson-Darling tests to compare distributions. Keep in mind that CO-adjusted data is derived from raw data and thus not statistically independent.

Goose also shows percentiles, again first the "raw" metrics followed by the "adjusted" metrics. The console's percentile tables are not part of the final summary in this release, so these come from the HTML report of the same run, written with `--report-file report.html`, rows left out as above:

```text
 Response Time Metrics
 Method | Name                | 50%ile | 60%ile | 70%ile | 80%ile | 90%ile | 95%ile | 99%ile | 100%ile
 GET    | /                   |     19 |     22 |     25 |     28 |     30 |     33 |     36 |   3,017
 GET    | /node/103           |    6.1 |    6.1 |    6.1 |    6.1 |    6.1 |    6.1 |    6.1 |     6.1
 GET    | /node/1377          |     25 |     25 |     25 |     25 |     25 |     25 |     25 |      25
 GET    | /node/1461          |     11 |     11 |     11 |     11 |     11 |     11 |     11 |      11
 GET    | /user/143           |  3,010 |  3,010 |  3,010 |  3,010 |  3,010 |  3,010 |  3,010 |   3,010
 GET    | static asset        |   5.22 |   5.98 |   6.75 |   7.52 |   8.26 |    8.9 |     10 |   3,005
 POST   | /comment/reply/1609 |  3,078 |  3,078 |  3,078 |  3,078 |  3,078 |  3,078 |  3,078 |   3,078
        | Aggregated          |   8.26 |     11 |     17 |     22 |     28 |     31 |     37 |   3,078

 Response Time Metrics With Coordinated Omission Mitigation
 Method | Name                | 50%ile | 60%ile | 70%ile | 80%ile | 90%ile | 95%ile | 99%ile | 100%ile
 GET    | /                   |     22 |     25 |     28 |     31 |  1,090 |  2,040 |  2,867 |   3,017
 GET    | /node/103           |  1,270 |  1,270 |  2,089 |  2,089 |  2,920 |  2,920 |  2,920 |   2,920
 GET    | /node/1377          |    922 |    922 |  1,827 |  1,827 |  2,737 |  2,737 |  2,737 |   2,737
 GET    | /node/1461          |  1,270 |  1,270 |  2,089 |  2,089 |  2,916 |  2,916 |  2,916 |   2,916
 GET    | static asset        |   5.86 |   6.75 |   7.65 |   8.51 |  1,106 |  2,056 |  2,867 |   3,005
        | Aggregated          |     10 |     16 |     23 |     29 |  1,106 |  2,056 |  2,867 |   3,078
```

The "raw" percentiles would suggest that only the slowest 1% of requests for `GET /` and the static assets were slow. Through Coordinated Omission Mitigation we can see that statistically more than 5% of them would have been affected, with the 90th percentile above a second. The HTML report's adjusted table lists only the requests that have synthetic times, followed by the aggregate.

## Where Adjusted Metrics Appear

Adjusted metrics appear once mitigation has added a synthetic time to at least one request:

- **Console**: an "Adjusted for Coordinated Omission" table follows the response time table of the "PER REQUEST METRICS", in the running metrics and the final summary.
- **HTML report**: "Request Metrics With Coordinated Omission Mitigation" (average, standard deviation and maximum) and "Response Time Metrics With Coordinated Omission Mitigation" (percentiles) follow the raw tables. With `--baseline-file`, both show how each value changed. The response time heatmap's **Adjusted** and **Measured** buttons choose whether the synthetic times are counted.
- **Markdown report**: no adjusted tables; it has the event summary described below.
- **JSON report**: `co_request_metrics` and `co_response_metrics` hold the adjusted tables, and each request in `raw_metrics` holds its adjusted timing data in `coordinated_omission_data`.
- **Live dashboard**: the KPI strip, the **Whole run** card and the requests table show adjusted response times once mitigation has recorded an event; see the [dashboard page](../controller/dashboard.md).

## CO Event Tracking

In addition to the raw and adjusted metrics, Goose tracks each Coordinated Omission event. The summary appears whenever mitigation is enabled, even with no events, in the console, HTML, Markdown and JSON reports.

### CO Event Metrics Display

The console shows a "COORDINATED OMISSION METRICS" section after the other tables, in the running metrics and the final summary. From the same run:

```bash
 === COORDINATED OMISSION METRICS ===
 Duration: 21 seconds
 Total CO Events: 296
 Events per minute: 845.71

 Request Breakdown:
   Actual requests: 6620
   Synthetic requests: 760 (10.3%)

 Severity Distribution:
   Minor: 296
```

- **Duration** is the time since the load test started, in whole seconds, and **Events per minute** is the events over that time. Neither restarts when Goose resets its metrics after the users start.
- **Total CO Events** counts requests, not stalls: each request a user made in the run after a stall, and that added synthetic times, is one event. Here five users each ran into the one stall, and their next runs made 296 requests.
- **Actual requests** are the measured requests since the load test started, including those made before the metrics were reset.
- **Synthetic requests** adds up, for each event, the stalled run's length less the cadence and the request's own response time, divided by the cadence and rounded down; the adjusted tables can hold about one more synthetic time per event than this. The percentage is the synthetic requests of all actual and synthetic requests.
- **Severity Distribution** lists only the severities with at least one event, in no fixed order.

The HTML report ends with a "Coordinated Omission Event Metrics" section: a summary table of the total events, events per minute, actual requests and synthetic requests, and a table of the severities with at least one event. The Markdown report has a "Coordinated Omission Metrics" section with the same figures, all four severities, and tables of events by user and by scenario:

```markdown
## Coordinated Omission Metrics

**Overall Metrics:**
- Total CO Events: 296
- Synthetic Requests Generated: 760
- Actual Requests: 6,620
- Synthetic Percentage: 10.3%

**Severity Breakdown:**
- Minor (2-5x): 296
- Moderate (5-10x): 0
- Severe (10-20x): 0
- Critical (>20x): 0
```

The JSON report holds the same summary under `coordinated_omission_metrics`.

### Understanding CO Event Severity

Goose classifies each event by how much longer the stalled run through the transactions took than the expected cadence:

- **Minor (2-5x)**: the run took less than 5 times the cadence; mitigation only starts above 2 times
- **Moderate (5-10x)**: the run took 5 to 10 times the cadence
- **Severe (10-20x)**: the run took 10 to 20 times the cadence
- **Critical (>20x)**: the run took 20 times the cadence or more

### Interpreting Synthetic Request Percentage

The synthetic request percentage tells you how much of your data comes from CO mitigation:

- **<10%**: High confidence in results, minimal CO impact
- **10-30%**: Medium confidence, some CO events occurred
- **30-50%**: Lower confidence, significant CO impact
- **>50%**: Results heavily influenced by synthetic data

### Practical Example: Microservice Testing

Consider testing a microservice with strict 100ms SLA requirements, with a load test of your own:

```bash
# Test with minimum cadence for the most sensitive mitigation
cargo run --release -- \
    --host https://api.example.com \
    --users 50 \
    --run-time 5m \
    --co-mitigation minimum

# Results might show:
# === COORDINATED OMISSION METRICS ===
# Duration: 300 seconds
# Total CO Events: 45
# Events per minute: 9.00
#
# Request Breakdown:
#   Actual requests: 14523
#   Synthetic requests: 892 (5.8%)
#
# Severity Distribution:
#   Minor: 38
#   Moderate: 6
#   Severe: 1
```

This tells you that there were 45 requests that backfilled a stall, and that 5.8% of your measurements are synthetic. The predominance of "Minor" events suggests occasional but not severe stalls.

### Practical Example: Web Application Testing

For a public-facing web application with more tolerance for variance:

```bash
# Test with average cadence for realistic user simulation
cargo run --release -- \
    --host https://webapp.example.com \
    --users 200 \
    --run-time 10m \
    --co-mitigation average

# Results might show:
# === COORDINATED OMISSION METRICS ===
# Duration: 600 seconds
# Total CO Events: 8
# Events per minute: 0.80
#
# Request Breakdown:
#   Actual requests: 28945
#   Synthetic requests: 67 (0.2%)
#
# Severity Distribution:
#   Minor: 5
#   Moderate: 2
#   Severe: 1
```

This shows a much healthier system with only occasional CO events and minimal synthetic data generation (0.2%), indicating the system handled the load well.

### When to Be Concerned

**Red flags in CO metrics:**
- Synthetic request percentage >30%
- High frequency of Severe or Critical events
- Events per minute consistently >10
- Large gaps between raw and adjusted percentiles

**Green flags:**
- Synthetic request percentage <10%
- Mostly Minor events with few Moderate
- Low events per minute (<5)
- Small differences between raw and adjusted metrics

### Using CO Metrics for Capacity Planning

CO event tracking helps with capacity planning:

1. **Identify Breaking Points**: Watch for sudden increases in CO events as load increases
2. **Sensitive Detection**: Use `minimum` mode to backfill any slowdown against the fastest run
3. **Performance Regression**: Compare CO metrics across test runs to detect degradation
4. **Resource Scaling**: CO events often indicate when additional resources are needed

The CO metrics show how often stalls occurred, how severe they were, and how much they changed the results.
