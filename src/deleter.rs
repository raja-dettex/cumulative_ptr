pub trait Deleter { 
    unsafe fn delete(&'static self, ptr: *mut dyn Reclaim);
}

impl Deleter for unsafe fn(*mut dyn Reclaim) { 
    unsafe fn delete(&'static self, ptr: *mut dyn Reclaim) { 
        unsafe { (*self)(ptr) }
    }
}

pub mod deleters { 
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
