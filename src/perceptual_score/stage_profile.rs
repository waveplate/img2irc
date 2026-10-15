//! Opt-in timings for the expensive incremental full-image score stages.

use std::sync::OnceLock;
use std::time::Instant;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("IMG2IRC_REPAIR_PROFILE").is_some())
}

#[derive(Default)]
pub(super) struct Profile {
    pub(super) nanos: [u128; 8],
    pub(super) updates: usize,
}

pub(super) struct Span(Option<Instant>);

impl Span {
    pub(super) fn new() -> Self {
        Self(enabled().then(Instant::now))
    }

    pub(super) fn mark(&mut self, profile: &mut Profile, phase: usize) {
        if let Some(started) = &mut self.0 {
            profile.nanos[phase] += started.elapsed().as_nanos();
            *started = Instant::now();
        }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        if !enabled() || self.updates == 0 {
            return;
        }
        let ms = self.nanos.map(|n| n as f64 / 1_000_000.0);
        log::info!("Repair Edges score stages ({} cache updates): smooth={:.1}ms edges={:.1}ms curvature={:.1}ms matching={:.1}ms coarse={:.1}ms invalidation={:.1}ms geometry-sums={:.1}ms fine-sums={:.1}ms", self.updates, ms[0], ms[1], ms[2], ms[3], ms[4], ms[5], ms[6], ms[7]);
    }
}
