//! Dev CLI example for testing SWAI core functionality.

use std::env;
use std::process;

fn main() {
    let config_path = env::args().nth(1); // nosemgrep: rust.lang.security.args.args

    let (config, reconcile_result, _guard) = match swai_core::run(config_path.as_deref()) {
        Ok(res) => res,
        Err(err) => {
            eprintln!("Error initializing core engine: {}", err);
            process::exit(1);
        }
    };

    println!("Loaded config: {} models", config.models.len());
    println!("Reconcile result: {:?}", reconcile_result);
}
