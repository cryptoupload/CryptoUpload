// SPDX-License-Identifier: GPL-3.0-or-later

pub const MIN_WINDOW: u64 = 8;
pub const MAX_WINDOW: u64 = 128;
pub const MIN_BATCH: u64 = 16;

pub struct ReadaheadState {
    last_chunk_seen: Option<u64>,
    window: u64,
    prefetch_frontier: Option<u64>,
}

impl ReadaheadState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_chunk_seen: None,
            window: MIN_WINDOW,
            prefetch_frontier: None,
        }
    }

    pub fn advance(&mut self, start: u64, end: u64) -> (u64, u64) {
        if self.last_chunk_seen == Some(end) {
            return (end + 1, 0);
        }
        let sequential = self.last_chunk_seen.is_some_and(|last| start <= last + 1);
        self.window = if sequential {
            (self.window * 2).min(MAX_WINDOW)
        } else {
            MIN_WINDOW
        };
        self.last_chunk_seen = Some(end);

        if !sequential {
            self.prefetch_frontier = None;
        }

        let from = self.prefetch_frontier.map_or(end + 1, |f| f.max(end + 1));
        let target = end + 1 + self.window;
        let count = target.saturating_sub(from);
        if self.prefetch_frontier.is_some() && count < MIN_BATCH.min(self.window / 2) {
            return (from, 0);
        }
        self.prefetch_frontier = Some(target.max(from));
        (from, count)
    }
}

impl Default for ReadaheadState {
    fn default() -> Self {
        Self::new()
    }
}
