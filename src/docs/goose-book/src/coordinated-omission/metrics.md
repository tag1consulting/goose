# Metrics

When Coordinated Omission Mitigation kicks in, Goose tracks both the "raw" metrics and the "adjusted" metrics. It shows both together when displaying metrics, first the "raw" (actually seen) metrics, followed by the "adjusted" metrics. As the minimum response time is never changed by Coordinated Omission Mitigation, this column is replacd with the "standard deviation" between the average "raw" response time, and the average "adjusted" response time.

The following example was "contrived". The [`drupal_memcache`](../example/drupal-memcache.md) example was run for 15 seconds, and after 10 seconds the upstream Apache server was manually "paused" for 3 seconds, forcing some abnormally slow queries. (More specifically, the apache web server was started by running `. /etc/apache2/envvars && /usr/sbin/apache2 -DFOREGROUND`, it was "paused" by pressing `ctrl-z`, and it was resumed three seconds later by typing `fg`.) In the "PER REQUEST METRICS" Goose shows first the "raw" metrics", followed by the "adjusted" metrics:

```bash
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 GET (Anon) front page    |       11.73 |          3 |          81 |         12
 GET (Anon) node page     |       81.76 |          5 |       3,390 |         37
 GET (Anon) user page     |       27.53 |         16 |          94 |         26
 GET (Auth) comment form  |       35.27 |         24 |          50 |         35
 GET (Auth) front page    |       30.68 |         20 |         111 |         26
 GET (Auth) node page     |       97.79 |         23 |       3,326 |         35
 GET (Auth) user page     |       25.20 |         21 |          30 |         25
 GET static asset         |        9.27 |          2 |          98 |          6
 POST (Auth) comment form |       52.47 |         43 |          59 |         52
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |       17.04 |          2 |       3,390 |          8
 ------------------------------------------------------------------------------
 Adjusted for Coordinated Omission:
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |    Std Dev |         Max |     Median
 ------------------------------------------------------------------------------
 GET (Anon) front page    |      419.82 |     288.56 |       3,153 |         14
 GET (Anon) node page     |      464.72 |     270.80 |       3,390 |         40
 GET (Anon) user page     |      420.48 |     277.86 |       3,133 |         27
 GET (Auth) comment form  |      503.38 |     331.01 |       2,951 |         37
 GET (Auth) front page    |      489.99 |     324.78 |       2,960 |         33
 GET (Auth) node page     |      530.29 |     305.82 |       3,326 |         37
 GET (Auth) user page     |      500.67 |     336.21 |       2,959 |         27
 GET static asset         |      427.70 |     295.87 |       3,154 |          9
 POST (Auth) comment form |      512.14 |     325.04 |       2,932 |         55
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |      432.98 |     294.11 |       3,390 |         14
 ```

From these two tables, we can observe a notable difference between the raw and adjusted metrics. The standard deviation between the "raw" average and the "adjusted" average is considerably larger than the "raw" average, indicating that a performance event occurred that affected request timing. Whether this indicates a "valid" load test depends on your specific goals and testing context.

**Note**: It is beyond the scope of Goose to test for statistically significant changes in the right-tail, or other locations, of the distribution of response times. Goose produces the raw data you need to conduct these tests. For detailed statistical analysis, consider using tools like the Kolmogorov-Smirnov or Anderson-Darling tests to compare distributions. Keep in mind that CO-adjusted data is derived from raw data and thus not statistically independent.

Goose also shows multiple percentile graphs, again showing first the "raw" metrics followed by the "adjusted" metrics. The "raw" graph would suggest that less than 1% of the requests for the `GET (Anon) node page` were slow, and less than 0.1% of the requests for the `GET (Auth) node page` were slow. However, through Coordinated Omission Mitigation we can see that statistically this would have actually affected all requests, and for authenticated users the impact is visible on >25% of the requests.

```bash
 ------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
 ------------------------------------------------------------------------------
 GET (Anon) front page    |     12 |     15 |     25 |     27 |     81 |     81
 GET (Anon) node page     |     37 |     43 |     60 |  3,000 |  3,000 |  3,000
 GET (Anon) user page     |     26 |     28 |     34 |     93 |     94 |     94
 GET (Auth) comment form  |     35 |     37 |     50 |     50 |     50 |     50
 GET (Auth) front page    |     26 |     34 |     45 |     88 |    110 |    110
 GET (Auth) node page     |     35 |     38 |     58 |     58 |  3,000 |  3,000
 GET (Auth) user page     |     25 |     27 |     30 |     30 |     30 |     30
 GET static asset         |      6 |     14 |     21 |     22 |     81 |     98
 POST (Auth) comment form |     52 |     55 |     59 |     59 |     59 |     59
 -------------------------+--------+--------+--------+--------+--------+-------
 Aggregated               |      8 |     16 |     47 |     53 |  3,000 |  3,000
 ------------------------------------------------------------------------------
 Adjusted for Coordinated Omission:
 ------------------------------------------------------------------------------
 Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
 ------------------------------------------------------------------------------
 GET (Anon) front page    |     14 |     21 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Anon) node page     |     40 |     55 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Anon) user page     |     27 |     32 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Auth) comment form  |     37 |    400 |  2,951 |  2,951 |  2,951 |  2,951
 GET (Auth) front page    |     33 |    410 |  2,960 |  2,960 |  2,960 |  2,960
 GET (Auth) node page     |     37 |    410 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Auth) user page     |     27 |    420 |  2,959 |  2,959 |  2,959 |  2,959
 GET static asset         |      9 |     20 |  3,000 |  3,000 |  3,000 |  3,000
 POST (Auth) comment form |     55 |    390 |  2,932 |  2,932 |  2,932 |  2,932
 -------------------------+--------+--------+--------+--------+--------+-------
 Aggregated               |     14 |     42 |  3,000 |  3,000 |  3,000 |  3,000
 ```

The Coordinated Omission metrics will also show up in the HTML report generated when Goose is started with the `--report-file` run-time option. If Coordinated Omission mitigation kicked in, the HTML report will include both the "raw" metrics and the "adjusted" metrics.

## Enhanced CO Event Tracking

In addition to the raw and adjusted metrics, Goose now provides detailed Coordinated Omission event tracking that appears in all report formats (console, HTML, markdown, and JSON). This enhanced tracking provides comprehensive insights into when and how CO events affected your test.

### CO Event Metrics Display

When CO events occur during your test, you'll see a dedicated "COORDINATED OMISSION METRICS" section that appears before the overview:

```bash
 === COORDINATED OMISSION METRICS ===
 Duration: 45 seconds
 Total CO Events: 12
 Events per minute: 16.00

 Request Breakdown:
   Actual requests: 2,847
   Synthetic requests: 156 (5.2%)

 Severity Distribution:
   Minor: 8
   Moderate: 3
   Severe: 1
   Critical: 0
```

### Understanding CO Event Severity

Goose classifies CO events based on how much longer the actual response took compared to the expected cadence:

- **Minor (2-5x)**: Response took 2-5 times longer than expected
- **Moderate (5-10x)**: Response took 5-10 times longer than expected  
- **Severe (10-20x)**: Response took 10-20 times longer than expected
- **Critical (>20x)**: Response took more than 20 times longer than expected

### Interpreting Synthetic Request Percentage

The synthetic request percentage tells you how much of your data comes from CO mitigation:

- **<10%**: High confidence in results, minimal CO impact
- **10-30%**: Medium confidence, some CO events occurred
- **30-50%**: Lower confidence, significant CO impact
- **>50%**: Results heavily influenced by synthetic data

### Practical Example: Microservice Testing

Consider testing a microservice with strict 100ms SLA requirements:

```bash
# Test with minimum cadence for strict SLA validation
cargo run --example api_test -- \
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
#   Actual requests: 14,523
#   Synthetic requests: 892 (5.8%)
# 
# Severity Distribution:
#   Minor: 38    # Most events were 2-5x slower than expected
#   Moderate: 6  # Some 5-10x slower
#   Severe: 1    # One event 10-20x slower
#   Critical: 0  # No critical events
```

This tells you that while most requests met the SLA, there were 45 instances where performance degraded, affecting 5.8% of your measurements. The predominance of "Minor" events suggests occasional but not severe performance issues.

### Practical Example: Web Application Testing

For a public-facing web application with more tolerance for variance:

```bash
# Test with average cadence for realistic user simulation
cargo run --example webapp_test -- \
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
#   Actual requests: 28,945
#   Synthetic requests: 67 (0.2%)
# 
# Severity Distribution:
#   Minor: 5
#   Moderate: 2
#   Severe: 1
#   Critical: 0
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
2. **SLA Validation**: Use minimum cadence mode to catch any SLA violations
3. **Performance Regression**: Compare CO metrics across test runs to detect degradation
4. **Resource Scaling**: CO events often indicate when additional resources are needed

The enhanced CO metrics provide the detailed insights needed to understand not just that performance issues occurred, but their frequency, severity, and impact on your test results.
