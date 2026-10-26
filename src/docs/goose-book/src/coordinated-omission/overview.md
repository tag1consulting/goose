# What is Coordinated Omission?

Coordinated Omission is a measurement problem that accidentally hides how many people are actually affected by server slowdowns during load testing.

## The Race Timer Problem

Imagine you're timing runners in a race:
- You're supposed to time a runner every 10 seconds
- But sometimes a runner gets stuck in mud for 30 seconds  
- Your timer waits for that stuck runner before timing the next one
- So instead of timing 6 runners per minute, you only time 2

**Result**: Your data says "average time: 15 seconds" when reality is most runners take 10 seconds, but some get stuck for 30+ seconds!

**The issue**: You're only measuring the runners who actually finish, missing all the runners who would have started during the delay.

## How This Affects Load Testing

### The Problem: Missing the Full Impact

```
Timeline of Goose User Thread:
Time 0s:  Send request ✅ (1 second response)
Time 1s:  Send request ✅ (1 second response)  
Time 2s:  Send request ✅ (1 second response)
Time 3s:  Send request... ⏳ (server freezes!)
Time 33s: Finally get response ❌ (30 seconds late!)
Time 34s: Send request ✅ (1 second response)
```

**What traditional load testing records:**
- 4 requests total
- Average response time: 8.25 seconds
- "Looks like mostly good performance"

**What really happened:**
During that 30-second freeze, **29 more requests should have been made** but couldn't because the thread was stuck waiting. So instead of 4 requests, there should have been 33 requests affected by the server problem.

## Why This Matters

When your server freezes for 30 seconds, **EVERY user trying to access it during those 30 seconds is affected**. Traditional load testing makes it look like only 1 user had problems, when really 30+ users experienced the issue.

This leads to dangerously optimistic reports:
- ❌ "99% of requests were fast" (hiding the freeze)
- ✅ "Server had a 30-second outage affecting 87% of traffic" (reality)

## How Goose Fixes This

Coordinated omission mitigation is off by default; `--co-mitigation average`, `minimum` or `maximum` turns it on. See [Mitigation Strategies](mitigation.md).

### 1. Detects Missing Requests
Goose tracks how long each user takes to run through all of its transactions. When a run takes more than twice the user's usual cadence, as one held up by a 30 second freeze does, Goose recognizes: "During that run, this user should have made more requests but couldn't."

### 2. Synthetic Request Injection
Goose adds "synthetic requests" to represent the requests that **would have been made** if the server hadn't frozen, giving you a complete picture of impact.

### 3. Clear Reporting
Goose shows the measured and the adjusted response times side by side, and sums up the events. Here is a real run like the freeze above: one user of a load test whose only transaction requests `/`, against a local server that answers in 1 second, with `--co-mitigation minimum` so the cadence is the user's fastest run. 3.5 seconds into the run the server froze for 30 seconds:

```bash
cargo run --release -- --host http://127.0.0.1:8080/ -u1 -r1 -t36s --co-mitigation minimum --no-telnet --no-websocket
```

```
 === COORDINATED OMISSION METRICS ===
 Duration: 37 seconds
 Total CO Events: 1
 Events per minute: 1.62

 Request Breakdown:
   Actual requests: 8
   Synthetic requests: 28 (77.8%)

 Severity Distribution:
   Critical: 1
```

The user measured 8 requests, one of them held for 30 seconds, and Goose added 28 synthetic requests for those it could not send during the freeze, so the `GET /` row counts 36 requests. This tells you: "77.8% of your expected traffic was affected by server problems!"

## Visual Comparison

**Before (Traditional Load Testing):**
```
Timeline: |--1s--|--1s--|--------30s--------|--1s--|
Requests:    ✅     ✅          ❌           ✅
Result: "4 requests, looks mostly fine" ❌ MISLEADING
```

**After (Goose with CO Mitigation):**
```
Timeline: |--1s--|--1s--|--------30s--------|--1s--|  
Real:        ✅     ✅          ❌           ✅
Synthetic:              ❌❌❌❌❌...❌❌❌    (about one for each second of the freeze)
Result: "30-second freeze affected most of the traffic" ✅ ACCURATE
```

## The Key Insight

A server problem affects every request that should have been sent while it lasted, not only the one that was slow. With mitigation enabled, Goose counts those requests too.
