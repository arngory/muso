//! The play queue: which track plays now and which comes next, with shuffle
//! and repeat.
//!
//! Items keep the order the user sees (a playlist, an album). Playback walks a
//! separate play order, which is the same as the item order unless shuffle is
//! on. Turning shuffle on keeps the current track playing and shuffles the
//! rest around it.

/// What happens at the end of a track or of the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    /// Stop after the last track.
    #[default]
    Off,
    /// Start over after the last track.
    All,
    /// Play the current track again when it ends.
    One,
}

impl Repeat {
    /// The next mode in the order the repeat button cycles through.
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Queue<T> {
    items: Vec<T>,
    /// Play order: indices into `items`.
    order: Vec<usize>,
    /// Position of the current track in `order`; `None` when stopped.
    pos: Option<usize>,
    shuffle: bool,
    repeat: Repeat,
    rng: Rng,
}

impl<T> Queue<T> {
    /// An empty queue. `seed` drives shuffling, so tests can pin it.
    pub fn new(seed: u64) -> Self {
        Queue {
            items: Vec::new(),
            order: Vec::new(),
            pos: None,
            shuffle: false,
            repeat: Repeat::Off,
            rng: Rng::new(seed),
        }
    }

    /// Replaces everything in the queue and stops.
    pub fn replace(&mut self, items: Vec<T>) {
        self.items = items;
        self.pos = None;
        self.order = (0..self.items.len()).collect();
        if self.shuffle {
            self.rng.shuffle(&mut self.order);
        }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// Index into [`items`](Self::items) of the track playing now.
    pub fn current_index(&self) -> Option<usize> {
        self.pos.map(|p| self.order[p])
    }

    pub fn current(&self) -> Option<&T> {
        self.current_index().map(|i| &self.items[i])
    }

    /// The track that plays after the current one ends on its own, for
    /// preloading (gapless playback). `None` when playback would stop.
    pub fn upcoming(&self) -> Option<&T> {
        let pos = self.pos?;
        if self.repeat == Repeat::One {
            return self.current();
        }
        let next = if pos + 1 < self.order.len() {
            pos + 1
        } else if self.repeat == Repeat::All && !self.shuffle {
            0
        } else {
            // With shuffle the next cycle's order is not drawn yet.
            return None;
        };
        Some(&self.items[self.order[next]])
    }

    /// Starts playing the item at `index` (an index into `items`).
    pub fn play(&mut self, index: usize) -> Option<&T> {
        if index >= self.items.len() {
            return None;
        }
        if self.shuffle {
            // A track picked by hand starts a fresh shuffle with it first.
            self.reshuffle(Some(index));
            self.pos = Some(0);
        } else {
            self.pos = Some(index);
        }
        self.current()
    }

    /// Stops playback; the queue keeps its items.
    pub fn stop(&mut self) {
        self.pos = None;
    }

    /// The current track ended by itself. Returns the track to play next, or
    /// `None` if playback stops.
    pub fn track_ended(&mut self) -> Option<&T> {
        if self.repeat == Repeat::One {
            return self.current();
        }
        self.advance(self.repeat == Repeat::All)
    }

    /// The user asked for the next track. Unlike [`track_ended`](Self::track_ended),
    /// this leaves the track even with [`Repeat::One`], and wraps around with
    /// any repeat mode.
    pub fn next_track(&mut self) -> Option<&T> {
        self.advance(self.repeat != Repeat::Off)
    }

    /// The user asked for the previous track. At the start of the queue it
    /// wraps around when repeat is on and stays put otherwise.
    pub fn prev_track(&mut self) -> Option<&T> {
        let pos = self.pos?;
        self.pos = Some(if pos > 0 {
            pos - 1
        } else if self.repeat != Repeat::Off {
            self.order.len() - 1
        } else {
            0
        });
        self.current()
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        let current = self.current_index();
        if on {
            self.reshuffle(current);
            self.pos = current.map(|_| 0);
        } else {
            self.order = (0..self.items.len()).collect();
            self.pos = current;
        }
    }

    /// Adds an item at the end of the queue.
    pub fn push(&mut self, item: T) {
        self.items.push(item);
        self.order.push(self.items.len() - 1);
    }

    /// Adds an item to play right after the current one ("play next").
    pub fn push_next(&mut self, item: T) {
        let at = self.pos.map_or(0, |p| p + 1);
        let index = self.order_to_item_slot(at);
        self.items.insert(index, item);
        // Everything at or after the new item's slot moved one to the right.
        for i in &mut self.order {
            if *i >= index {
                *i += 1;
            }
        }
        self.order.insert(at, index);
    }

    /// Removes the item at `index` (an index into `items`). If it was playing,
    /// the next track in play order becomes current, or playback stops at the
    /// end of the queue.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index >= self.items.len() {
            return None;
        }
        let at = self.order.iter().position(|&i| i == index)?;
        self.order.remove(at);
        for i in &mut self.order {
            if *i > index {
                *i -= 1;
            }
        }
        if let Some(pos) = self.pos {
            if at < pos {
                self.pos = Some(pos - 1);
            } else if at == pos && pos >= self.order.len() {
                self.pos = None;
            }
        }
        Some(self.items.remove(index))
    }

    /// Moves an item in the order the user sees, as when dragging a row.
    /// Without shuffle the play order follows; with shuffle it is unchanged.
    pub fn move_item(&mut self, from: usize, to: usize) {
        let len = self.items.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        let moved = |i: usize| {
            if i == from {
                to
            } else if from < to && i > from && i <= to {
                i - 1
            } else if to < from && i >= to && i < from {
                i + 1
            } else {
                i
            }
        };
        if self.shuffle {
            for i in &mut self.order {
                *i = moved(*i);
            }
        } else {
            self.pos = self.pos.map(moved);
        }
    }

    fn advance(&mut self, wrap: bool) -> Option<&T> {
        let pos = self.pos?;
        if pos + 1 < self.order.len() {
            self.pos = Some(pos + 1);
        } else if wrap && !self.order.is_empty() {
            if self.shuffle {
                let last = self.order[pos];
                self.reshuffle(None);
                // Don't play the same track twice in a row across cycles.
                if self.order.len() > 1 && self.order[0] == last {
                    self.order.swap(0, 1);
                }
            }
            self.pos = Some(0);
        } else {
            self.pos = None;
        }
        self.current()
    }

    /// Draws a new play order; `first`, if given, goes to the front.
    fn reshuffle(&mut self, first: Option<usize>) {
        self.order = (0..self.items.len()).collect();
        self.rng.shuffle(&mut self.order);
        if let Some(first) = first {
            let at = self.order.iter().position(|&i| i == first).unwrap();
            self.order.swap(0, at);
        }
    }

    /// The `items` index a new item takes when it goes to play-order slot
    /// `at`. Without shuffle the two orders match; with shuffle the item goes
    /// right after the current one in the visible order too.
    fn order_to_item_slot(&self, at: usize) -> usize {
        if self.shuffle {
            self.current_index().map_or(self.items.len(), |i| i + 1)
        } else {
            at
        }
    }
}

/// xorshift64*: small, fast and good enough to shuffle a playlist.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // xorshift needs a non-zero state.
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Fisher–Yates.
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(n: usize) -> Queue<usize> {
        let mut q = Queue::new(7);
        q.replace((0..n).collect());
        q
    }

    #[test]
    fn plays_in_order_and_stops_at_end() {
        let mut q = queue(3);
        assert_eq!(q.play(0), Some(&0));
        assert_eq!(q.track_ended(), Some(&1));
        assert_eq!(q.track_ended(), Some(&2));
        assert_eq!(q.track_ended(), None);
        assert_eq!(q.current(), None);
    }

    #[test]
    fn repeat_all_wraps() {
        let mut q = queue(2);
        q.set_repeat(Repeat::All);
        q.play(1);
        assert_eq!(q.upcoming(), Some(&0));
        assert_eq!(q.track_ended(), Some(&0));
    }

    #[test]
    fn repeat_one_replays_but_next_moves_on() {
        let mut q = queue(3);
        q.set_repeat(Repeat::One);
        q.play(2);
        assert_eq!(q.track_ended(), Some(&2));
        assert_eq!(q.next_track(), Some(&0));
    }

    #[test]
    fn prev_stays_at_start_without_repeat() {
        let mut q = queue(3);
        q.play(0);
        assert_eq!(q.prev_track(), Some(&0));
        q.set_repeat(Repeat::All);
        assert_eq!(q.prev_track(), Some(&2));
    }

    #[test]
    fn repeat_cycles_like_the_button() {
        assert_eq!(Repeat::Off.cycle(), Repeat::All);
        assert_eq!(Repeat::All.cycle(), Repeat::One);
        assert_eq!(Repeat::One.cycle(), Repeat::Off);
    }

    #[test]
    fn shuffle_keeps_current_and_visits_every_track_once() {
        let mut q = queue(10);
        q.play(4);
        q.set_shuffle(true);
        assert_eq!(q.current(), Some(&4));
        let mut seen = vec![4];
        while let Some(&t) = q.track_ended() {
            seen.push(t);
        }
        seen.sort();
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_off_returns_to_item_order() {
        let mut q = queue(10);
        q.play(0);
        q.set_shuffle(true);
        q.next_track();
        let playing = *q.current().unwrap();
        q.set_shuffle(false);
        assert_eq!(q.current(), Some(&playing));
        assert_eq!(
            q.next_track().copied(),
            (playing + 1 < 10).then_some(playing + 1)
        );
    }

    #[test]
    fn shuffle_does_not_repeat_a_track_across_cycles() {
        for seed in 0..50 {
            let mut q = Queue::new(seed);
            q.replace(vec![0, 1, 2]);
            q.set_shuffle(true);
            q.set_repeat(Repeat::All);
            q.play(0);
            let mut last = 0;
            for _ in 0..20 {
                let t = *q.track_ended().unwrap();
                assert_ne!(t, last, "seed {seed}");
                last = t;
            }
        }
    }

    #[test]
    fn push_next_plays_after_current() {
        let mut q = queue(3);
        q.play(0);
        q.push_next(9);
        assert_eq!(q.items(), &[0, 9, 1, 2]);
        assert_eq!(q.next_track(), Some(&9));
        assert_eq!(q.next_track(), Some(&1));
    }

    #[test]
    fn push_next_with_shuffle_plays_after_current() {
        let mut q = queue(5);
        q.set_shuffle(true);
        q.play(2);
        q.push_next(9);
        assert_eq!(q.next_track(), Some(&9));
        assert_eq!(q.items()[3], 9);
    }

    #[test]
    fn push_next_when_stopped_plays_first() {
        let mut q = queue(2);
        q.push_next(9);
        assert_eq!(q.items(), &[9, 0, 1]);
        assert_eq!(q.play(0), Some(&9));
    }

    #[test]
    fn removing_the_current_track_moves_to_the_next() {
        let mut q = queue(3);
        q.play(1);
        assert_eq!(q.remove(1), Some(1));
        assert_eq!(q.current(), Some(&2));
        assert_eq!(q.remove(1), Some(2));
        assert_eq!(q.current(), None);
    }

    #[test]
    fn removing_an_earlier_track_keeps_the_current_one() {
        let mut q = queue(3);
        q.play(2);
        q.remove(0);
        assert_eq!(q.current(), Some(&2));
        assert_eq!(q.current_index(), Some(1));
    }

    #[test]
    fn moving_items_keeps_the_current_track() {
        let mut q = queue(4);
        q.play(1);
        q.move_item(1, 3);
        assert_eq!(q.items(), &[0, 2, 3, 1]);
        assert_eq!(q.current(), Some(&1));
        q.move_item(0, 2);
        assert_eq!(q.items(), &[2, 3, 0, 1]);
        assert_eq!(q.current(), Some(&1));
        assert_eq!(q.prev_track(), Some(&0));
    }

    #[test]
    fn moving_items_under_shuffle_keeps_play_order() {
        let mut q = queue(5);
        q.set_shuffle(true);
        q.play(0);
        let mut probe = q.clone();
        let expected: Vec<usize> = (0..4).map(|_| *probe.next_track().unwrap()).collect();
        q.move_item(4, 0);
        let actual: Vec<usize> = (0..4).map(|_| *q.next_track().unwrap()).collect();
        assert_eq!(actual, expected);
    }
}
