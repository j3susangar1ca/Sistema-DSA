mod protocol;
mod storage;
mod sync;
mod etl;
mod scanner;
mod models;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Sistema-DSA Edge Agent iniciado correctamente.");
    Ok(())
}
