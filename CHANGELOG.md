# Changelog

## 0.7.1 May 26, 2020
 - no longer compile Reqwest blocking client
 - remove need to declare `use std::boxed::Box` in load tests
 - remove unnecessary mutexes
 - introduce `use goose::prelude::*`

## 0.7.0 May 25, 2020
 - initial async support

## 0.6.3-dev
 - nng does not support udp as a transport protocol, and tcp overhead isn't
   problematic; remove to-do to add udp, hard-code tcp
 - add worker id for tracing gaggle worker threads
 - cleanup gaggle logic and comments

## 0.6.2 May 18, 2020
 - replace `unsafe` code blocks with `lazy_static` singleton
 - perform checksum to confirm workers are running same load test,
   `--no-hash-check` to ignore
 - code and documentation consistency

## 0.6.1 May 16, 2020
 - replace `--print-stats` with `--no-stats`, default to printing stats
 - make gaggle an optional compile-time feature
 - GooseState is now GooseAttack

## 0.6.0 May 14, 2020
 - Initial support for gaggles: distributed load testing
