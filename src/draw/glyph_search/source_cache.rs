//! A source window never changes during a search. Share its prepared filters
//! across pair jobs and passes; only output features need refreshing on edits.
use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Cache {
    entries: HashMap<[usize; 4], Arc<PreparedReference>>,
    content: HashMap<u64, Vec<[usize; 4]>>,
    bytes: usize,
    hits: usize,
    misses: usize,
    content_hits: usize,
}

pub(super) struct SourceReferences<'a> {
    source: &'a [[u8; 3]],
    width: usize,
    budget: usize,
    cache: Mutex<Cache>,
}

impl<'a> SourceReferences<'a> {
    pub(super) fn source(&self) -> &[[u8; 3]] {
        self.source
    }

    fn same_pixels(
        &self,
        bounds: [usize; 4],
        pixels: &[[u8; 3]],
        width: usize,
        height: usize,
    ) -> bool {
        let [x0, y0, x1, y1] = bounds;
        (x1 - x0) * CW == width
            && (y1 - y0) * CH == height
            && pixels.chunks_exact(width).enumerate().all(|(y, row)| {
                let begin = (y0 * CH + y) * self.width + x0 * CW;
                row == &self.source[begin..begin + width]
            })
    }

    fn find_content(
        &self,
        cache: &mut Cache,
        hash: u64,
        bounds: [usize; 4],
        pixels: &[[u8; 3]],
        width: usize,
        height: usize,
    ) -> Option<Arc<PreparedReference>> {
        let found = cache
            .content
            .get(&hash)?
            .iter()
            .find(|&&key| self.same_pixels(key, pixels, width, height))?;
        let reference = cache.entries[found].clone();
        cache.content_hits += 1;
        let bytes = std::mem::size_of::<([usize; 4], Arc<PreparedReference>)>();
        if bytes <= self.budget.saturating_sub(cache.bytes) && !cache.entries.contains_key(&bounds)
        {
            cache.bytes += bytes;
            cache.entries.insert(bounds, reference.clone());
        }
        Some(reference)
    }

    pub(super) fn new(source: &'a [[u8; 3]], width: usize) -> Self {
        Self {
            source,
            width,
            budget: if cfg!(target_arch = "wasm32") { 8 } else { 32 } * 1024 * 1024,
            cache: Mutex::new(Cache::default()),
        }
    }

    pub(super) fn get(&self, bounds: [usize; 4]) -> Arc<PreparedReference> {
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(reference) = cache.entries.get(&bounds).cloned() {
                cache.hits += 1;
                return reference;
            }
            cache.misses += 1;
        }
        let [x0, y0, x1, y1] = bounds;
        let (width, height) = ((x1 - x0) * CW, (y1 - y0) * CH);
        let mut pixels = Vec::with_capacity(width * height);
        for y in y0 * CH..y1 * CH {
            let begin = y * self.width + x0 * CW;
            pixels.extend_from_slice(&self.source[begin..begin + width]);
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (width, height).hash(&mut hasher);
        pixels.as_flattened().hash(&mut hasher);
        let hash = hasher.finish();
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(reference) =
                self.find_content(&mut cache, hash, bounds, &pixels, width, height)
            {
                return reference;
            }
        }
        // Prepare outside the lock so unrelated windows can run concurrently.
        let reference = Arc::new(
            PreparedReference::new(&pixels, width, height, CW)
                .expect("valid glyph search source window"),
        );
        let mut cache = self.cache.lock().unwrap();
        if let Some(existing) = cache.entries.get(&bounds) {
            return existing.clone();
        }
        if let Some(existing) = self.find_content(&mut cache, hash, bounds, &pixels, width, height)
        {
            return existing;
        }
        let bytes = reference.allocated_bytes()
            + std::mem::size_of::<([usize; 4], Arc<PreparedReference>)>()
            + 128;
        if bytes <= self.budget.saturating_sub(cache.bytes) {
            cache.bytes += bytes;
            cache.entries.insert(bounds, reference.clone());
            cache.content.entry(hash).or_default().push(bounds);
        }
        reference
    }

    pub(super) fn report(&self) {
        let cache = self.cache.lock().unwrap();
        log::info!("Source reference cache: {} hits, {} misses, {} windows retained ({:.1} MiB / {} MiB budget)",
            cache.hits, cache.misses, cache.entries.len(), cache.bytes as f64 / 1048576.0,
            self.budget / 1048576);
        log::info!(
            "Source reference content reuse: {} hits across {} unique retained windows",
            cache.content_hits,
            cache.content.values().map(Vec::len).sum::<usize>()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_source_windows_at_different_locations_share_filters() {
        let source = vec![[20, 30, 40]; 4 * CW * 4 * CH];
        let cache = SourceReferences::new(&source, 4 * CW);
        let a = cache.get([0, 0, 2, 2]);
        let b = cache.get([1, 1, 3, 3]);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.cache.lock().unwrap().content_hits, 1);
        assert!(!Arc::ptr_eq(&a, &cache.get([0, 0, 1, 4])));
    }

    #[test]
    fn content_hash_collision_cannot_reuse_different_source_pixels() {
        let width = 4 * CW;
        let source: Vec<_> = (0..width * 4 * CH)
            .map(|p| [(p % 251) as u8, 30, 40])
            .collect();
        let references = SourceReferences::new(&source, width);
        references.get([0, 0, 2, 2]);
        let mut cache = references.cache.lock().unwrap();
        let hash = *cache.content.keys().next().unwrap();
        let different = vec![[255; 3]; 2 * CW * 2 * CH];
        assert!(references
            .find_content(&mut cache, hash, [1, 1, 3, 3], &different, 2 * CW, 2 * CH)
            .is_none());
        assert_eq!(cache.content_hits, 0);
    }

    #[test]
    fn concurrent_requests_publish_one_shared_reference() {
        let source = vec![[20, 30, 40]; 3 * CW * 3 * CH];
        let cache = SourceReferences::new(&source, 3 * CW);
        let barrier = std::sync::Barrier::new(4);
        let references = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        cache.get([0, 0, 2, 2])
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(references.iter().all(|r| Arc::ptr_eq(r, &references[0])));
        assert_eq!(cache.cache.lock().unwrap().entries.len(), 1);
    }

    #[test]
    fn source_windows_reuse_exact_references_and_respect_the_budget() {
        let width = 4 * CW;
        let source: Vec<_> = (0..width * 4 * CH)
            .map(|p| [(p * 17 % 256) as u8, (p * 31 % 256) as u8, 90])
            .collect();
        let cache = SourceReferences::new(&source, width);
        let first = cache.get([0, 0, 2, 2]);
        assert!(Arc::ptr_eq(&first, &cache.get([0, 0, 2, 2])));
        assert!(!Arc::ptr_eq(&first, &cache.get([1, 1, 3, 3])));
        assert!(cache.cache.lock().unwrap().bytes <= cache.budget);
        let mut uncached = SourceReferences::new(&source, width);
        uncached.budget = 0;
        let second = uncached.get([0, 0, 2, 2]);
        assert!(uncached.cache.lock().unwrap().entries.is_empty());
        let pixels = vec![[40, 50, 60]; 2 * CW * 2 * CH];
        assert_eq!(
            first.score(&pixels).total.to_bits(),
            second.score(&pixels).total.to_bits()
        );
    }
}
