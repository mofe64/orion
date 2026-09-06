use crate::speech::Chunk;

#[derive(Default)]
pub(crate) struct StartupBuffer {
    pub audio: f64,
    pub generation: f64,
    longest: f64,
    complete_only: bool,
}
impl StartupBuffer {
    pub fn add(&mut self, chunk: &Chunk) -> bool {
        self.audio += chunk.pcm.len() as f64 / 48000.;
        self.generation += chunk.generation_ms / 1000.;
        self.longest = self.longest.max(chunk.generation_ms / 1000.);
        let target = 6f64.max(2. * self.longest + 2.);
        if self.audio >= 6. && (self.generation / self.audio > 0.75 || target > 12.) {
            self.complete_only = true;
        }
        !self.complete_only && self.audio >= target
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(ms: f64) -> Chunk {
        Chunk {
            pcm: vec![0; 48000],
            generation_ms: ms,
            synthesis_ms: ms,
        }
    }
    #[test]
    fn fast_generation_holds_six_seconds() {
        let mut buffer = StartupBuffer::default();
        for _ in 0..5 {
            assert!(!buffer.add(&chunk(100.)));
        }
        assert!(buffer.add(&chunk(100.)));
    }
    #[test]
    fn slow_generation_and_long_pauses_latch_complete_buffering() {
        for ms in [1000., 6000.] {
            let mut buffer = StartupBuffer::default();
            for _ in 0..6 {
                assert!(!buffer.add(&chunk(ms)));
            }
            for _ in 0..30 {
                assert!(!buffer.add(&chunk(1.)));
            }
        }
    }
    #[test]
    fn decoder_pause_increases_required_reserve() {
        let mut buffer = StartupBuffer::default();
        buffer.add(&chunk(3000.));
        for _ in 0..6 {
            assert!(!buffer.add(&chunk(100.)));
        }
        assert!(buffer.add(&chunk(100.)));
    }
}
