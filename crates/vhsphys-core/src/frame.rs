//! The RGB frame that goes into and comes out of an engine. It is shared by the 525-line and
//! the 625-line engines, so a frame made by one type-checks against the other; only its size
//! differs (the engine's `ACTIVE_W` x `ACTIVE_H`).

/// An RGB frame at the active-picture size of the engine that made it (754 x 480 for the
/// 525-line engine, 740 x 576 for the 625-line one).
#[derive(Clone)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

impl Frame {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, rgb: vec![0; w * h * 3] }
    }
}
