//! `Swap`: a dialer whose inner dialer can change while flows run (T-180).
//! A flow that started keeps the inner dialer that it took; a new flow takes
//! the new one. The router puts each network behind a `Swap`, so that a
//! reconnect or a rotate reaches each group, each rule and each door.

use std::sync::{Arc, RwLock};

use tokio::time::Instant;

use super::{BoxFuture, Connected, Datagrams, Dial, Failure, Target};

/// A dialer that forwards to a dialer that can be replaced.
pub struct Swap {
    inner: RwLock<Arc<dyn Dial>>,
}

impl Swap {
    pub fn new(inner: Arc<dyn Dial>) -> Swap {
        Swap {
            inner: RwLock::new(inner),
        }
    }

    /// The dialer now.
    pub fn current(&self) -> Arc<dyn Dial> {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Give new flows `inner`. The old dialer is returned: flows that use it
    /// go on.
    pub fn replace(&self, inner: Arc<dyn Dial>) -> Arc<dyn Dial> {
        let mut now = self.inner.write().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *now, inner)
    }
}

impl Dial for Swap {
    fn label(&self) -> String {
        self.current().label()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        let inner = self.current();
        Box::pin(async move { inner.dial(target, deadline).await })
    }

    fn names_inside(&self) -> bool {
        self.current().names_inside()
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        let inner = self.current();
        Box::pin(async move { inner.datagrams().await })
    }
}

#[cfg(test)]
mod tests {
    use super::super::fakes::{carrier, log, soon, Fake};
    use super::*;

    #[tokio::test]
    async fn swap_sends_new_flows_to_the_new_dialer() {
        let log = log();
        let swap = Swap::new(Arc::new(Fake::up("old", &log)));
        let target: Target = "example.com:443".parse().unwrap();
        let first = swap.dial(&target, soon()).await.unwrap();
        assert_eq!(carrier(first).await, "old");
        let old = swap.replace(Arc::new(Fake::up("new", &log)));
        assert_eq!(old.label(), "old");
        assert_eq!(swap.label(), "new");
        let second = swap.dial(&target, soon()).await.unwrap();
        assert_eq!(carrier(second).await, "new");
    }
}
