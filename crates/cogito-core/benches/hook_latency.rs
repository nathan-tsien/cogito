//! Hook latency benchmark — verifies the H09 §"Critical invariants"
//! P99 < 5ms budget for a typical 5-handler pipeline. Smoke
//! threshold; baseline numbers recorded in
//! `docs/quality/v0.1-hook-latency.md`.

// Criterion benches expand macros that produce undocumented harness items
// and we intentionally use patterns that would trip workspace-level lints.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use cogito_core::harness::hooks::CompositeHookPipeline;
use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler};
use criterion::{Criterion, criterion_group, criterion_main};

struct LightWork;

impl HookHandler for LightWork {
    fn name(&self) -> &'static str {
        "light"
    }

    fn pre_prompt(&self, _input: &ModelInput) -> HookDecision {
        // Simulate ~100us of synthetic work without any I/O.
        let mut acc = 0u64;
        for i in 0..1000 {
            acc = acc.wrapping_add(i);
        }
        std::hint::black_box(acc);
        HookDecision::Allow
    }
}

fn bench_pre_prompt_5_handlers(c: &mut Criterion) {
    let pipeline = CompositeHookPipeline::with_handlers(
        (0..5)
            .map(|_| Arc::new(LightWork) as Arc<dyn HookHandler>)
            .collect(),
    );
    c.bench_function("hook.pre_prompt.5_handlers", |b| {
        b.iter(|| {
            let _ = pipeline.pre_prompt(&ModelInput::default());
        });
    });
}

criterion_group!(benches, bench_pre_prompt_5_handlers);
criterion_main!(benches);
