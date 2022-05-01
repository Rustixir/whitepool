
# Whitepool - A hunky Rust+Tokio worker pool factory


*   WhitePool is a **lightweight, generic** pooling library for Rust+Tokio with a focus 
    on **simplicity**, **performance**, and **rock-solid** disaster recovery.

*   Whitepool is duplicate of **Elixir/Erlang Poolboy**
    **almost all Elixir/Erlang Database library** rely on it for **Connetcion Pooling**




# Example

```rust 

#[tokio::main]
async fn main() {

    let pool_size = 7;
    let max_overflow = 4;


    // Create a session for communicate with pool channel
    let session = Pool::new(pool_size, 
                            max_overflow, 
                            || Box::pin(async move {
                                Connection                                              
                            })).await.run_service();                            
    
    // session.clone() internally call channel mpsc::Sender::Clone
    printer(session.clone()).await;
    
    printer(session.clone()).await;
    
    printer(session).await;
}

pub async fn printer(mut session: Session<Connection>) {


    // Checkout a resource from Pool
    // block_checkout don't block your scheduler
    // just await on oneshot with timeout
    if let Ok(mut resource) = session.block_checkout().await {
        
        // call operaiton here
        resource.get().print();

        // after job done, call checkin
        session.checkin(resource).await;

        // OR if resource destroy, call this
        // session.destroyed(resource).await;
    } 
}



#[derive(Debug)]
pub struct Connection;
impl Connection {
    pub fn print(&self){ 
        println!("==> Print!")
    }
}


```

## Options

- `pool_size`: maximum pool size
- `max_overflow`: maximum number of workers created if pool is empty
- `factory`: is a closure create a worker


# Author

- DanyalMh 



## License

Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file except in compliance with the License. You may obtain a copy of the License at https://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific language governing permissions and limitations under the License.