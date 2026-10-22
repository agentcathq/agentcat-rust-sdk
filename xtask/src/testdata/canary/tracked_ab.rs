impl<H: ServerHandler> ServerHandler for Tracked<H> {
    fn a(&self) -> u32 {
        self.inner.a()
    }
    fn b(&self) {
        self.inner.b();
    }
}
