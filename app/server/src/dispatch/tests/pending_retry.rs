// Retry only the API's explicit pending reply, with the advised interval.

pub(crate) async fn retry_pending<T, F, Fut>(mut call: F, is_pending: impl Fn(&T) -> bool) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let reply = call().await;
            if !is_pending(&reply) {
                return reply;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("background work did not stop within 30 seconds")
}
