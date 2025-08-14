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
During that 30-second freeze, **30 more requests should have been made** but couldn't because the thread was stuck waiting. So instead of 4 requests, there should have been 34 requests affected by the server problem.

## Why This Matters

When your server freezes for 30 seconds, **EVERY user trying to access it during those 30 seconds is affected**. Traditional load testing makes it look like only 1 user had problems, when really 30+ users experienced the issue.

This leads to dangerously optimistic reports:
- ❌ "99% of requests were fast" (hiding the freeze)
- ✅ "Server had a 30-second outage affecting 87% of traffic" (reality)

## How Goose Fixes This

### 1. Detects Missing Requests
When Goose sees an abnormally long 30-second response, it recognizes: "During those 30 seconds, I should have made 30 requests but couldn't."

### 2. Synthetic Request Injection
Goose adds "synthetic requests" to represent the requests that **would have been made** if the server hadn't frozen, giving you a complete picture of impact.

### 3. Clear Reporting
```
=== COORDINATED OMISSION METRICS ===
Total CO Events: 1
Actual requests: 4  
Synthetic requests: 29 (87.9%)
Severity: 1 Critical event detected
```

This tells you: "87.9% of your expected traffic was affected by server problems!"

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
Synthetic:              ❌❌❌❌❌...❌❌❌    (29 more)
Result: "34 total requests, 30-second freeze affected 87.9% of traffic" ✅ ACCURATE
```

## The Key Insight

Server problems don't just affect one request, they affect ALL the requests that should have happened during the problem period. Goose now captures this reality, giving you honest data about your system's behavior under load.
