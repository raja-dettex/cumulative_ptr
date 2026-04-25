use std::ops::{Deref, DerefMut};
use crate::{HazardPtrDomain, SHARED_DOMAIN, Deleter, Reclaim};
pub trait HazardPtrObject
where Self:  Sized + 'static
{
    fn domain(&self) -> &HazardPtrDomain;
    // safety contracts
    // caller has to gurantee that the pointer addrss is valid
    // caller also has to gurantee that self is no longer accessible by other readers,
    // caller also has to make sure that deleter is valid drop anyway 
    // so its okay to deref it. 
    unsafe fn retire(me: *mut Self, deleter: &'static dyn Deleter) {
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