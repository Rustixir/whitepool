
# Whitepool - A hunky Rust+Tokio worker pool factory


*   WhitePool is a **lightweight, generic** pooling library for Rust+Tokio 
    with a focus on **simplicity**, **performance**, and **rock-solid** disaster recovery.

*   Whitepool inspired by **Elixir/Erlang Poolboy**
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
                            || {Box::pin(async move {

                                // tokio_postgres create connection 

                                let (client, connection) =
                                        tokio_postgres::connect("host=localhost user=postgres", NoTls)
                                        .await.unwrap();

                                tokio::spawn(async move {
                                    if let Err(e) = connection.await {
                                        eprintln!("connection error: {}", e);
                                    }
                                });
                                
                                // return client
                                client

                            })}).await.run_service();                            
    

                            
    // session.clone() internally call channel mpsc::Sender::Clone
    process(session.clone()).await;
    
    process(session.clone()).await;
    
    process(session.clone()).await;



    // wait until all pending checkout handled, then shutdown
    session.safe_shutdown().await
    


    tokio::time::sleep(Duration::from_secs(100)).await;
}

pub async fn process(session: Session<Client>) {


    // Checkout a resource from Pool
    // block_checkout don't block your scheduler
    // just await on oneshot with timeout
    if let Ok(mut client) = session.block_checkout().await {
                
    // ============== start ===================

        let _rows = client
                    .get()
                    .query("SELECT $1::TEXT", &[&"hello world"])
                    .await.unwrap();


    // ============== end ===================

        // after job done, call checkin
        let _ = session.checkin(client).await;

        // OR if resource destroy, call this
        // session.destroyed(resource).await;
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
