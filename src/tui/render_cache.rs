use super::*;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

const MAX_ENTRIES: usize = 4;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(PartialEq, Eq)]
pub(super) struct RenderCacheKey {
    options: Vec<u8>,
    image: u64,
    glyphs: u64,
}

struct Entry {
    key: RenderCacheKey,
    result: draw::RenderResult,
    bytes: usize,
}

#[derive(Default)]
pub(super) struct CompletedRenderCache {
    entries: VecDeque<Entry>,
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

    pub(super) fn get(
        &mut self,
        key: &RenderCacheKey,
        need_map: bool,
    ) -> Option<draw::RenderResult> {
        let i = self.entries.iter().position(|entry| {
            &entry.key == key && (!need_map || entry.result.glyph_defects.is_some())
        })?;
        let entry = self.entries.remove(i)?;
        let mut result = entry.result.clone();
        self.entries.push_back(entry);
        result.profile = draw::RenderProfile::default();
        result.initial_time = Duration::ZERO;
        Some(result)
    }

    pub(super) fn insert(&mut self, key: RenderCacheKey, result: &draw::RenderResult) {
        if result.grid.is_empty() || result.content == "Cancelled" {
            return;
        }
        let cells = result.grid.iter().map(Vec::len).sum::<usize>();
        let bytes = result.content.len()
            + result.save_content.as_ref().map_or(0, String::len)
            + cells * std::mem::size_of::<draw::RenderedCell>()
            + result
                .glyph_defects
                .as_ref()
                .map_or(0, |map| map.priorities.len() * 6);
        if bytes > MAX_BYTES {
            return;
        }
        self.entries.retain(|entry| entry.key != key);
        while self.entries.len() >= MAX_ENTRIES
            || self.entries.iter().map(|entry| entry.bytes).sum::<usize>() + bytes > MAX_BYTES
        {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            key,
            result: result.clone(),
            bytes,
        });
    }
}
