use std::process;

use download::run;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("运行异常 {}", e);
        process::exit(1)
    }
}
