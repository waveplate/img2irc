use super::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(PartialEq, Eq, Hash)]
pub(super) struct RenderCacheKey {
    options: Vec<u8>,
    image: u64,
    glyphs: u64,
}

#[derive(Default)]
pub(super) struct CompletedRenderCache {
    entries: HashMap<RenderCacheKey, draw::RenderResult>,
}

impl CompletedRenderCache {
    pub(super) fn key(
        args: &RenderArgs,
        source: &PhotonImage,
        glyphs: u64,
    ) -> Option<RenderCacheKey> {
        let mut options = args.clone();
        options.overlays.clear();
        options.save = None;
        options.show_glyph_defects = false;
        options.pipeline.retain(|effect| !effect.is_output_colour());
        let options = serde_json::to_vec(&options).ok()?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        source.get_width().hash(&mut hash);
        source.get_height().hash(&mut hash);
        source.get_raw_pixels().hash(&mut hash);
        Some(RenderCacheKey {
            options,
            image: hash.finish(),
            glyphs,
        })
    }

    pub(super) fn get(&self, key: &RenderCacheKey, need_map: bool) -> Option<draw::RenderResult> {
        let cached = self.entries.get(key)?;
        if need_map && cached.glyph_defects.is_none() {
            return None;
        }
        let mut result = cached.clone();
        result.profile = draw::RenderProfile::default();
        result.initial_time = Duration::ZERO;
        Some(result)
    }

    pub(super) fn insert(&mut self, key: RenderCacheKey, result: &draw::RenderResult) {
        if result.grid.is_empty() || result.content == "Cancelled" {
            return;
        }
        self.entries.insert(key, result.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(image: u64) -> RenderCacheKey {
        RenderCacheKey {
            options: vec![1, 2, 3],
            image,
            glyphs: 42,
        }
    }

    fn result(score: f64) -> draw::RenderResult {
        let mut result = draw::RenderResult::from("cached output");
        result.score = score;
        result.grid = vec![vec![draw::RenderedCell {
            char: 'X',
            fg: [255; 3],
            bg: Some([0; 3]),
            inverted: false,
            bold: false,
            italic: false,
            underline: false,
        }]];
        result.initial_time = Duration::from_secs(1);
        result.profile.glyph_search = Duration::from_secs(1);
        result
    }

    #[test]
    fn render_history_retains_old_entries_and_resets_only_returned_timings() {
        let mut cache = CompletedRenderCache::default();
        for i in 0..128 {
            cache.insert(key(i), &result(i as f64));
        }
        for i in 0..128 {
            let hit = cache.get(&key(i), false).unwrap();
            assert_eq!(hit.score, i as f64);
            assert_eq!(hit.initial_time, Duration::ZERO);
            assert_eq!(hit.profile.glyph_search, Duration::ZERO);
        }
        assert_eq!(cache.entries.len(), 128);
        assert_eq!(
            cache.entries[&key(0)].profile.glyph_search,
            Duration::from_secs(1)
        );
    }

    #[test]
    fn cache_preserves_diagnostic_requirements_and_ignores_cancelled_results() {
        let mut cache = CompletedRenderCache::default();
        let mut completed = result(12.0);
        cache.insert(key(0), &completed);
        assert!(cache.get(&key(0), true).is_none());
        completed.glyph_defects = Some(Arc::new(draw::search_status::GlyphDefectMap {
            columns: 1,
            rows: 1,
            threshold: 0.5,
            priorities: vec![1.0],
            targets: vec![true],
            active: vec![true],
        }));
        cache.insert(key(0), &completed);
        assert!(cache.get(&key(0), true).unwrap().glyph_defects.is_some());
        let mut cancelled = result(99.0);
        cancelled.content = "Cancelled".into();
        cache.insert(key(0), &cancelled);
        cache.insert(key(1), &draw::RenderResult::from(""));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.get(&key(0), false).unwrap().score, 12.0);
    }
}
