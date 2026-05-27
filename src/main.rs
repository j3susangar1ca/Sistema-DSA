// src/main.rs
mod protocol;
mod storage;
mod sync;
mod etl;
mod scanner;
mod models;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("====================================================");
    println!("SISTEMA-DSA: AGENTE DE EJECUCIÓN EDGE (WINDOWS 11)");
    println!("Cumplimiento de Calidad ISO/IEC 25010");
    println!("====================================================");

    // Reemplaza esta URL con el ID de despliegue real generado por CLASP tras ejecutar 'clasp deploy'
    let apps_script_web_app_url = "https://script.google.com/macros/s/AKfycbxxxxxxxxx/exec";

    // Disparar el bucle asíncrono persistente de polling
    protocol::start_polling_engine(apps_script_web_app_url).await;

    Ok(())
}