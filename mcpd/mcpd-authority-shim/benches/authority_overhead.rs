//! WS1.M6 authority+audit overhead criterion.
//!
//! This benchmark is intentionally dependency-free so it can run inside the
//! constrained ACOS workspace without pulling a benchmarking framework. It
//! measures the hot-path combination expected for a no-op routed MCP call:
//!
//! 1. capability check (`CapabilityPolicy::can_invoke`),
//! 2. append a result event to the authority audit ring.
//!
//! The process exits non-zero if p99 exceeds the WS1.M6 gate (<100µs).

use std::process;
use std::time::Instant;

use acos_authority_types::{AuditEvent, CallerContext};
use mcpd_authority_shim::{AuthorityShim, CapabilityPolicy};

const ITERATIONS: usize = 20_000;
const WARMUP: usize = 1_000;
const P99_GATE_US: u128 = 100;
const AUDIT_CAPACITY: usize = ITERATIONS + WARMUP + 16;

fn run_once(
    policy: &CapabilityPolicy,
    shim: &AuthorityShim,
    caller: &CallerContext,
    trace_id: u128,
) {
    let verdict = policy.can_invoke(caller, "noop", "ping");
    let latency_us = 0;
    let event = AuditEvent::new(
        trace_id,
        caller.label(),
        "noop.ping".to_string(),
        verdict,
        None,
        latency_us,
    );
    shim.record(event)
        .expect("audit ring append must succeed during overhead bench");
}

fn percentile(sorted: &[u128], percent: usize) -> u128 {
    debug_assert!(!sorted.is_empty());
    let index = ((sorted.len() * percent).saturating_add(99) / 100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

fn main() {
    let policy = CapabilityPolicy::new();
    policy.allow_uid(1_000);
    let shim = AuthorityShim::with_capacity(AUDIT_CAPACITY);
    let caller = CallerContext::from_parts(1_000, 1_000, 42);

    for i in 0..WARMUP {
        run_once(&policy, &shim, &caller, i as u128);
    }

    let mut samples_ns = Vec::with_capacity(ITERATIONS);
    for i in 0..ITERATIONS {
        let started = Instant::now();
        run_once(&policy, &shim, &caller, (WARMUP + i) as u128);
        samples_ns.push(started.elapsed().as_nanos());
    }
    samples_ns.sort_unstable();

    let p50_ns = percentile(&samples_ns, 50);
    let p95_ns = percentile(&samples_ns, 95);
    let p99_ns = percentile(&samples_ns, 99);
    let p99_us = p99_ns.div_ceil(1_000);

    println!(
        "authority_overhead iterations={} p50_ns={} p95_ns={} p99_ns={} p99_us={} gate_us={}",
        ITERATIONS, p50_ns, p95_ns, p99_ns, p99_us, P99_GATE_US
    );

    if p99_us >= P99_GATE_US {
        eprintln!(
            "authority_overhead gate failed: p99={}µs, expected <{}µs",
            p99_us, P99_GATE_US
        );
        process::exit(1);
    }
}
