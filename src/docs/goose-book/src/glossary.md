# Glossary

## GooseUser
A thread that repeatedly runs a single [**scenario**](./getting-started/metrics.html#scenarios) for the duration of the load test. For example, when Goose starts, you may use the [`--users`](./getting-started/common.html#how-many-users-to-simulate) command line option to configure how many GooseUser threads are started. This is not intended to be a 1:1 correlation between GooseUsers and real website users.

## Request
A single [**request**](./getting-started/metrics.html#requests) based around HTTP verbs.

## Scenario
A [**scenario**](./getting-started/metrics.html#scenarios) is a collection of [**transactions**](./getting-started/metrics.html#transactions) (aka steps) a user would undertake to achieve a specific user journey.

## Transaction
A [**transaction**](./getting-started/metrics.html#transactions) is a collection of one or more [**requests**](./getting-started/metrics.html#request) and any desired validation. For example, this may include loading the front page and all contained static assets, logging into the website, or adding one or more items to a shopping chart.  Transactions typically include assertions or expectation validation.
