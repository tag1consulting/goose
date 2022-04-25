# Simple Example 

The [`examples/simple.rs`](https://github.com/tag1consulting/goose/blob/main/examples/simple.rs) example copies the simple load test documented on the [locust.io web page](https://locust.io/), rewritten in Rust for Goose. It uses minimal advanced functionality, but demonstrates how to GET and POST pages. It defines a single Scenario which has the user log in and then loads a couple of pages.

Goose can make use of all available CPU cores. By default, it will launch 1 user per core, and it can be configured to launch many more. The following was configured instead to launch 1,024 users. Each user randomly pauses 5 to 15 seconds after each transaction is loaded, so it's possible to spin up a large number of users. Here is a snapshot of `top` when running this example on a 1-core VM with 10G of available RAM -- there were ample resources to launch considerably more "users", though `ulimit` had to be resized:

```bash
top - 06:56:06 up 15 days,  3:13,  2 users,  load average: 0.22, 0.10, 0.04
Tasks: 116 total,   3 running, 113 sleeping,   0 stopped,   0 zombie
%Cpu(s):  1.7 us,  0.7 sy,  0.0 ni, 96.7 id,  0.0 wa,  0.0 hi,  1.0 si,  0.0 st
MiB Mem :   9994.9 total,   7836.8 free,   1101.2 used,   1056.9 buff/cache
MiB Swap:  10237.0 total,  10237.0 free,      0.0 used.   8606.9 avail Mem

  PID USER      PR  NI    VIRT    RES    SHR S  %CPU  %MEM     TIME+ COMMAND
 1339 goose     20   0 1235480 758292   8984 R   3.0   7.4   0:06.56 simple
```

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/simple.rs}}
```
