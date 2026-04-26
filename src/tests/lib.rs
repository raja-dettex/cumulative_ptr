use std::sync::{Arc, atomic::{AtomicPtr, AtomicUsize, Ordering}};

use crate::{HazardPtrDomain, HazardPtrHolder, HazardPtrObject, HazardPtrObjectWrapper, deleters};
struct CountDrops(Arc<AtomicUsize>);
impl Drop for CountDrops { 
    fn drop(&mut self) {
        println!("thsi drop is being called");
        self.0.fetch_add(1, Ordering::SeqCst);
    }
} 

const SHARED_DOMAIN: &'static HazardPtrDomain = HazardPtrDomain::global;
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
    unsafe { HazardPtrObjectWrapper::retire(old, &deleters::DROP_BOX); };

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