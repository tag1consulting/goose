
# Goose Attack Report


## Plan Overview

| Action | Started | Stopped | Elapsed | Users |
| ------ | ------- | ------- | ------- | ----: |
| Increasing | 25-08-07 08:34:32 | 25-08-07 08:34:33 | 00:00:01 | 0 &rarr; 16 |
| Maintaining | 25-08-07 08:34:33 | 25-08-07 08:34:43 | 00:00:10 | 16 |
| Decreasing | 25-08-07 08:34:43 | 25-08-07 08:34:44 | 00:00:01 | 0 &larr; 16 |

## Request Metrics

| Method | Name | # Requests | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ------ | ---- | ---------: | ------: | -----------: | -------: | -------: | --: | ---------: |
| GET | / | 7 | 0 | 283.43 | 217 | 368 | 0.70 | 0.00 |
| GET | /about/ | 9 | 9 | 196.33 | 106 | 539 | 0.90 | 0.90 |
| POST | /login | 3 | 3 | 225.67 | 222 | 229 | 0.30 | 0.30 |
|  | Aggregated | 19 | 12 | 233.05 | 106 | 539 | 1.90 | 1.20 |

## Response Time Metrics

| Method | Name | 50%ile (ms) | 60%ile (ms) | 70%ile (ms) | 80%ile (ms) | 90%ile (ms) | 95%ile (ms) | 99%ile (ms) | 100%ile (ms) |
| ------ | ---- | ----------: | ----------: | ----------: | ----------: | ----------: | ----------: | ----------: | -----------: |
| GET | / | 230 | 230 | 360 | 368 | 368 | 368 | 368 | 368 |
| GET | /about/ | 120 | 120 | 120 | 140 | 410 | 500 | 500 | 500 |
| POST | /login | 229 | 229 | 229 | 229 | 229 | 229 | 229 | 229 |
|  | Aggregated | 220 | 220 | 230 | 360 | 370 | 410 | 500 | 500 |

## Status Code Metrics

| Method | Name | Status Codes |
| ------ | ---- | ------------ |
| GET | / | 7 [200] |
| GET | /about/ | 9 [404] |
| POST | /login | 3 [403] |
|  | Aggregated | 7 [200], 3 [403], 9 [404] |

## Transaction Metrics

| Transaction | # Times Run | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ----------- | ----------: | ------: | -----------: | -------: | -------: | --: | ---------: |
| **WebsiteUser** |
| 0.0  | 3 | 0 | 225.67 | 222 | 229 | 0.30 | 0.00 |
| 0.1  | 7 | 0 | 283.43 | 217 | 368 | 0.70 | 0.00 |
| 0.2  | 9 | 0 | 196.33 | 106 | 539 | 0.90 | 0.00 |
|  Aggregated | 19 | 0 | 233.05 | 106 | 539 | 1.90 | 0.00 |

## Scenario Metrics

| Transaction | # Users | # Times Run | Average (ms) | Min (ms) | Max (ms) | Scenarios/s | Iterations |
| ----------- | ------: | ----------: | -----------: | -------: | -------: | ----------: | ---------: |
| WebsiteUser | 0 | 0 | 0.00 | 0  | 0 | 0.00 | NaN |
| Aggregated | 0 | 0 | NaN | 0  | 0 | 0.00 | NaN |

## Error Metrics

| Method | Name |  #  | Error |
| ------ | ---- | --: | ----- |
| POST | /login | 16 | 403 Forbidden: /login |
| GET | /about/ | 9 | 404 Not Found: /about/ |
