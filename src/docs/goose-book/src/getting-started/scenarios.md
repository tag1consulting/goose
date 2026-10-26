# Limiting Which Scenarios Run

It can often be useful to run only a subset of the [Scenarios](../glossary.html#scenario) defined by a load test. Instead of commenting them out in the source code and recompiling, the `--scenarios` run-time option allows you to dynamically control which Scenarios are running.

## Listing Scenarios By Machine Name
To ensure that each scenario has a unique name, you must use the machine name of the scenario when filtering which are running. For example, using the [Umami example](../example/umami.html) enable the `--scenarios-list` flag:

```bash,ignore
% cargo run --release --example umami -- --scenarios-list
    Finished `release` profile [optimized] target(s) in 0.22s
     Running `target/release/examples/umami --scenarios-list`
21:02:23 [INFO] Output verbosity level: INFO
21:02:23 [INFO] Logfile verbosity level: WARN
21:02:23 [INFO] users defaulted to number of CPUs = 18
Scenarios:
 - adminuser: ("Admin user")
 - anonymousenglishuser: ("Anonymous English user")
 - anonymousspanishuser: ("Anonymous Spanish user")
 ```

> **What Is A Machine Name:** A Scenario can have any name, and several Scenarios can share one. A machine name ensures that you can still identify each Scenario uniquely, and without any special characters that can be difficult or insecure to pass through the command line. A machine name is made up of only the ASCII letters and digits found in your Scenario's full name, in lower case, and optionally with `_` and a number appended to differentiate between multiple Scenarios that would otherwise have the same name.
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

To run a subset of the above scenarios, pass a comma separated list of machine names with the `--scenarios` run time option. Goose uses **exact matching** by default: `admin` does not run `adminuser`. To match multiple scenarios with a single pattern, use the `*` wildcard character which matches zero or more characters. Matching ignores case and the white space around each name, so `AdminUser` runs `adminuser`. Set the list in code with `GooseDefault::Scenarios`.

For example, to run only the two anonymous Scenarios, you could add `--scenarios "anon*"`:

```bash,ignore
% cargo run --release --example umami -- --increase-rate 10 --scenarios "anon*"
    Finished `release` profile [optimized] target(s) in 0.11s
     Running `target/release/examples/umami --increase-rate 10 --scenarios 'anon*'`
21:02:35 [INFO] Output verbosity level: INFO
21:02:35 [INFO] Logfile verbosity level: WARN
21:02:35 [INFO] users defaulted to number of CPUs = 18
21:02:35 [INFO] increase_rate = 10.0
21:02:35 [INFO] scenarios = Scenarios { active: ["anon*"] }
```

Or, to run only the "Anonymous Spanish user" and "Admin user" Scenarios using their exact machine names, you could add `--scenarios "anonymousspanishuser,adminuser"`:

```bash,ignore
% cargo run --release --example umami -- --increase-rate 10 --scenarios "anonymousspanishuser,adminuser"
21:02:26 [INFO] scenarios = Scenarios { active: ["anonymousspanishuser", "adminuser"] }
```

You can also mix exact and wildcard matching: `--scenarios "adminuser,anon*"` would run the exact `adminuser` scenario plus all scenarios starting with `anon`.

A name may hold only letters, digits, `_` and `*`. Anything else is refused before the load test starts, with the reason:

```bash,ignore
% cargo run --release --example umami -- --scenarios "admin-user"
error: invalid value 'admin-user' for '--scenarios <LIST>': 'admin-user' is not a scenario name: use letters, digits and _, with * as a wildcard; --scenarios-list prints the names.

For more information, try '--help'.
```

A machine name with a `_` suffix, such as `loadtesttransactions_1` above, selects that one scenario. For example, a load test that registers two scenarios both named `Visitor` lists them as `visitor` and `visitor_1`, and `--scenarios visitor_1` runs only the second:

```bash,ignore
% cargo run --release -- --host http://127.0.0.1:8000 -u2 -r2 -t2s --scenarios visitor_1
...
 === PER SCENARIO METRICS ===
 ------------------------------------------------------------------------------
 Name                     |  # users |  # times run | scenarios/s | iterations
 ------------------------------------------------------------------------------
 1: Visitor               |        0 |            0 |        0.00 |       0.00
 2: Visitor               |        2 |       115156 |       57578 |      57578
 -------------------------+----------+--------------+-------------+------------
 Aggregated               |        2 |       115156 |       57578 |      57578
```

The final summary still lists the first `Visitor` scenario, with no users.

A name that matches no scenario logs a warning, as it is most likely mistyped, and is otherwise ignored as long as another name in the list matches one. When nothing in the list matches any scenario, Goose prints the scenarios and stops with an error:

```bash,ignore
% cargo run --release --example umami -- --scenarios "nosuch"
19:55:15 [INFO] scenarios = Scenarios { active: ["nosuch"] }
19:55:15 [WARN] --scenarios "nosuch" matches no scenario; --scenarios-list prints the names
Scenarios:
 - adminuser: ("Admin user")
 - anonymousenglishuser: ("Anonymous English user")
 - anonymousspanishuser: ("Anonymous Spanish user")
Error: NoScenarios { detail: "No scenarios are enabled." }
```

When the load test completes, you can refer to the [Scenario metrics](./metrics.html#scenarios) to confirm which Scenarios were enabled, and which were not.
