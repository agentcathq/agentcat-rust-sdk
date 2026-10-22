// Fixture: a plain trait body with three methods and a default body that holds blocks.
pub trait ServerHandler: Sized + Send + Sync + 'static {
    /// A doc comment with a brace { that must not count.
    fn a(&self) -> u32 {
        if true {
            let _s = "} not a brace {";
            1
        } else {
            2
        }
    }
    fn b(&self) {}
    fn c(&self) -> Option<u8> {
        match 1 {
            _ => {
                let c = '}';
                let _ = c;
                None
            }
        }
    }
}

macro_rules! unrelated {
    () => {
        fn must_not_be_collected() {}
    };
}
