impl<H: ServerHandler> ServerHandler for Tracked<H> {
    fn ping(&self) -> u8 { self.inner.ping() }
    fn initialize(&self) { self.inner.initialize() }
}
