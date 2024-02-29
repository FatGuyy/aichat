use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

// this is a smart pointer to an AbortSignalInner
pub type AbortSignal = Arc<AbortSignalInner>;

// this struct contains two atomic boolean flags: ctrlc and ctrld
pub struct AbortSignalInner {
    ctrlc: AtomicBool, // indicates whether a Ctrl+C signal has been received
    ctrld: AtomicBool, // indicates whether a Ctrl+D signal has been received
}

// this function creates and returns a new instance of AbortSignal
pub fn create_abort_signal() -> AbortSignal {
    AbortSignalInner::new()
}

impl AbortSignalInner {
    // this function creates and returns a new AbortSignal instance wrapped in an Arc
    pub fn new() -> AbortSignal {
        Arc::new(Self {
            ctrlc: AtomicBool::new(false),
            ctrld: AtomicBool::new(false),
        })
    }

    // this function checks whether either ctrlc or ctrld flag is set, indicating an aborted state
    pub fn aborted(&self) -> bool {
        if self.aborted_ctrlc() {
            return true;
        }
        if self.aborted_ctrld() {
            return true;
        }
        false
    }

    // this function checks the current state of the ctrlc flags
    pub fn aborted_ctrlc(&self) -> bool {
        self.ctrlc.load(Ordering::SeqCst)
    }

    // this function checks the current state of the ctrld flags
    pub fn aborted_ctrld(&self) -> bool {
        self.ctrld.load(Ordering::SeqCst)
    }

    // Resets both the ctrlc and ctrld flags to false
    pub fn reset(&self) {
        self.ctrlc.store(false, Ordering::SeqCst);
        self.ctrld.store(false, Ordering::SeqCst);
    }

    // Set the ctrlc flag to true
    pub fn set_ctrlc(&self) {
        self.ctrlc.store(true, Ordering::SeqCst);
    }

    // Set the ctrld  flag to true
    pub fn set_ctrld(&self) {
        self.ctrld.store(true, Ordering::SeqCst);
    }
}
