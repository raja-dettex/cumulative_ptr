use std::sync::atomic::{ AtomicPtr, Ordering};
use crate::HazardPtr;
use crate::SHARED_DOMAIN;
#[derive(Default)]
pub struct HazardPtrHolder(Option<&'static HazardPtr>);


impl HazardPtrHolder {
    pub fn haz_ptr(&mut self) -> &HazardPtr { 
       let  haz_ptr = if let Some( haz_ptr) = self.0 { 
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
