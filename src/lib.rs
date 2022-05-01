use std::future::Future;
use std::pin::Pin;
use std::time::Duration;


mod pool;


pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub static TIMEOUT: Duration = Duration::from_secs(5);



#[derive(Debug)]
pub enum SessionResult {
    Closed,
    Timeout,
    Full,
    NoResponse
}


pub enum WorkerState {
    Continue,
    Disconnected,
    Empty
}



#[derive(Debug)]
pub enum Status {
    SenderNotFound,
    SendersRepetive
}





#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
