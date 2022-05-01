use crate::WorkerState;
use crate::BoxFuture;
use crate::SessionResult;
use std::fmt;
use std::collections::VecDeque;

use tokio::sync::mpsc::error::{TrySendError, SendTimeoutError};
use tokio::sync::{mpsc, oneshot};



use crate::TIMEOUT;


/// Pool is a lightweight, generic pooling library for Rust tokio 
/// with a focus on simplicity, performance, and rock-solid disaster recovery.
/// 
/// caller checkout a resource from pool use it
/// after finish checkin it to pool
/// 
/// if resource destroy, instead of calling checkin
/// just call `session::destroyed(&self, resource: Resource<T>)`
/// 
/// if forget call `session::checkin(&self, resource: Resource<T>)` 
/// and `session::destroyed(&self, resource: Resource<T>)`
/// 
/// resource itself impl drop but if pool is under load
/// may cpu usage high because send to channel with try_send in loop
/// 
/// **Recommended** 
/// 
/// after job done with resource call `checkin` or if destroy resource
/// call `destroyed`
/// 
/// 
/// # Example
/// 
/// ```rust
/// 
/// #[tokio::main]
///  async fn main() {
///  
///      let pool_size = 7;
///      let max_overflow = 4;
///  
///  
///      // Create a session for communicate with pool channel
///      let session = Pool::new(pool_size, 
///                              max_overflow, 
///                              || Box::pin(async move {
///                                  Connection                                              
///                              })).await.run_service();                            
///      
///      // session.clone() internally call channel mpsc::Sender::Clone
///      printer(session.clone()).await;
///      
///      printer(session.clone()).await;
///      
///      printer(session).await;
///  }
///  
///  pub async fn printer(mut session: Session<Connection>) {
///  
///  
///      // Checkout a resource from Pool
/// 
///      // block_checkout don't block your scheduler
///      // just await on oneshot with timeout
///      if let Ok(mut resource) = session.block_checkout().await {
///          
///          // call operaiton here
///          resource.get().print();
///  
///          // after job done, call checkin
///          session.checkin(resource).await;
///  
///          // OR if resource destroy, call this
///          // session.destroyed(resource).await;
///      } 
///  }
///  
///  
///  
///  #[derive(Debug)]
///  pub struct Connection;
///  impl Connection {
///      pub fn print(&self){ 
///          println!("==> Print!")
///      }
///  }
/// 
/// 
/// ```
/// 
/// 

#[derive(Debug)]
pub struct Resource<T> {
    resource: T,
    manager_sender: mpsc::Sender<Request<T>>,

    // if onetime called recycle, send notify recycle to manager and change to true this 
    // just one time can called
    recycled: bool
}

impl<T> Drop for Resource<T> {
    fn drop(&mut self) {

        // if recycle notify not sended before 
        if !self.recycled {
            loop {
                match self.manager_sender.try_send(Request::Recycle) {
                    Ok(_) => break,
                    Err(TrySendError::Closed(_)) => break,
                    Err(TrySendError::Full(_)) => (),                    
                }
            }
            return;
        }
    }
}

impl<T> Resource<T> {

    fn new(resource: T, manager_sender: mpsc::Sender<Request<T>>) -> Self {
        Resource {
            resource,
            manager_sender,
            recycled: false
        }
    }

    /// borrow resource
    pub fn get(&mut self) -> &mut T {
        &mut self.resource
    }


    /// if resource failed and cannot recover, call this and drop it
    pub async fn recycle(mut self) {

        // if recycle notify not sended before 
        if !self.recycled {            
            let res = self.manager_sender.send_timeout(Request::Recycle, TIMEOUT).await;
            if let Ok(_) = res {
                // if sending was successful change to true
                self.recycled = true;
            }
        }
    }

    
}




pub enum Request<T> {
    
    Checkin(Resource<T>),
    
    /// if not exist block until avail resource
    BlockCheckout(oneshot::Sender<Option<Resource<T>>>),

    /// if not exist return None
    NonBlockCheckout(oneshot::Sender<Option<Resource<T>>>),

    /// when we get this, its mean is it dropped and we create new
    Recycle,

    PrintStatistics,

    /// not recev new Checkout
    ///  continues until all waiting handled.
    ShutdownSafe,
}


pub struct Pool<T, F> 
where
    F: Fn() -> BoxFuture<T> + Send + 'static,
    T: Send + 'static
{
    recv: mpsc::Receiver<Request<T>>,
    self_sender: mpsc::Sender<Request<T>>,
    factory: F,

    resources: VecDeque<Resource<T>>,
    waiting: VecDeque<oneshot::Sender<Option<Resource<T>>>>,

    size: usize,
    max_overflow: usize,
    overflow_worker: usize,

    shutdown: bool,
}

impl<T, F> fmt::Debug for Pool<T, F> 
where
    F: Fn() -> BoxFuture<T> + Send + 'static,
    T: Send + 'static
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pool")
            .field("size", &self.size)
            .field("max_overflow", &self.max_overflow)
            .field("overflow_worker", &self.overflow_worker)
            .field("resources", &self.resources.len()).finish()
    }
}


impl<T, F> Pool<T, F> 
where
    F: Fn() -> BoxFuture<T> + Send + 'static,
    T: fmt::Debug + Send + 'static
{
    
    /// size is queue resources 
    /// 
    /// max_overflow is maximum resource can create if queue is empty
    /// 
    /// factory, create new resource  
    pub async fn new (pool_size: usize, 
                      max_overflow: usize, 
                      factory: F) -> Self {

        let (self_sender, recv) = mpsc::channel(50);
        
        let mut pool = Pool { 
            recv,
            self_sender,
            factory, 
            resources: VecDeque::new(),
            waiting: VecDeque::new(),
            size: pool_size,
            max_overflow,
            overflow_worker: 0,
            shutdown: false
        };

        for _ in 0..pool_size {
            pool.factory_resource().await;
        }

        pool
    }


    pub fn run_service(mut self) -> Session<T> {

        let session = Session::new(self.self_sender.clone());

        tokio::spawn(async move {
            loop {
                let res = self.recv.recv().await;
                if let WorkerState::Disconnected = self.handle_recv(res).await {
                    return ();
                }
            }
        });

        return session
    }


    #[inline]
    async fn handle_recv(&mut self, res: Option<Request<T>>) -> WorkerState {
        match res {
            Some(req) => {
                match req {
                    Request::Checkin(resource) => {
                        
                        // --------------- Come to sending resource state -----------------------

                        let mut resource = Some(resource);

                        // while waiting queue not empty
                        while let Some(resp) = self.waiting.pop_front() {
                            // if receiver not drop channel
                            match resp.send(resource) {
                                Ok(_) => {
                                    // pool think, sending was successful 
                                    return WorkerState::Continue
                                }
                                Err(res) => {
                                    resource = res;
                                }
                            }
                        }

                        // not exist any waiting, 
                        if self.shutdown {
                            return WorkerState::Disconnected
                        }
                        
                        // unwrap because always Some not None
                        self.checkin(resource.unwrap());
                        return WorkerState::Continue
                    }
                    Request::BlockCheckout(resp) => {

                        if self.shutdown {
                            let _ = resp.send(None);
                            return  WorkerState::Continue
                        }

                        match self.checkout().await {
                            Some(resource) => {

                                let resource = Some(resource);

                                // send resource to oneshot channel
                                match resp.send(resource) {
                                    // pool think, sending was successful
                                    Ok(_) => (),

                                    // if receiver dropped channel
                                    Err(mut resource) => {

                                        // --------------- Come to sending resource state -----------------------                                    

                                        // while waiting queue not empty
                                        while let Some(resp) = self.waiting.pop_front() {
                                            // if receiver not drop channel
                                            match resp.send(resource) {
                                                Ok(_) => {
                                                    // pool think, sending was successful 
                                                    return WorkerState::Continue
                                                }
                                                Err(res) => {
                                                    resource = res;
                                                }
                                            }
                                        }

                                        // not exist any waiting handle checkin
                                        self.checkin(resource.unwrap());
                                    }
                                }

                            }
                            None => {
                                // not exist any resource, block until available
                                self.waiting.push_back(resp);
                            }
                        }
                        return WorkerState::Continue                        
                    }
                    Request::NonBlockCheckout(resp) => {
                       
                        if self.shutdown {
                            let _ = resp.send(None);
                            return WorkerState::Continue;
                        }
                       
                        match self.checkout().await {
                            Some(resource) => {

                                let resource = Some(resource);

                                // send resource to oneshot channel
                                if let Err(mut resource) = resp.send(resource) {

                                    // --------------- Come to sending resource state -----------------------

                                    // while waiting queue not empty
                                    while let Some(resp) = self.waiting.pop_front() {
                                        // if receiver not drop channel
                                        match resp.send(resource) {
                                            Ok(_) => {
                                                // pool think, sending was successful 
                                                return WorkerState::Continue
                                            }
                                            Err(res) => {
                                                resource = res;
                                            }
                                        }
                                    }

                                    // not exist any waiting handle checkin
                                    self.checkin(resource.unwrap());
                                }

                            }
                            None => {
                                // not exist any resource, block until available
                                let _ = resp.send(None);
                            }
                        }
                        return WorkerState::Continue
                    }
                    Request::ShutdownSafe => {
                        self.shutdown = true;
                        return WorkerState::Continue
                    }
                    Request::Recycle => {
                                            
                        // if overflow_worker is greater than zero decrease
                        if self.overflow_worker > 0 {                                                
                            self.overflow_worker -= 1;
                        
                        } else {

                            // create resource
                            self.factory_resource().await;
                            
                        }
                    
                    
                        // --------------- Come to sending resource state -----------------------


                        let resource = self.checkout().await.unwrap();

                        let mut resource = Some(resource);

                        // while waiting queue not empty
                        while let Some(resp) = self.waiting.pop_front() {
                            // if receiver not drop channel
                            match resp.send(resource) {
                                Ok(_) => {
                                    // pool think, sending was successful 
                                    return WorkerState::Continue
                                }
                                Err(res) => {
                                    resource = res;
                                }
                            }
                        }

                        // not exist any waiting, 
                        if self.shutdown {
                            return WorkerState::Disconnected
                        }
                        
                        // unwrap because always Some not None
                        self.checkin(resource.unwrap());
                        return WorkerState::Continue
                    }
                    Request::PrintStatistics => {
                        println!("==> {:?}", &self);
                        return WorkerState::Continue
                    }
                }
            }
            None => WorkerState::Disconnected
        }
    }



    /// add a resource
    #[inline]
    fn checkin(&mut self, mut resource: Resource<T>) {

        // if called recycled its mean this failed and pool get Recycle notify already
        if resource.recycled {
            return;
        }

        // if overflow_worker is greater than zero decrease
        if self.overflow_worker > 0 {


            // # Trick !!!

            // if caller not set recycled to true,
            // after drop channel got a Recycle signal,
            // then here set to true to skip take a signal
            resource.recycled = true;

            self.overflow_worker -= 1;
            return;
        }


        // resources is not full already
        self.resources.push_back(resource);
    }

    /// get a resource
    #[inline]
    async fn checkout(&mut self) -> Option<Resource<T>> {
        
        // if not exist any resource         
        if self.resources.len() == 0 {
            
            // check if overflow_worker was not full create resource
            return self.factory_overflow().await
        }

        // exist resource 
        return self.resources.pop_front()
    }




    /// create new resource and push(back it)
    #[inline]
    async fn factory_resource(&mut self) {
        let resource = (self.factory)();
        self.resources.push_back(Resource::new(resource.await, self.self_sender.clone()));
    } 


    #[inline]
    async fn factory_overflow(&mut self) -> Option<Resource<T>> {
        
        // if can create overflow_worker resource 
        if self.overflow_worker < self.max_overflow {
            let resource = (self.factory)();
            self.overflow_worker += 1;
            
            let res = Resource::new(resource.await, self.self_sender.clone());
            return Some(res)
        } 
        
        
        // overflow_worker is full
        return None
    }

}




// --------------------- Client Code --------------------------


pub struct Session<T> {
    sender: mpsc::Sender<Request<T>>
}

impl<T> Session<T> 
where
    T: Send + 'static
{
    fn new(sender: mpsc::Sender<Request<T>>) -> Self {
        Session { 
            sender 
        }
    }

    /// create new session
    pub fn clone(&self) -> Self {
        let sender = self.sender.clone();
        Session { 
            sender 
        }
    }

    /// checkin a resource
    pub async fn checkin(&self, resource: Resource<T>) -> Result<(), SessionResult> {
        let res = self.sender.send_timeout(Request::Checkin(resource), TIMEOUT).await;
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                match e {
                    SendTimeoutError::Timeout(_) => Err(SessionResult::Timeout),
                    SendTimeoutError::Closed(_) => Err(SessionResult::Closed),
                }
            }
        }
    }   


    // Checkout a resource from Pool
    //
    // block_checkout don't block your scheduler
    // if not exist resource await until avail with timeout
    pub async fn block_checkout(&self) -> Result<Resource<T>, SessionResult> {
        
        // create oneshot channel
        let (ask, resp) = oneshot::channel();
        
        // create a request
        let req = Request::BlockCheckout(ask);
        
        
        // send request with timeout (5 seconds)
        let res = self.sender.send_timeout(req, TIMEOUT).await;
           
        match res {
            // Closed
            Err(SendTimeoutError::Closed(_req)) => {
                return Err(SessionResult::Closed)
            }
            // Timeout
            Err(SendTimeoutError::Timeout(_req)) => {
                return Err(SessionResult::Timeout)
            }

            // Sending request was successful
            Ok(_) => {
                // Await for resource
                match resp.await {
                    Ok(oresource) => {

                        // because BlockCheckout always send resource
                        let resource = unsafe {
                            oresource.unwrap_unchecked()
                        };
                        Ok(resource)
                    }
                    Err(_) => {
                        return Err(SessionResult::NoResponse)
                    }
                }
            }
        }
    }


    // Checkout a resource from Pool
    //
    // block_checkout don't block your scheduler
    // if not exist resource send full
    pub async fn nonblock_checkout(&self) -> Result<Resource<T>, SessionResult> {
        
        // create oneshot channel
        let (ask, resp) = oneshot::channel();
        
        // create a request
        let req = Request::NonBlockCheckout(ask);
        
        
        // send request with timeout (5 seconds)
        let res = self.sender.send_timeout(req, TIMEOUT).await;
           
        match res {
            // Closed
            Err(SendTimeoutError::Closed(_req)) => {
                return Err(SessionResult::Closed)
            }
            // Timeout
            Err(SendTimeoutError::Timeout(_req)) => {
                return Err(SessionResult::Timeout)
            }

            // Sending request was successful
            Ok(_) => {
                // Await for resource
                match resp.await {
                    Ok(oresource) => {

                        match oresource {
                            Some(r) => Ok(r),
                            None => Err(SessionResult::Full),
                        }
                        
                    }
                    Err(_) => {
                        return Err(SessionResult::NoResponse)
                    }
                }
            }
        }
    }


    /// if resource destroyed call this or drop that
    pub async fn destroyed(&mut self, resource: Resource<T>) {
        resource.recycle().await;
    }

    
    pub async fn print_statistics(&self) {
        let _ = self.sender.send_timeout(Request::PrintStatistics, TIMEOUT).await;
    }
}