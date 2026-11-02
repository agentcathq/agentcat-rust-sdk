// Fixture: the shape rmcp uses, with the fn items inside a macro and the trait invoking it.
macro_rules! server_handler_methods {
    () => {
        fn ping(&self) -> u8 {
            0
        }
        // a comment with fn ghost()
        fn initialize(&self) {
            let _ = { 1 };
        }
        fn negotiate_initialize(&self) {}
    };
}

#[cfg(feature = "a")]
pub trait ServerHandler: Sized + Send + Sync + 'static {
    server_handler_methods!();
}

#[cfg(not(feature = "a"))]
pub trait ServerHandler: Sized + 'static {
    server_handler_methods!();
}

macro_rules! impl_server_handler_for_wrapper {
    ($wrapper:ident) => {
        impl<T: ServerHandler> ServerHandler for $wrapper<T> {
            fn wrapper_only(&self) {}
        }
    };
}
impl_server_handler_for_wrapper!(Box);
