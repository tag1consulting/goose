# Limiting Which Scenarios Run

It can often be useful to run only a subset of the [Scenarios](../glossary.html#scenario) defined by a load test. Instead of commenting them out in the source code and recompiling, the `--scenarios` run-time option allows you to dynamically control which Scenarios are running.

## Listing Scenarios By Machine Name
To ensure that each scenario has a unique name, you must use the machine name of the scenario when filtering which are running. For example, using the [Umami example](../example/umami.html) enable the `--scenarios-list` flag:

```bash,ignore
% cargo run --release --example umami -- --scenarios-list
    Finished release [optimized] target(s) in 0.15s
     Running `target/release/examples/umami --scenarios-list`
05:24:03 [INFO] Output verbosity level: INFO
05:24:03 [INFO] Logfile verbosity level: WARN
05:24:03 [INFO] users defaulted to number of CPUs = 10
05:24:03 [INFO] iterations = 0
Scenarios:
 - adminuser: ("Admin user")
 - anonymousenglishuser: ("Anonymous English user")
 - anonymousspanishuser: ("Anonymous Spanish user")
 ```

> **What Is A Machine Name:** It is possible to name your Scenarios pretty much anything you want in your load test, including even using the same identical name for multiple Scenarios. A machine name ensures that you can still identify each Scenario uniquely, and without any special characters that can be difficult or insecure to pass through the command line. A machine name is made up of only the alphanumeric characters found in your Scenario's full name, and optionally with a number appended to differentiate between multiple Scenarios that would otherwise have the same name.
>
> In the following example, we have three very similarly named Scenarios. One simply has an extra white space between words. The second has an airplane emoticon in the name. Both the extra space and the airplane symbol are stripped away from the machine name as they are not alphanumerics, and instead `_1` and `_2` are appended to the end to differentiate:
>
> ```ignore
> Scenarios:
> - loadtesttransactions: ("LoadtestTransactions")
> - loadtesttransactions_1: ("Loadtest Transactions")
> - loadtesttransactions_2: ("LoadtestTransactions ✈️")
>
> ```

## Running Scenarios By Machine Name

It is now possible to run any subset of the above scenarios by passing a comma separated list of machine names with the `--scenarios` run time option. Goose uses **exact matching** by default. To match multiple scenarios with a single pattern, use the `*` wildcard character which matches zero or more characters.

For example, to run only the two anonymous Scenarios, you could add `--scenarios "anon*"`:

```bash,ignore
% cargo run --release --example umami -- --hatch-rate 10 --scenarios "anon*"
    Finished release [optimized] target(s) in 0.15s
     Running `target/release/examples/umami --hatch-rate 10 --scenarios 'anon*'`
05:50:17 [INFO] Output verbosity level: INFO
05:50:17 [INFO] Logfile verbosity level: WARN
05:50:17 [INFO] users defaulted to number of CPUs = 10
05:50:17 [INFO] hatch_rate = 10
05:50:17 [INFO] iterations = 0
05:50:17 [INFO] scenarios = Scenarios { active: ["anon*"] }
```

Or, to run only the "Anonymous Spanish user" and "Admin user" Scenarios using their exact machine names, you could add `--scenarios "anonymousspanishuser,adminuser"`:

```bash,ignore
% cargo run --release --example umami -- --hatch-rate 10 --scenarios "anonymousspanishuser,adminuser"
05:53:45 [INFO] scenarios = Scenarios { active: ["anonymousspanishuser", "adminuser"] }
```

You can also mix exact and wildcard matching: `--scenarios "adminuser,anon*"` would run the exact `adminuser` scenario plus all scenarios starting with `anon`.

When the load test completes, you can refer to the [Scenario metrics](./metrics.html#scenarios) to confirm which Scenarios were enabled, and which were not.
