use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use crate::{Deleter, HazardPtr, Reclaim};
pub(crate) static SHARED_DOMAIN: HazardPtrDomain = HazardPtrDomain::new();

struct HazardPtrs {
    head: AtomicPtr<HazardPtr>,
}

struct Retired {
    ptr: *mut dyn Reclaim,
    deleter: &'static dyn Deleter,
    next: AtomicPtr<Retired>,
}
impl Retired {
    pub fn new<'domain>(
        _: &'domain HazardPtrDomain,
        ptr: *mut (dyn Reclaim + 'domain),
        deleter: &'static dyn Deleter,
    ) -> Self {
        Retired {
            ptr: unsafe { ptr as *mut dyn Reclaim},
            deleter,
            next: AtomicPtr::new(std::ptr::null_mut())
        }
    }
}
struct RetiredList {
    head: AtomicPtr<Retired>,
    count: AtomicUsize,
}
// holds a list of all acquried haz ptrs
// this domain is sort of does not depend on the target type: could be Box, Vec whatever
// this `HazardPtrDomain` is a slab of collection of pointers those need to be reclaimed,
pub struct HazardPtrDomain {
    hazptrs: HazardPtrs,
    retired: RetiredList,
}
impl HazardPtrDomain {
    pub const global: &'static HazardPtrDomain = &SHARED_DOMAIN;
    pub const fn new() -> Self {
        Self {
            retired: RetiredList {
                head: AtomicPtr::new(std::ptr::null_mut()),
                count: AtomicUsize::new(0),
            },
            hazptrs: HazardPtrs {
                head: AtomicPtr::new(std::ptr::null_mut()),
            },
        }
    }
    pub fn acquire(&self) -> &HazardPtr {
        let head_ptr = &self.hazptrs.head;
        let mut node = head_ptr.load(Ordering::SeqCst);
        let node = loop {
            while !node.is_null() && unsafe { &*node }.active.load(Ordering::SeqCst) {
                node = unsafe { &*node }.next.load(Ordering::SeqCst);
            }
            if node.is_null() {
                // allocate the node
                // if there is no like free ptr that is non null and not already been acquired
                // create a new node
                let haz_ptr = Box::into_raw(Box::new(HazardPtr {
                    ptr: AtomicPtr::new(std::ptr::null_mut()),
                    next: AtomicPtr::new(std::ptr::null_mut()),
                    active: AtomicBool::new(true),
                }));
                // stick it to the head of the linked list
                let mut head = head_ptr.load(Ordering::SeqCst);
                break loop {
                    *unsafe { &mut *haz_ptr }.next.get_mut() = head;
                    match head_ptr.compare_exchange_weak(
                        head,
                        haz_ptr,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break haz_ptr,
                        Err(head_already) => head = head_already,
                    }
                };
            } else {
                if unsafe { &*node }
                    .active
                    .compare_exchange_weak(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    break node;
                } else {
                    // some one else just acquired before we are going to do it
                }
            }
        };
        unsafe { &*node }
    }
    pub fn retire<'domain>(
        &'domain self,
        ptr: *mut (dyn Reclaim + 'domain),
        deleter: &'static dyn Deleter,
    ) {
        // first stick to the list of retired
        let retired = Box::into_raw(Box::new(Retired::new(self, ptr, deleter)));

        let head_ptr = &self.retired.head;
        let mut head = head_ptr.load(Ordering::SeqCst);
        // increment the counter before proceeding to add to the list of retired objects
        self.retired.count.fetch_add(1, Ordering::SeqCst);
        loop {
            *unsafe { &mut *retired }.next.get_mut() = head;
            match head_ptr.compare_exchange_weak(head, retired, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(head_already) => {
                    // note: head is already changed stick swap the new head
                    head = head_already
                }
            }
        }
        //lets do the reclaim
        // if the count is bigger than zero reclaim the objects
        if self.retired.count.load(Ordering::SeqCst) != 0 {
            self.bulk_reclaim(0, false);
        }
        // compare the value in the tables, not the vtables,
        // (ptr, d)
    }
    fn bulk_reclaim(&self, prev_reclaimed: usize, block: bool) -> usize {
        let steal = self
            .retired
            .head
            .swap(std::ptr::null_mut(), Ordering::SeqCst);
        if steal.is_null() {
            // nothing to reclaim (might be already reclaimed or something) fallback
            return 0;
        }
        let mut guarded_ptrs = HashSet::new();
        // walk the list of haz ptrs and find all the ptrs those are still being guarded
        let mut node = self.hazptrs.head.load(Ordering::SeqCst);
        while !node.is_null() {
            let n = unsafe { &*node };
            if n.active.load(Ordering::SeqCst) {
                guarded_ptrs.insert(n.ptr.load(Ordering::SeqCst));
            }
            node = n.next.load(Ordering::SeqCst);
        }

        // now walk the list of retired ptrs beginning from the steal
        let mut node = steal;
        let mut remaining = std::ptr::null_mut();
        let mut reclaimed = 0usize;
        let mut tail = None;
        while !node.is_null() {
            let current = node;
            let n = unsafe { &*current };
            node = n.next.load(Ordering::SeqCst);
            if guarded_ptrs.contains(&(n.ptr as *mut u8)) {
                // being guarded by readers and writers not safe to reclaim
                n.next.store(remaining, Ordering::SeqCst);
                remaining = current;
                if tail.is_none() {
                    tail = Some(remaining);
                }
            } else {
                let n = unsafe { Box::from_raw(current) };
                // now we can reclaim it no longer being guarded
                reclaimed += 1;
                unsafe { n.deleter.delete(n.ptr) }
            }
        }
        self.retired.count.fetch_sub(reclaimed, Ordering::SeqCst);
        let total_reclaimed = prev_reclaimed + reclaimed;
        let tail = if let Some(tail) = tail {
            assert!(!remaining.is_null());
            tail
        } else {
            assert!(remaining.is_null());
            return total_reclaimed;
        };

        let head_ptr = &self.retired.head;
        let mut head = head_ptr.load(Ordering::SeqCst);
        // increment the counter before proceeding to add to the list of retired objects
        self.retired.count.fetch_add(1, Ordering::SeqCst);
        loop {
            *unsafe { &mut *tail }.next.get_mut() = head;
            match head_ptr.compare_exchange_weak(
                head,
                remaining,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(head_already) => {
                    // note: head is already changed stick swap the new head
                    head = head_already
                }
            }
        }

        if !remaining.is_null() && block {
            // caller wants to reclaim if anythign is left, must call reclaim
            std::thread::yield_now();
            // tail recursion
            return self.bulk_reclaim(reclaimed, true);
        }
        reclaimed
    }

    pub fn eager_reclaim(&self, block: bool) -> usize {
        return self.bulk_reclaim(0, block);
    }
}
