use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::ops::{Deref, DerefMut};
static SHARED_DOMAIN: HazardPtrDomain = HazardPtrDomain { 
    retired: RetiredList { 
        head: AtomicPtr::new(std::ptr::null_mut()),
        count: AtomicUsize::new(0)
        
    },
    hazptrs: HazardPtrs {
        head: AtomicPtr::new(std::ptr::null_mut())
    }
};
#[derive(Default)]
pub struct HazardPtrHolder(Option<&'static HazardPtr>);
pub struct HazardPtr { 
    ptr: AtomicPtr<u8>,
    next: AtomicPtr<HazardPtr>, 
    active: AtomicBool
}

impl HazardPtr { 
    pub fn protect(&self, ptr: *mut u8) { 
        self.ptr.store(ptr, std::sync::atomic::Ordering::SeqCst);
    }
}
// this is sort of classic holder of any pointer which is atomic ptr pointing to raw pointer
//  
impl HazardPtrHolder {
    pub fn haz_ptr(&mut self) -> &HazardPtr { 
       let mut haz_ptr = if let Some(mut haz_ptr) = self.0 { 
            haz_ptr
        } else { 
            let haz_ptr = SHARED_DOMAIN.acquire();
            self.0 = Some(haz_ptr);
            haz_ptr
        };
        haz_ptr 
    }
    // safety contract
    // caller has to gurantee the pointer address is valid by reference or null and aligned
    // and the pointer will only be dealloacated only by `[HazardPtrObject::retire]` 
    pub unsafe fn load<'a, T>(&'a mut self, ptr: &'_ AtomicPtr<T>) -> Option<&'a T> { 
        let haz_ptr = self.haz_ptr();
        let mut ptr1 = ptr.load(std::sync::atomic::Ordering::SeqCst);
        loop { 
            haz_ptr.protect(ptr1 as *mut u8);
            let ptr2 = ptr.load(std::sync::atomic::Ordering::SeqCst);
            if ptr1 == ptr2 { 
                // ptr is protected
                break std::ptr::NonNull::new(ptr1).map(|nn| {
                    // this is safe beacause
                    // 
                    // ptr is valid and will not be deallocated for the returned lifetime of 
                    // target pointer
                    // ptr address is also valid by the safety contract of load 
                    unsafe { nn.as_ref()}
                });
            } else { 
                ptr1 = ptr2;
            }
        }
    }
    pub fn reset(&mut self)  { 
        if let Some(ptr) = self.0 { 
            ptr.ptr.store(std::ptr::null_mut(), std::sync::atomic::Ordering::SeqCst);
        }
    }
}

impl Drop for HazardPtrHolder { 
    fn drop(&mut self) { 
        self.reset();
        if let Some(haz_ptr) = self.0 { 
            haz_ptr.active.store(false, Ordering::SeqCst);
        }
    }
}

pub trait Deleter { 
    unsafe fn delete(&'static self, ptr: *mut dyn Reclaim);
}

impl Deleter for unsafe fn(*mut dyn Reclaim) { 
    unsafe fn delete(&'static self, ptr: *mut dyn Reclaim) { 
        unsafe { (*self)(ptr) }
    }
}

pub mod deleter { 
    use super::*;

    unsafe fn drop_box(ptr: *mut dyn Reclaim) { 
        println!("drop box is bieng called");
            let _ = unsafe { Box::from_raw(ptr)};
    }

    pub static DROP_BOX: unsafe fn(*mut dyn Reclaim) = drop_box;


    unsafe fn drop_in_place(ptr: *mut dyn Reclaim) { 
            unsafe { std::ptr::drop_in_place(ptr)};
        }

    pub static DROP_IN_PLACE: unsafe fn(*mut dyn Reclaim) = drop_in_place;

}

pub trait Reclaim {}
impl<T> Reclaim for T {}

pub trait HazardPtrObject
where Self: Reclaim + Sized + 'static
{
    fn domain(&self) -> &HazardPtrDomain;
    // safety contracts
    // caller has to gurantee that the pointer addrss is valid
    // caller also has to gurantee that self is no longer accessible by other readers,
    // caller also has to make sure that deleter is valid drop anyway 
    // so its okay to deref it. 
    unsafe fn retire(me: *mut Self, deleter: &'static dyn Deleter) {
        if !std::mem::needs_drop::<Self>() {
            return;
        } 
        unsafe { &*me }.domain().retire(me as *mut dyn Reclaim, deleter) 
    }
}


// so here is the thing any raw pointer of any type T; this is just the wrapper type of 
// that raw pointer and thus by dereferncing it will handover the pointer which is the raw one. 

pub struct HazardPtrObjectWrapper<T> { 
    inner: T, 
}

impl<T: 'static> HazardPtrObject for HazardPtrObjectWrapper<T> { 
    fn domain(&self) -> &HazardPtrDomain { 
        &SHARED_DOMAIN
    }    
}
impl<T> HazardPtrObjectWrapper<T> { 
    pub fn new_with_default(t: T ) -> Self { 
        Self { inner: t}
    }
} 
impl<T> Deref for HazardPtrObjectWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for HazardPtrObjectWrapper<T> { 
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

struct HazardPtrs { 
    head: AtomicPtr<HazardPtr>
}

struct Retired { 
    ptr: *mut dyn Reclaim,
    deleter: &'static dyn Deleter,
    next: AtomicPtr<Retired>
}

struct RetiredList { 
    head: AtomicPtr<Retired>,
    count: AtomicUsize
}
// holds a list of all acquried haz ptrs
// this domain is sort of does not depend on the target type: could be Box, Vec whatever
// this `HazardPtrDomain` is a slab of collection of pointers those need to be reclaimed, 
pub struct HazardPtrDomain { 
    hazptrs: HazardPtrs,
    retired: RetiredList
}
impl HazardPtrDomain { 
    pub fn acquire(&self) -> &'static HazardPtr { 
        let head_ptr = &self.hazptrs.head;
        let mut node = head_ptr.load(Ordering::SeqCst);
        let node = loop { 
            while !node.is_null() && unsafe { &* node}.active.load(Ordering::SeqCst) { 
               node = unsafe { &* node}.next.load(Ordering::SeqCst);   
            }
            if node.is_null() { 
                // allocate the node
                // if there is no like free ptr that is non null and not already been acquired
                // create a new node
                let haz_ptr  = Box::into_raw(Box::new(HazardPtr{ 
                    ptr: AtomicPtr::new(std::ptr::null_mut()),
                    next: AtomicPtr::new(std::ptr::null_mut()),
                    active: AtomicBool::new(true)
                }));
                // stick it to the head of the linked list
                let mut head = head_ptr.load(Ordering::SeqCst);
                break loop { 
                    *unsafe {&mut *haz_ptr}.next.get_mut() = head;
                    match head_ptr.compare_exchange_weak(
                        head, 
                            haz_ptr, 
                        Ordering::SeqCst, 
                        Ordering::SeqCst) { 
                            Ok(_) => break haz_ptr,
                            Err(head_already) => head = head_already
                        }
                }
            } else { 
                if unsafe { &* node}.active.compare_exchange_weak(
                    false, 
                        true, 
                    Ordering::SeqCst,
                    Ordering::SeqCst).is_ok() { 
                        break node;
                } else {  
                    // some one else just acquired before we are going to do it
                }
            }
        };
        unsafe { &* node }
    }
    pub fn retire(&self, ptr: *mut dyn Reclaim, deleter: &'static dyn Deleter) {
        // first stick to the list of retired
        let retired = Box::into_raw(Box::new(Retired { 
            ptr,
            deleter,
            next: AtomicPtr::new(std::ptr::null_mut())
        }));
        
        let head_ptr = &self.retired.head;
        let mut head  = head_ptr.load(Ordering::SeqCst); 
        // increment the counter before proceeding to add to the list of retired objects
        self.retired.count.fetch_add(1, Ordering::SeqCst);
        loop { 
            *unsafe {&mut *retired}.next.get_mut() = head;
            match head_ptr.compare_exchange_weak(
                head, 
                retired, 
                Ordering::SeqCst, 
                Ordering::SeqCst) { 
                Ok(_) => break,
                Err(head_already) =>  { 
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
       let steal = self.retired.head.swap(
        std::ptr::null_mut(), 
        Ordering::SeqCst);
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
            let n = unsafe {&*current};
            node = n.next.load(Ordering::SeqCst);
            if guarded_ptrs.contains(&(n.ptr as *mut u8)) { 
                // being guarded by readers and writers not safe to reclaim
                n.next.store(remaining, Ordering::SeqCst);
                remaining = current;
                if tail.is_none() { 
                    tail = Some(remaining);
                }
            } else { 
                let n = unsafe { Box::from_raw(current)};
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
        let mut head  = head_ptr.load(Ordering::SeqCst); 
        // increment the counter before proceeding to add to the list of retired objects
        self.retired.count.fetch_add(1, Ordering::SeqCst);
        loop { 
            *unsafe {&mut *tail}.next.get_mut() = head;
            match head_ptr.compare_exchange_weak(
                head, 
                remaining, 
                Ordering::SeqCst, 
                Ordering::SeqCst) { 
                Ok(_) => break,
                Err(head_already) =>  { 
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
    
    pub fn eager_reclaim(&self, block: bool) -> usize  {
        return self.bulk_reclaim(0, block);
    }
}


#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::{AtomicPtr, AtomicUsize, Ordering}};

    use crate::{HazardPtrHolder, HazardPtrObject, HazardPtrObjectWrapper, SHARED_DOMAIN, deleter};
   struct CountDrops(Arc<AtomicUsize>);
   impl Drop for CountDrops { 
    fn drop(&mut self) {
        println!("thsi drop is being called");
        self.0.fetch_add(1, Ordering::SeqCst);
    }
   } 
    #[test]
   fn first_test() { 
        println!("set started");
        let drops_42 = Arc::new(AtomicUsize::new(0));
        let x = AtomicPtr::new(Box::into_raw(Box::new(
            HazardPtrObjectWrapper::new_with_default((42 as i32, CountDrops(Arc::clone(&drops_42))))
        )));
        // as a reader
        let mut holder = HazardPtrHolder::default();
        
        let my_value = unsafe { holder.load(&x) .expect("not null") };
        assert_eq!(my_value.0, 42);
        
        holder.reset();

        // invalid becasue we have reset it
        //let _ = **my_value;
        let my_value = unsafe { holder.load(&x) .expect("not null") };
        // valid
        assert_eq!(my_value.0, 42);
        //drop(holder);

        // invalid again

        let mut holder_temp = HazardPtrHolder::default();
        let _ = unsafe { holder_temp.load(&x).expect("not null") };
        drop(holder_temp);
        //assert_eq!(val_temp.0, 42);
        let drops_16 = Arc::new(AtomicUsize::new(0));
        
        // as a writer 
        let old = x.swap(
            Box::into_raw(Box::new(
                HazardPtrObjectWrapper::new_with_default((16, CountDrops(Arc::clone(&drops_16))))
            )),
            std::sync::atomic::Ordering::SeqCst
        );


        // the ptr came from box , so always valid,
        // retire is being called only by hazardptrobject 
        // old is no longer in use, have already been swapped, safe to retire
        unsafe { HazardPtrObjectWrapper::retire(old, &deleter::DROP_BOX); };

        // we have swapped the the raw pointer with new value, and then we have retired via hazard pointer object retired,
        // i think the wrapper type of objectWrapper from where the raw pointer came from ( e.g Box) that
        // destructor is being called, but the actual raw pointer is still there, i still wonder how i am still able 
        // to deref the hazardptr, 

        // let mut holder_2 = HazardPtrHolder::default();
        // let _ = unsafe { holder_2.load(&x).expect("not null") };
        // drop(holder_2); 
        //assert_eq!(my_value_x2.0, 16);
        assert_eq!(drops_42.load(Ordering::SeqCst), 0);

        assert_eq!(my_value.0, 42);
        assert_eq!(drops_42.load(Ordering::SeqCst), 0);

        let _ = SHARED_DOMAIN.eager_reclaim(false);
        assert_eq!(drops_42.load(Ordering::SeqCst), 0);

        assert_eq!(my_value.0, 42);
        drop(holder);
        
        let n = SHARED_DOMAIN.eager_reclaim(false);
        assert_eq!(n, 1);
        assert_eq!(drops_42.load(Ordering::SeqCst), 1);
        

        // TODO: check wheather it is reclaimed

    }
}