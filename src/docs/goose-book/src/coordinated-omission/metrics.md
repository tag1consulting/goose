# Metrics

When Coordinated Omission Mitigation backfills a request, Goose keeps both the "raw" metrics, the response times it measured, and the "adjusted" metrics, the measured times together with the synthetic ones. Wherever it shows both, the "raw" metrics come first, followed by the "adjusted" metrics. As the minimum response time is never changed by Coordinated Omission Mitigation, the adjusted table replaces that column with a "Std Dev" column: the standard deviation of just two numbers, the "raw" average and the "adjusted" average, which works out to their difference divided by the square root of 2.

The following example was "contrived". The [`drupal_memcache`](../example/drupal-memcache.md) example ran with five users for 20 seconds against a local test server that answers each request in 5 to 25 ms, and 11 seconds into the run the server held every request for 3 seconds, forcing some abnormally slow responses:

```bash
cargo run --release --example drupal_memcache -- --host http://127.0.0.1:8080/ \
    -u5 -r5 -t20s --co-mitigation average --no-telnet \
    --request-log requests.json --request-format json \
    --report-file report.html --report-file report.json --report-file report.md
```

The example names each node and user page request by its path, so the run made requests under about 1,150 names; the tables below keep a few of them and the static assets, and `...` marks the rows left out. In the "PER REQUEST METRICS" Goose shows first the "raw" metrics, followed by the "adjusted" metrics:

```bash
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 GET /                    |          19 |       5.85 |       3,026 |         17
 GET /node/101            |        8.83 |       8.83 |        8.83 |       8.83
 ...
 GET /node/1568           |          25 |         25 |          25 |         25
 ...
 GET /node/1814           |          28 |         28 |          28 |         28
 ...
 GET static asset         |          22 |       5.27 |       3,046 |         17
 POST /comment/reply/1088 |          14 |         14 |          14 |         14
 ...
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |          20 |       5.27 |       3,046 |         17
 ------------------------------------------------------------------------------
 Adjusted for Coordinated Omission:
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |    Std Dev |         Max |     Median
 ------------------------------------------------------------------------------
 GET /                    |         254 |        166 |       3,026 |         19
 GET /node/101            |           - |          - |           - |          -
 ...
 GET /node/1568           |       1,322 |        917 |       2,894 |        610
 ...
 GET /node/1814           |       1,073 |        739 |       2,744 |         98
 ...
 GET static asset         |         255 |        165 |       3,046 |         19
 POST /comment/reply/1088 |           - |          - |           - |          -
 ...
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |         254 |        166 |       3,046 |         19
```

`GET /node/1568` was requested once, by a user just after the stall, and took 25 ms, but the user would have requested it while the server was stalled, so its adjusted times run up to 2,894 ms. A request with no synthetic times, such as `GET /node/101`, requested before the stall, shows `-` in the adjusted table. The `GET /` and static asset requests held up by the stall itself took about 3 seconds. The adjusted aggregate covers every request, using the measured times of those without synthetic ones.

The `# reqs` and `req/s` columns of the same section count the synthetic requests too. `GET /node/1568` shows 4 requests: the one it measured and three synthetic ones. The Aggregated `# reqs`, 5,930, is the 5,042 actual requests and the 888 synthetic requests counted in the summary below.

**Note**: It is beyond the scope of Goose to test for statistically significant changes in the right-tail, or other locations, of the distribution of response times. Goose produces the raw data you need to conduct these tests. For detailed statistical analysis, consider using tools like the Kolmogorov-Smirnov or Anderson-Darling tests to compare distributions. Keep in mind that CO-adjusted data is derived from raw data and thus not statistically independent.

Goose also shows percentiles, again first the "raw" metrics followed by the "adjusted" metrics. From the same run's final summary, rows left out as above:

```bash
 ------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
 ------------------------------------------------------------------------------
 GET /                    |     17 |     22 |     27 |     27 |     29 |   3026
 GET /node/101            |   8.83 |   8.83 |   8.83 |   8.83 |   8.83 |   8.83
 ...
 GET /node/1568           |     25 |     25 |     25 |     25 |     25 |     25
 ...
 GET /node/1814           |     28 |     28 |     28 |     28 |     28 |     28
 ...
 GET static asset         |     17 |     22 |     27 |     27 |   3031 |   3046
 POST /comment/reply/1088 |     14 |     14 |     14 |     14 |     14 |     14
 ...
 -------------------------+--------+--------+--------+--------+--------+-------
 Aggregated               |     17 |     22 |     27 |     28 |     29 |   3031
 ------------------------------------------------------------------------------
 Adjusted for Coordinated Omission:
 ------------------------------------------------------------------------------
 Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
 ------------------------------------------------------------------------------
 GET /                    |     19 |     25 |   2867 |   2900 |   2900 |   3026
 GET /node/101            |      - |      - |      - |      - |      - |      -
 ...
 GET /node/1568           |    610 |   1761 |   2894 |   2894 |   2894 |   2894
 ...
 GET /node/1814           |     98 |   1417 |   2744 |   2744 |   2744 |   2744
 ...
 GET static asset         |     19 |     25 |   2867 |   2900 |   3031 |   3046
 POST /comment/reply/1088 |      - |      - |      - |      - |      - |      -
 ...
 -------------------------+--------+--------+--------+--------+--------+-------
 Aggregated               |     19 |     25 |   2867 |   2900 |   2900 |   3031
```

The "raw" percentiles would suggest that well under 1% of requests for `GET /` and the static assets were slow. Through Coordinated Omission Mitigation we can see that statistically more than 2% of them would have been affected, with the 98th percentile near 3 seconds. The HTML report's adjusted table lists only the requests that have synthetic times, followed by the aggregate.

## Where Adjusted Metrics Appear

Adjusted metrics appear once mitigation has added a synthetic time to at least one request:

- **Console**: an "Adjusted for Coordinated Omission" table follows the response time table of the "PER REQUEST METRICS", in the running metrics and the final summary, and in the final summary another follows its percentile table.
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
 Duration: 20 seconds
 Total CO Events: 296
 Events per minute: 888.00

 Request Breakdown:
   Actual requests: 5042
   Synthetic requests: 888 (15.0%)

 Severity Distribution:
   Minor: 296
```

- **Duration** is the time in whole seconds since the counts started, and **Events per minute** is the events over that time.
- **Total CO Events** counts requests, not stalls: each request a user made in the run after a stall, and that added synthetic times, is one event. Here five users each ran into the one stall, and their next runs made 296 requests.
- **Actual requests** are the requests Goose measured.
- **Synthetic requests** are the synthetic times mitigation added to the adjusted tables. For each event they are the stalled run's length less the cadence and the request's own response time, then less the cadence again and again, for as long as what is left is longer than the request's own response time. The percentage is the synthetic requests of all actual and synthetic requests.
- Actual requests plus synthetic requests is the Aggregated `# reqs` of the "PER REQUEST METRICS".
- All of these start again when Goose resets its metrics after the users start, as the other metrics do, so they leave out the ramp up. With `--no-reset-metrics` they cover the whole run.
- **Severity Distribution** lists only the severities with at least one event, in no fixed order.

The HTML report ends with a "Coordinated Omission Event Metrics" section: a summary table of the total events, events per minute, actual requests and synthetic requests, and a table of the severities with at least one event. The Markdown report has a "Coordinated Omission Metrics" section with the same figures, all four severities, and tables of events by user and by scenario:

```markdown
## Coordinated Omission Metrics

**Overall Metrics:**
- Total CO Events: 296
- Synthetic Requests Generated: 888
- Actual Requests: 5,042
- Synthetic Percentage: 15.0%

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
