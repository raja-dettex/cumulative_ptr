use std::sync::atomic::{AtomicBool, AtomicPtr};



#[derive(Default)]
pub struct HazardPtr { 
    pub(crate) ptr: AtomicPtr<u8>,
    pub(crate) next: AtomicPtr<HazardPtr>, 
    pub(crate) active: AtomicBool
}

impl HazardPtr { 
    pub(crate) fn protect(&self, ptr: *mut u8) { 
        self.ptr.store(ptr, std::sync::atomic::Ordering::SeqCst);
    }
}