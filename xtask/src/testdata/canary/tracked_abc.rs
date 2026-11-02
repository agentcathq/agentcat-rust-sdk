impl<H: ServerHandler> ServerHandler for Tracked<H> {
    fn a(&self) -> u32 { self.inner.a() }
    fn b(&self) { self.inner.b(); }
    fn c(&self) -> Option<u8> { self.inner.c() }
}
