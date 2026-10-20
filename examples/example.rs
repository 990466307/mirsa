#![feature(step_trait)]
use core::iter::Step;
use std::alloc::Layout;
use std::ffi::{CStr, c_char};
use std::num::NonZero;
use std::ptr::NonNull;

fn main() {
    unsafe {
        let vec_ptr = std::ptr::null_mut::<u8>();
        let _ = Vec::from_raw_parts(vec_ptr, 0, 0);

        let size = 8usize;
        let zero_align = 0usize;
        let _ = Layout::from_size_align_unchecked(size, zero_align);

        let too_large = 0x11_0000_u32;
        let _ = char::from_u32_unchecked(too_large);

        let cstr_ptr = std::ptr::null::<c_char>();
        let _ = CStr::from_ptr(cstr_ptr);

        let zero = 0i32;
        let _ = NonZero::<i32>::new_unchecked(zero);

        let nonnull_ptr = std::ptr::null_mut::<u8>();
        let _ = NonNull::new_unchecked(nonnull_ptr);

        let ptr = std::ptr::null::<u8>();
        let _ = core::ptr::read(ptr);

        let write_ptr = std::ptr::null_mut::<u8>();
        core::ptr::write(write_ptr, 1);

        let slice_mut_ptr = std::ptr::null_mut::<u8>();
        let _ = std::slice::from_raw_parts_mut(slice_mut_ptr, 1);

        let mut mutable_values = [10u8, 20, 30, 40];
        let bad_mut_index = 7usize;
        let _ = mutable_values.get_unchecked_mut(bad_mut_index);

        let mut split_mut_values = [1u8, 2, 3];
        let bad_mut_mid = 6usize;
        let _ = split_mut_values.split_at_mut_unchecked(bad_mut_mid);

        let valid_signed_start = -120_i8;
        let valid_signed_count = 200_usize;
        let _ = <i8 as Step>::forward_unchecked(valid_signed_start, valid_signed_count);
    }
}
