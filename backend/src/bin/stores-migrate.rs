use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match stores_api::run_migration_command(std::env::args_os().skip(1)).await {
        Ok(()) => {
            println!("stores database migrations verified");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("stores migration failed: {error}");
            ExitCode::FAILURE
        }
    }
}
