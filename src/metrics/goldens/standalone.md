
# Goose Attack Report


## Plan Overview

| Action | Started | Stopped | Elapsed | Users |
| ------ | ------- | ------- | ------- | ----: |

## Request Metrics

| Method | Name | # Requests | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ------ | ---- | ---------: | ------: | -----------: | -------: | -------: | --: | ---------: |
| GET | / | 9 | 1 | 0.65 | 0.45 | 0.95 | 0.15 | 0.02 |
|  | Aggregated | 9 | 1 | 0.65 | 0.45 | 0.95 | 0.15 | 0.02 |

## Response Time Metrics

| Method | Name | 50%ile (ms) | 60%ile (ms) | 70%ile (ms) | 80%ile (ms) | 90%ile (ms) | 95%ile (ms) | 99%ile (ms) | 100%ile (ms) |
| ------ | ---- | ----------: | ----------: | ----------: | ----------: | ----------: | ----------: | ----------: | -----------: |
| GET | / | 0.59 | 0.59 | 0.64 | 0.7 | 0.82 | 0.95 | 0.95 | 0.95 |
|  | Aggregated | 0.59 | 0.59 | 0.64 | 0.7 | 0.82 | 0.95 | 0.95 | 0.95 |

## Status Code Metrics

| Method | Name | Status Codes |
| ------ | ---- | ------------ |
| GET | / |  |
|  | Aggregated |  |

## Transaction Metrics

| Transaction | # Times Run | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ----------- | ----------: | ------: | -----------: | -------: | -------: | --: | ---------: |
| **Scenario** |
| 0.0 index | 3 | 0 | 0.65 | 0.59 | 0.72 | 0.05 | 0.00 |
|  Aggregated | 3 | 0 | 0.65 | 0.59 | 0.72 | 0.05 | 0.00 |

## Scenario Metrics

| Scenario | # Users | # Times Run | Average (ms) | Min (ms) | Max (ms) | Scenarios/s | Iterations |
| ----------- | ------: | ----------: | -----------: | -------: | -------: | ----------: | ---------: |
| Scenario | 1 | 2 | 1.55 | 1.23  | 1.87 | 0.03 | 2.00 |
| Aggregated | 1 | 2 | 1.55 | 1.23  | 1.87 | 0.03 | 2.00 |
