//! A utility trait for passing user data

pub trait UserData {
    type Ref<'a>;
}

impl<U> UserData for &'static U {
    type Ref<'a> = &'a U;
}

impl<U> UserData for &'static mut U {
    type Ref<'a> = &'a mut U;
}

impl<U, V> UserData for (&'static mut U, &'static V) {
    type Ref<'a> = (&'a mut U, &'a V);
}

impl<U, V> UserData for (&'static mut U, &'static mut V) {
    type Ref<'a> = (&'a mut U, &'a mut V);
}
