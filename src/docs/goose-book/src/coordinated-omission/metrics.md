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

From these two tables, it is clear that there was a statistically significant event affecting the load testing metrics. In particular, note that the standard deviation between the "raw" average and the "adjusted" average is considerably larger than the "raw" average, calling into questing whether or not your load test was "valid". (The answer to that question depends very much on your specific goals and load test.)

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
